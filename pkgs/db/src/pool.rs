//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! PostgreSQL pool creation, split by workload. Private to the crate: the four
//! pools never leave `hyphen-db`.
//!
//! Each pool differs in three ways, and all three are configuration rather
//! than constants: its size, how long a caller waits for a free connection,
//! and the per-session settings applied to every connection it opens. The
//! third is the interesting one — see [`PoolSpec::session_sql`].

use core::fmt;
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::{Executor as _, PgPool};

use crate::config::HyDbClientCfg;
use crate::error::HyDbError;

/// The four workload pool names, as they appear in `application_name` and in
/// diagnostics. Suffixed onto `[db.client] app_name`, which is the whole
/// point: `pg_stat_activity` can then attribute a backend to the workload that
/// owns it, so "86–100% idle" becomes a statement about a *specific* pool
/// instead of about the daemon as a whole.
pub(crate) const POOL_WRITE: &str = "write";
pub(crate) const POOL_QUERY: &str = "query";
pub(crate) const POOL_HEAVY: &str = "heavy";
pub(crate) const POOL_ADMIN: &str = "admin";

/// The daemon's Postgres pools, split by workload so a slow analytical query
/// can never starve the ingest writer.
#[derive(Clone)]
pub(crate) struct HyDbPools {
  /// Single-writer ingest path.
  pub write: PgPool,
  /// Public, latency-sensitive reads.
  pub query: PgPool,
  /// Heavy analytical reads (rich list, ranges).
  pub heavy_query: PgPool,
  /// Maintenance and diagnostics.
  pub admin: PgPool,
}

/// Everything that distinguishes one pool from another.
struct PoolSpec {
  name: &'static str,
  size: usize,
  acquire_timeout: u64,
  /// `SET` statements applied to every connection this pool opens.
  session: Vec<String>,
}

impl PoolSpec {
  /// The per-session settings, as one statement batch.
  ///
  /// These run once per *connection*, not once per acquisition, because a
  /// `SET` is session-scoped: re-issuing it on every checkout would be the
  /// same round trip repeated for no change in effect, on a pipeline that is
  /// already round-trip bound. The settings persist for the connection's whole
  /// life, which is exactly the "applied on acquisition" contract, minus the
  /// per-acquisition cost.
  ///
  /// Values reaching this string are validated by
  /// [`HyDbConfig::validate`](crate::HyDbConfig::validate) first — `SET` takes
  /// no bind parameters, so the check has to happen before the statement is
  /// built.
  fn session_sql(&self) -> Option<String> {
    if self.session.is_empty() {
      return None;
    }
    Some(self.session.join(" "))
  }
}

fn specs(cfg: &HyDbClientCfg) -> [PoolSpec; 4] {
  [
    PoolSpec {
      name: POOL_WRITE,
      size: cfg.write.size,
      acquire_timeout: cfg.write.acquire_timeout,
      session: vec![
        format!("SET synchronous_commit = '{}';", cfg.write.sync_commit),
        format!("SET jit = '{}';", cfg.write.jit),
        format!("SET statement_timeout = {};", cfg.write.statement_timeout),
      ],
    },
    PoolSpec {
      name: POOL_QUERY,
      size: cfg.query.size,
      acquire_timeout: cfg.query.acquire_timeout,
      session: vec![
        format!("SET jit = '{}';", cfg.query.jit),
        format!("SET statement_timeout = {};", cfg.query.statement_timeout),
      ],
    },
    PoolSpec {
      name: POOL_HEAVY,
      size: cfg.heavy.size,
      acquire_timeout: cfg.heavy.acquire_timeout,
      session: vec![
        format!("SET work_mem = '{}';", cfg.heavy.work_mem),
        format!("SET statement_timeout = {};", cfg.heavy.statement_timeout),
      ],
    },
    PoolSpec {
      name: POOL_ADMIN,
      size: cfg.admin.size,
      acquire_timeout: cfg.admin.acquire_timeout(),
      session: vec![format!(
        "SET maintenance_work_mem = '{}';",
        cfg.admin.maint_work_mem
      )],
    },
  ]
}

impl HyDbPools {
  pub(crate) async fn connect(cfg: &HyDbClientCfg) -> Result<Self, HyDbError> {
    cfg.validate()?;
    let [write_spec, query_spec, heavy_spec, admin_spec] = specs(cfg);
    Ok(Self {
      write: build_pool(cfg, &write_spec).await?,
      query: build_pool(cfg, &query_spec).await?,
      heavy_query: build_pool(cfg, &heavy_spec).await?,
      admin: build_pool(cfg, &admin_spec).await?,
    })
  }
}

impl fmt::Debug for HyDbPools {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("HyDbPools").finish()
  }
}

