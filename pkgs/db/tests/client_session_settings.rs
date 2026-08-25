//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! `[db.client]` against a live server.
//!
//! The point of per-pool settings is that they are *in effect on the backend*,
//! not that the code that would apply them compiles. So every assertion here
//! reads the value back from Postgres itself — `current_setting()` on the
//! session, and `pg_stat_activity` for the `application_name` that lets an
//! operator tell one pool's backends from another's.
//!
//! Pools are built here rather than through `HyphenDb::connect` so a single
//! pool's behaviour can be isolated, and so the process-wide layout install is
//! not involved.

mod common;

use std::time::Duration;

use common::TempDb;
use hyphen_db::{HyDbAdminCfg, HyDbClientCfg, HyDbHeavyCfg, HyDbQueryCfg, HyDbWriteCfg};
use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Executor as _, PgPool, Row as _};

type Res = Result<(), Box<dyn std::error::Error>>;

/// A one-connection pool with an explicit `statement_timeout`, for the one
/// assertion that needs to *provoke* the timeout rather than read it back.
/// The pools the daemon actually builds are covered by
/// `each_pool_runs_with_its_own_server_side_settings`, which goes through
/// `HyphenDb::connect`; there is no public API that can hold the query pool
/// busy for ten seconds on demand.
async fn pool_with_timeout(url: &str, timeout_ms: u64) -> Option<PgPool> {
  let opts: PgConnectOptions = url.parse().ok()?;
  PgPoolOptions::new()
    .max_connections(1)
    .acquire_timeout(Duration::from_secs(5))
    .after_connect(move |conn, _meta| {
      Box::pin(async move {
        conn
          .execute(format!("SET statement_timeout = {timeout_ms};").as_str())
          .await?;
        Ok(())
      })
    })
    .connect_with(opts)
    .await
    .ok()
}

async fn setting(pool: &PgPool, name: &str) -> String {
  sqlx::query(&format!("SELECT current_setting('{name}')"))
    .fetch_one(pool)
    .await
    .ok()
    .and_then(|row| row.try_get::<String, _>(0).ok())
    .unwrap_or_default()
}

/// Pick a value guaranteed to differ from what the server already has, so
/// "the setting is in effect" cannot be satisfied by the cluster default. The
/// local server here happens to run `synchronous_commit = off` cluster-wide,
/// which is exactly the trap: a test that asserted the literal value would
/// have passed without any SET being applied at all.
fn contrasting(current: &str, a: &str, b: &str) -> String {
  if current == a {
    b.to_owned()
  } else {
    a.to_owned()
  }
}

#[tokio::test]
async fn each_pool_runs_with_its_own_server_side_settings() -> Res {
  let temp = TempDb::create().await?;
  let result = run_each_pool_runs_with_its_own_server_side_settings(&temp).await;
  temp.drop_now().await;
  result
}

async fn run_each_pool_runs_with_its_own_server_side_settings(temp: &TempDb) -> Res {
  // Read the cluster's own values first. Everything below is asserted
  // relative to these, never against a hard-coded literal: the server used
  // here happens to run `synchronous_commit = off` cluster-wide, which is
  // exactly the trap — a test asserting the literal value would pass with no
  // `SET` applied at all.
  let pool = temp.seed_pool().await?;
  let default_sync = setting(&pool, "synchronous_commit").await;
  let default_work_mem = setting(&pool, "work_mem").await;
  let default_maint = setting(&pool, "maintenance_work_mem").await;
  let default_jit = setting(&pool, "jit").await;
  pool.close().await;

  let client = HyDbClientCfg {
    app_name: format!("hyphend-t4t7-{}", std::process::id()),
    write: HyDbWriteCfg {
      size: 2,
      sync_commit: contrasting(&default_sync, "off", "on"),
      jit: "off".to_owned(),
      statement_timeout: 0,
      ..HyDbWriteCfg::default()
    },
    query: HyDbQueryCfg {
      size: 2,
      statement_timeout: 4_000,
      jit: "off".to_owned(),
      ..HyDbQueryCfg::default()
    },
    heavy: HyDbHeavyCfg {
      size: 1,
      statement_timeout: 45_000,
      work_mem: contrasting(&default_work_mem, "128MB", "96MB"),
      ..HyDbHeavyCfg::default()
    },
    admin: HyDbAdminCfg {
      size: 1,
      maint_work_mem: contrasting(&default_maint, "512MB", "384MB"),
    },
    ..HyDbClientCfg::default()
  };

  // Through the real connect path, so this tests the shipped builder rather
  // than a copy of it.
  let db = temp.open_with(client.clone()).await?;
  let sessions = db.pool_sessions().await.unwrap_or_default();
  assert_eq!(sessions.len(), 4, "{sessions:#?}");
  let get = |name: &str| {
    sessions
      .iter()
      .find(|s| s.name == name)
      .cloned()
      .unwrap_or_default()
  };
  let (write, query, heavy, admin) = (get("write"), get("query"), get("heavy"), get("admin"));

  // T3 — every pool is separately attributable in pg_stat_activity.
  assert_eq!(write.app_name, format!("{}/write", client.app_name));
  assert_eq!(query.app_name, format!("{}/query", client.app_name));
  assert_eq!(heavy.app_name, format!("{}/heavy", client.app_name));
  assert_eq!(admin.app_name, format!("{}/admin", client.app_name));

  // T4 — the write pool carries its own commit mode, and it is not the
  // server's. `statement_timeout = 0` is unbounded: a block connect is one
  // atomic transaction and must not be killed mid-flight.
  assert_eq!(write.synchronous_commit, client.write.sync_commit);
  assert_ne!(write.synchronous_commit, default_sync);
  assert_eq!(write.jit, "off");
  assert_eq!(write.statement_timeout, "0");

  // ...and the scoping is the whole argument for per-pool SETs over
  // cluster-wide settings, so it is asserted rather than assumed: no other
  // pool inherited the write pool's commit mode or its unbounded timeout.
  for other in [&query, &heavy, &admin] {
    assert_eq!(
      other.synchronous_commit, default_sync,
      "{} inherited the write pool's commit mode",
      other.name
    );
  }
  assert_ne!(query.statement_timeout, "0");
  assert_ne!(heavy.statement_timeout, "0");

  // T5 — the query pool's timeout is the configured one, in milliseconds.
  assert_eq!(
    query.statement_timeout,
    client.query.statement_timeout.to_string()
  );
  assert_eq!(query.jit, "off");

  // T6 — heavy reads get the configured work_mem, and nobody else does.
  assert_eq!(heavy.work_mem, client.heavy.work_mem);
  assert_eq!(
    heavy.statement_timeout,
    client.heavy.statement_timeout.to_string()
  );
  assert_eq!(query.work_mem, default_work_mem);
  assert_eq!(write.work_mem, default_work_mem);

  // T7 — admin gets maintenance_work_mem, the index-build input, and nothing
  // else does.
  assert_eq!(admin.maintenance_work_mem, client.admin.maint_work_mem);
  assert_eq!(
    query.maintenance_work_mem, default_maint,
    "maintenance_work_mem must not leak off the admin pool"
  );
  // The admin pool sets nothing else, so it still has the server's jit.
  assert_eq!(admin.jit, default_jit);

  drop(db);
  Ok(())
}

#[tokio::test]
async fn a_query_pool_timeout_cancels_a_slow_read_instead_of_holding_the_pool() -> Res {
  let temp = TempDb::create().await?;
  let result =
    run_a_query_pool_timeout_cancels_a_slow_read_instead_of_holding_the_pool(&temp).await;
  temp.drop_now().await;
  result
}

async fn run_a_query_pool_timeout_cancels_a_slow_read_instead_of_holding_the_pool(
  temp: &TempDb,
) -> Res {
  let base = temp.client_cfg()?;
  let query = pool_with_timeout(&base.url, 250)
    .await
    .ok_or("a one-connection pool with a statement_timeout must build")?;

  let started = std::time::Instant::now();
  let result = sqlx::query("SELECT pg_sleep(10)").fetch_one(&query).await;
  let elapsed = started.elapsed();

  let code = result
    .as_ref()
    .err()
    .and_then(|e| e.as_database_error())
    .and_then(|e| e.code())
    .map(|c| c.to_string())
    .unwrap_or_default();
  assert_eq!(
    code, "57014",
    "a read past the timeout must be cancelled by the server (query_canceled), got {result:?}"
  );
  assert!(
    elapsed < Duration::from_secs(5),
    "the cancellation must happen at the timeout, not at the end of the query: {elapsed:?}"
  );

  // The connection survives, so a cancelled statement costs the pool nothing
  // — which is the difference between a capped read and one that holds a
  // connection until the pool is exhausted.
  let alive = sqlx::query("SELECT 1").fetch_one(&query).await;
  assert!(alive.is_ok(), "{alive:?}");

  query.close().await;
  Ok(())
}

#[tokio::test]
async fn app_name_attributes_backends_to_a_pool_in_pg_stat_activity() -> Res {
  let temp = TempDb::create().await?;
  let result = run_app_name_attributes_backends_to_a_pool_in_pg_stat_activity(&temp).await;
  temp.drop_now().await;
  result
}