/// A one-connection pool for read-only diagnostics (`--show-layout`), which
/// must work against a database this process is not otherwise opening — no
/// migrations, no layout resolution, no ingest lock.
///
/// # Errors
///
/// [`HyDbError::Config`] if the URL is unparseable, [`HyDbError::Pg`] if the
/// connection cannot be established.
pub(crate) async fn diagnostic_pool(cfg: &HyDbClientCfg) -> Result<PgPool, HyDbError> {
  build_pool(
    cfg,
    &PoolSpec {
      name: "diag",
      size: 1,
      acquire_timeout: cfg.connect_timeout.max(1),
      session: Vec::new(),
    },
  )
  .await
}

async fn build_pool(cfg: &HyDbClientCfg, spec: &PoolSpec) -> Result<PgPool, HyDbError> {
  use sqlx::ConnectOptions as _;

  let app_name = format!("{}/{}", cfg.app_name, spec.name);
  tracing::info!(
    endpoint = %cfg.endpoint(),
    pool = spec.name,
    pool_size = spec.size,
    app_name = %app_name,
    "creating postgres pool"
  );

  let slow_threshold = Duration::from_millis(cfg.slow_query_ms);
  let opts = connect_options(cfg)?
    .application_name(&app_name)
    .statement_cache_capacity(cfg.stmt_cache_size)
    .log_statements(tracing::log::LevelFilter::Debug)
    .log_slow_statements(tracing::log::LevelFilter::Warn, slow_threshold);

  let session_sql = spec.session_sql();
  let mut builder = PgPoolOptions::new()
    .max_connections(spec.size as u32)
    .acquire_timeout(Duration::from_secs(spec.acquire_timeout));
  if cfg.idle_timeout > 0 {
    builder = builder.idle_timeout(Duration::from_secs(cfg.idle_timeout));
  }
  if cfg.max_lifetime > 0 {
    builder = builder.max_lifetime(Duration::from_secs(cfg.max_lifetime));
  }
  if let Some(sql) = session_sql.clone() {
    builder = builder.after_connect(move |conn, _meta| {
      let sql = sql.clone();
      Box::pin(async move {
        conn.execute(sql.as_str()).await?;
        Ok(())
      })
    });
  }

  // sqlx has no per-connection connect timeout; `connect_with` establishes the
  // pool's first connection, so bounding that call is what `connect_timeout`
  // means here. Without it an unreachable host hangs on the TCP stack's own
  // timeout, which on some paths is minutes.
  let connect = builder.connect_with(opts);
  let pool = tokio::time::timeout(Duration::from_secs(cfg.connect_timeout.max(1)), connect)
    .await
    .map_err(|_| {
      HyDbError::Pg {
        message: format!(
          "failed to connect {} pool: no connection within db.client.connect_timeout ({}s)",
          spec.name, cfg.connect_timeout
        ),
        // 08006 connection_failure: a timeout reaching the server is transient,
        // and the supervisor must be allowed to retry it.
        code: Some("08006".to_owned()),
      }
    })?
    .map_err(|e| HyDbError::pg(&format!("failed to connect {} pool", spec.name), e))?;

  tracing::info!(
    pool = spec.name,
    session = session_sql.as_deref().unwrap_or("(none)"),
    "postgres pool created"
  );
  Ok(pool)
}

/// Resolve `[db.client] socket` and `url` into connect options.
///
/// The rule is libpq's, and the reason is latency: a configured socket
/// directory replaces the URL's *host*, and nothing else. Credentials,
/// database name and port keep coming from the URL, so switching a deployment
/// between TCP and a socket is one added line rather than a second copy of the
/// connection details that can drift.
fn connect_options(cfg: &HyDbClientCfg) -> Result<PgConnectOptions, HyDbError> {
  let base = parse_connect_options(&cfg.url)?;
  match cfg
    .socket
    .as_deref()
    .map(str::trim)
    .filter(|s| !s.is_empty())
  {
    Some(dir) => Ok(base.socket(dir)),
    None => Ok(base),
  }
}