async fn run_app_name_attributes_backends_to_a_pool_in_pg_stat_activity(temp: &TempDb) -> Res {
  let app_name = format!("hyphend-attrib-{}", std::process::id());
  let db = temp
    .open_with(HyDbClientCfg {
      app_name: app_name.clone(),
      write: HyDbWriteCfg {
        size: 1,
        ..HyDbWriteCfg::default()
      },
      query: HyDbQueryCfg {
        size: 1,
        ..HyDbQueryCfg::default()
      },
      heavy: HyDbHeavyCfg {
        size: 1,
        ..HyDbHeavyCfg::default()
      },
      admin: HyDbAdminCfg {
        size: 1,
        ..HyDbAdminCfg::default()
      },
      ..HyDbClientCfg::default()
    })
    .await?;
  // Touch every pool so each has a live backend on the server.
  let _ = db.pool_sessions().await;

  // Read from an unrelated connection: this is the operator's view, the one
  // that was previously coarse enough to say "86–100% idle" without being
  // able to say *which* pool was idle.
  let observer = temp.seed_pool().await?;
  let names: Vec<String> = sqlx::query(
    "SELECT application_name FROM pg_stat_activity \
       WHERE application_name LIKE $1 ORDER BY application_name",
  )
  .bind(format!("{app_name}/%"))
  .fetch_all(&observer)
  .await
  .map(|rows| {
    rows
      .iter()
      .filter_map(|r| r.try_get::<String, _>(0).ok())
      .collect()
  })
  .unwrap_or_default();

  for pool in ["write", "query", "heavy", "admin"] {
    assert!(
      names.iter().any(|n| n == &format!("{app_name}/{pool}")),
      "the {pool} pool's backends are not attributable: {names:?}"
    );
  }

  observer.close().await;
  drop(db);
  Ok(())
}

#[tokio::test]
async fn a_configured_unix_socket_carries_a_real_connection() -> Res {
  let temp = TempDb::create().await?;
  let result = run_a_configured_unix_socket_carries_a_real_connection(&temp).await;
  temp.drop_now().await;
  result
}

async fn run_a_configured_unix_socket_carries_a_real_connection(temp: &TempDb) -> Res {
  let observer = temp.seed_pool().await?;

  // The server's own socket directory, asked for rather than assumed — /tmp
  // on Homebrew, /var/run/postgresql on Debian.
  let dirs = setting(&observer, "unix_socket_directories").await;
  let dir = dirs
    .split(',')
    .map(str::trim)
    .find(|d| !d.is_empty())
    .map(str::to_owned)
    .ok_or("the server must report at least one unix_socket_directory")?;

  let app_name = format!("hyphend-sock-{}", std::process::id());
  let client = HyDbClientCfg {
    socket: Some(dir.clone()),
    app_name: app_name.clone(),
    write: HyDbWriteCfg {
      size: 1,
      ..HyDbWriteCfg::default()
    },
    query: HyDbQueryCfg {
      size: 1,
      ..HyDbQueryCfg::default()
    },
    heavy: HyDbHeavyCfg {
      size: 1,
      ..HyDbHeavyCfg::default()
    },
    admin: HyDbAdminCfg {
      size: 1,
      ..HyDbAdminCfg::default()
    },
    ..HyDbClientCfg::default()
  };
  assert_eq!(client.endpoint(), format!("unix:{dir}"));

  // A full connect: migrations, the layout contract and the four pools all
  // go over the socket, not just a bare handshake.
  let db = temp.open_with(client).await?;
  let sessions = db.pool_sessions().await.unwrap_or_default();
  assert_eq!(sessions.len(), 4, "{sessions:#?}");

  // Postgres reports no client address for a Unix-socket connection. That is
  // the assertion that the socket is what carried these sessions, rather than
  // the URL's TCP host quietly winning.
  let tcp_backends: i64 = sqlx::query(
    "SELECT count(*) FROM pg_stat_activity \
       WHERE application_name LIKE $1 AND client_addr IS NOT NULL",
  )
  .bind(format!("{app_name}/%"))
  .fetch_one(&observer)
  .await
  .ok()
  .and_then(|row| row.try_get::<i64, _>(0).ok())
  .unwrap_or(-1);
  assert_eq!(
    tcp_backends, 0,
    "a configured socket must win over the URL's TCP host"
  );
  let socket_backends: i64 = sqlx::query(
    "SELECT count(*) FROM pg_stat_activity \
       WHERE application_name LIKE $1 AND client_addr IS NULL",
  )
  .bind(format!("{app_name}/%"))
  .fetch_one(&observer)
  .await
  .ok()
  .and_then(|row| row.try_get::<i64, _>(0).ok())
  .unwrap_or_default();
  assert!(
    socket_backends >= 4,
    "got {socket_backends} socket backends"
  );

  observer.close().await;
  drop(db);
  Ok(())
}