/// Parse a connection URL into `PgConnectOptions`, handling the libpq
/// `?host=/path` Unix-socket form that sqlx does not accept natively.
fn parse_connect_options(url: &str) -> Result<PgConnectOptions, HyDbError> {
  if let Ok(opts) = url.parse::<PgConnectOptions>() {
    return Ok(opts);
  }

  let query_str = url.split_once('?').map(|(_, q)| q).unwrap_or("");
  let socket_dir = query_str
    .split('&')
    .filter_map(|pair| {
      let (k, v) = pair.split_once('=')?;
      (k == "host").then_some(v)
    })
    .next()
    .ok_or_else(|| {
      HyDbError::Config(format!(
        "db.client.url has empty host and no ?host= query parameter, and \
         db.client.socket is not set: {url}"
      ))
    })?;

  let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
  let authority_path = after_scheme
    .split_once('?')
    .map(|(a, _)| a)
    .unwrap_or(after_scheme);
  let (userinfo, host_path) = authority_path
    .split_once('@')
    .unwrap_or(("", authority_path));
  let (username, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
  let db = host_path.trim_start_matches('/');

  let mut opts = PgConnectOptions::new().socket(socket_dir);
  if !db.is_empty() {
    opts = opts.database(db);
  }
  if !username.is_empty() {
    opts = opts.username(username);
  }
  if !password.is_empty() {
    opts = opts.password(password);
  }
  for pair in query_str.split('&') {
    if let Some(("port", v)) = pair.split_once('=') {
      if let Ok(p) = v.parse::<u16>() {
        opts = opts.port(p);
      }
    }
  }
  Ok(opts)
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::config::{HyDbAdminCfg, HyDbHeavyCfg, HyDbWriteCfg};

  #[test]
  fn parses_tcp_url() {
    assert!(parse_connect_options("postgresql://u:p@127.0.0.1:5432/db").is_ok());
  }

  #[test]
  fn parses_unix_socket_url() {
    let opts = parse_connect_options("postgresql://u:p@/db?host=/var/run/postgresql&port=5433");
    assert!(opts.is_ok());
  }

  #[test]
  fn empty_host_without_socket_errors() {
    assert!(parse_connect_options("postgresql://u:p@/db").is_err());
  }

  #[test]
  fn a_configured_socket_wins_over_the_urls_host() {
    let cfg = HyDbClientCfg {
      socket: Some("/tmp".to_owned()),
      url: "postgresql://u:p@127.0.0.1:5432/db".to_owned(),
      ..HyDbClientCfg::default()
    };
    let opts = connect_options(&cfg);
    assert!(opts.is_ok());
    assert_eq!(cfg.endpoint(), "unix:/tmp");
    // ...and the credentials and database still come from the URL.
    if let Ok(opts) = opts {
      assert_eq!(opts.get_database(), Some("db"));
    }
  }

  #[test]
  fn an_unset_or_blank_socket_falls_back_to_the_url() {
    for socket in [None, Some(String::new()), Some("   ".to_owned())] {
      let cfg = HyDbClientCfg {
        socket,
        url: "postgresql://u:p@127.0.0.1:5432/db".to_owned(),
        ..HyDbClientCfg::default()
      };
      assert!(connect_options(&cfg).is_ok());
      assert!(cfg.endpoint().starts_with("postgresql://"));
    }
  }

  #[test]
  fn each_pool_gets_its_own_settings_and_no_others() {
    let cfg = HyDbClientCfg {
      write: HyDbWriteCfg {
        sync_commit: "off".to_owned(),
        statement_timeout: 0,
        ..HyDbWriteCfg::default()
      },
      heavy: HyDbHeavyCfg {
        work_mem: "128MB".to_owned(),
        ..HyDbHeavyCfg::default()
      },
      admin: HyDbAdminCfg {
        maint_work_mem: "2GB".to_owned(),
        ..HyDbAdminCfg::default()
      },
      ..HyDbClientCfg::default()
    };
    let specs = specs(&cfg);
    let sql = |name: &str| {
      specs
        .iter()
        .find(|s| s.name == name)
        .and_then(PoolSpec::session_sql)
        .unwrap_or_default()
    };

    let write = sql(POOL_WRITE);
    assert!(write.contains("SET synchronous_commit = 'off';"), "{write}");
    assert!(write.contains("SET statement_timeout = 0;"), "{write}");

    // Scoping is the entire point: nothing but the write pool relaxes commit
    // durability, and nothing but the write pool is unbounded.
    for other in [POOL_QUERY, POOL_HEAVY, POOL_ADMIN] {
      let s = sql(other);
      assert!(!s.contains("synchronous_commit"), "{other}: {s}");
      assert!(!s.contains("statement_timeout = 0;"), "{other}: {s}");
    }

    assert!(sql(POOL_HEAVY).contains("SET work_mem = '128MB';"));
    assert!(sql(POOL_ADMIN).contains("SET maintenance_work_mem = '2GB';"));
    assert!(!sql(POOL_QUERY).contains("work_mem"));
  }

  #[test]
  fn app_name_is_suffixed_per_pool() {
    let cfg = HyDbClientCfg {
      app_name: "hyphend".to_owned(),
      ..HyDbClientCfg::default()
    };
    let names: Vec<String> = specs(&cfg)
      .iter()
      .map(|s| format!("{}/{}", cfg.app_name, s.name))
      .collect();
    assert_eq!(
      names,
      vec![
        "hyphend/write",
        "hyphend/query",
        "hyphend/heavy",
        "hyphend/admin"
      ]
    );
  }
}
