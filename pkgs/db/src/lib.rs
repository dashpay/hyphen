//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! `hyphen-db` — the storage crate for the Hyphen block explorer.
//!
//! The engine (PostgreSQL) is a private concern. Callers deal only in Rust
//! types: no SQL, no pools, no knowledge of how consistency, undo, or reorgs
//! are implemented. The database holds **chain-derivable data only** — base
//! facts, precomputed accumulations, and query indexes — all rebuildable from
//! the raw block store plus a caller-supplied [`HyDbDataSrc`]. Application
//! state and live node state (mempool, governance, locks) stay in the daemon.
//!
//! # Shape
//!
//! - [`HyphenDb`] is the cheap-to-clone facade. Its accessors ([`HyphenDb::blocks`],
//!   [`HyphenDb::txs`], ...) return read-only domain handles (trait objects).
//! - [`HyphenDb::ingest`] returns the single-writer [`HyDbIngest`], an
//!   event-shaped sink (`on_block`/`on_tip`) the daemon wires into node
//!   announcements.
//! - The naming convention: plural types are *handles* ([`HyDbBlockIdx`]);
//!   singular types are *facts/records* ([`crate::HyDbBlock`],
//!   [`query::HyDbQrBlockDetail`]).

mod config;
mod error;
mod ingest;
pub mod layout;
mod migrate;
mod pool;
pub mod query;
mod records;
pub mod types;
mod util;

#[cfg(any(test, feature = "tests"))]
pub mod tests;

use core::fmt;
use std::sync::Arc;

pub use config::{
  HyDbAdminCfg, HyDbClientCfg, HyDbConfig, HyDbHeavyCfg, HyDbQueryCfg, HyDbWriteCfg,
  DEFAULT_POSTGRES_URL,
};
pub use error::{HyDbError, HyDbResult, Outcome};
pub use ingest::{HyDbBlockSeen, HyDbIngest, HyDbProgress, HyDbReorg, HyDbStage, HyDbTipReport};
pub use records::{
  HyDbBlock, HyDbBlockId, HyDbCbFields, HyDbDataSrc, HyDbHeader, HyDbPayload, HyDbTx, HyDbTxIn,
  HyDbTxOut, HyDbVlEvent, HyDbVlRegister, HyDbVlRevoke, HyDbVlUpdRegistrar, HyDbVlUpdService,
};
// The query surface is broad; rather than mirror every row type at the crate
// root, consumers reach them through `hyphen_db::query::*` (re-exported by
// `query/mod.rs`). Only the domain traits are pulled in here, for the facade.
use query::{
  HyDbAddrIdx, HyDbBlockIdx, HyDbMetricIdx, HyDbSearch, HyDbStatus, HyDbTxIdx, HyDbVlIdx,
};
pub use query::{HyDbQrPoolSession, HyDbQrPoolStat};
pub use types::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbCursor, HyDbHash, HyDbHeight, HyDbOutpoint, HyDbPage,
  HyDbTxType, HyDbVlType,
};

use query::PgRead;

/// Read the storage layout recorded in a database and render it against the
/// configured one, for `hyphend --show-layout`.
///
/// Deliberately does everything the startup path does *except* act: no
/// migration, no write to `init_params`, no advisory lock. An operator staring
/// at a layout refusal can run this against the same database while the
/// daemon is down and see exactly which key disagrees and what its remedy is.
///
/// # Errors
///
/// [`HyDbError::Config`] if the URL is unparseable, [`HyDbError::Pg`] if the
/// database cannot be read.
pub async fn show_layout(cfg: &HyDbConfig) -> HyDbResult<String> {
  // Same validation the serve path runs: a diagnostic that happily explains
  // the layout while `[db.client]` holds a value that will refuse to start is
  // answering the wrong question.
  cfg.validate()?;
  let pool = pool::diagnostic_pool(&cfg.client).await?;
  let mut layout_cfg = cfg.layout.clone();
  layout_cfg
    .schema_version
    .get_or_insert(migrate::latest_version());
  let rendered = layout::show(&pool, &layout_cfg).await;
  pool.close().await;
  rendered
}

/// Class/object key for the single-writer ingest advisory lock. Arbitrary but
/// stable: ASCII "HyDbIngst" packed into an i64.
const INGEST_ADVISORY_LOCK_KEY: i64 = 0x4879_4462_496e_6773;

/// The storage facade. Cheap to clone (a bundle of `Arc`d domain handles); pass
/// it wherever the daemon previously threaded a pool.
#[derive(Clone)]
pub struct HyphenDb {
  blocks: Arc<dyn HyDbBlockIdx>,
  txs: Arc<dyn HyDbTxIdx>,
  addrs: Arc<dyn HyDbAddrIdx>,
  validators: Arc<dyn HyDbVlIdx>,
  metrics: Arc<dyn HyDbMetricIdx>,
  search: Arc<dyn HyDbSearch>,
  status: Arc<dyn HyDbStatus>,
  /// Present only for the Postgres backend; a mock facade leaves it `None` and
  /// cannot start an ingest.
  write: Option<sqlx::PgPool>,
  /// All four pools, retained for diagnostics only. `None` on a mock facade.
  pools: Option<pool::HyDbPools>,
}

impl HyphenDb {
  pub fn addrs(&self) -> &dyn HyDbAddrIdx {
    self.addrs.as_ref()
  }

  pub fn blocks(&self) -> &dyn HyDbBlockIdx {
    self.blocks.as_ref()
  }

  /// Connect to Postgres, apply pending migrations, and build the facade.
  ///
  /// # Errors
  ///
  /// [`HyDbError::Config`] if the URL is unparseable, [`HyDbError::Pg`] if a
  /// pool cannot connect, [`HyDbError::Migrate`] if the schema fails to apply,
  /// or [`HyDbError::Layout`] if `[db.layout]` disagrees with the layout this
  /// database was created with.
  pub async fn connect(cfg: HyDbConfig) -> HyDbResult<Self> {
    let pools = pool::HyDbPools::connect(&cfg.client).await?;
    migrate::apply(&pools.write).await?;

    // Immediately after the migrator, and before anything can read or write a
    // block: `init_params` is guaranteed to exist by migration 0002, and no
    // other code path may run against a database whose layout has not been
    // agreed.
    let mut layout_cfg = cfg.layout.clone();
    layout_cfg
      .schema_version
      .get_or_insert(migrate::latest_version());
    let resolved = layout::verify(&pools.write, &layout_cfg, &cfg.created_by).await?;
    layout::install(resolved)?;

    let read = PgRead {
      query: pools.query.clone(),
      heavy: pools.heavy_query.clone(),
      admin: pools.admin.clone(),
    };

    Ok(Self {
      blocks: Arc::new(query::blocks::PgBlockIdx { pg: read.clone() }),
      txs: Arc::new(query::txs::PgTxIdx { pg: read.clone() }),
      addrs: Arc::new(query::addrs::PgAddrIdx { pg: read.clone() }),
      validators: Arc::new(query::validators::PgVlIdx { pg: read.clone() }),
      metrics: Arc::new(query::metrics::PgMetricIdx { pg: read.clone() }),
      search: Arc::new(query::search::PgSearch { pg: read.clone() }),
      status: Arc::new(query::status::PgStatus { pg: read }),
      write: Some(pools.write.clone()),
      pools: Some(pools),
    })
  }

  /// Assemble a facade from caller-supplied domain handles. The daemon uses
  /// this to inject in-memory fakes for its own tests; the resulting facade has
  /// no ingest capability.
  #[cfg(any(test, feature = "tests"))]
  #[allow(clippy::too_many_arguments)]
  pub fn from_parts(
    blocks: Arc<dyn HyDbBlockIdx>,
    txs: Arc<dyn HyDbTxIdx>,
    addrs: Arc<dyn HyDbAddrIdx>,
    validators: Arc<dyn HyDbVlIdx>,
    metrics: Arc<dyn HyDbMetricIdx>,
    search: Arc<dyn HyDbSearch>,
    status: Arc<dyn HyDbStatus>,
  ) -> Self {
    Self {
      blocks,
      txs,
      addrs,
      validators,
      metrics,
      search,
      status,
      write: None,
      pools: None,
    }
  }

  /// Acquire the single-writer ingest handle, decoding blocks through the
  /// supplied data source. Guarded by a session-scoped Postgres advisory lock
  /// held for the lifetime of the returned handle, so exactly one ingest
  /// exists across all processes — two writers interleaving undo pre-images
  /// would permanently corrupt balances.
  ///
  /// # Errors
  ///
  /// [`HyDbError::Config`] on a mock facade with no write pool, or when
  /// another process already holds the ingest lock. [`HyDbError::Pg`] if the
  /// lock connection cannot be established.
  pub async fn ingest(&self, data_src: Arc<dyn HyDbDataSrc>) -> HyDbResult<HyDbIngest> {
    let write = self
      .write
      .as_ref()
      .ok_or_else(|| HyDbError::Config("ingest requires a Postgres-backed HyphenDb".to_owned()))?;

    // A dedicated, detached connection owns the session lock. It is closed
    // (releasing the lock) when the ingest handle drops; detaching keeps the
    // pool from ever handing this session — and its lock — to someone else.
    let mut lock_conn = write
      .acquire()
      .await
      .map_err(|e| HyDbError::pg("ingest: acquire lock connection", e))?
      .detach();
    let acquired: bool = sqlx::query_scalar("SELECT pg_try_advisory_lock($1)")
      .bind(INGEST_ADVISORY_LOCK_KEY)
      .fetch_one(&mut lock_conn)
      .await
      .map_err(|e| HyDbError::pg("ingest: take advisory lock", e))?;
    if !acquired {
      return Err(HyDbError::Config(
        "another process already holds the ingest advisory lock on this database".to_owned(),
      ));
    }
    Ok(HyDbIngest::new(data_src, write.clone(), lock_conn))
  }

  /// Per-pool utilization for diagnostics (empty on a mock facade).
  pub fn pool_stats(&self) -> Vec<HyDbQrPoolStat> {
    let Some(pools) = self.pools.as_ref() else {
      return Vec::new();
    };
    let stat = |name: &str, pool: &sqlx::PgPool| HyDbQrPoolStat {
      name: name.to_owned(),
      size: pool.size(),
      idle: pool.num_idle(),
    };
    vec![
      stat("write", &pools.write),
      stat("query", &pools.query),
      stat("heavy_query", &pools.heavy_query),
      stat("admin", &pools.admin),
    ]
  }

  /// Per-pool session settings as the server reports them, for diagnostics
  /// and for the tests that prove `[db.client.*]` actually reaches a backend.
  ///
  /// One round trip per pool, on the pool itself — the values are
  /// session-scoped, so they can only be read from a connection that pool
  /// owns. Empty on a mock facade.
  ///
  /// # Errors
  ///
  /// [`HyDbError::Pg`] if a pool cannot answer.
  pub async fn pool_sessions(&self) -> HyDbResult<Vec<HyDbQrPoolSession>> {
    let Some(pools) = self.pools.as_ref() else {
      return Ok(Vec::new());
    };
    let mut out = Vec::with_capacity(4);
    for (name, pool) in [
      (pool::POOL_WRITE, &pools.write),
      (pool::POOL_QUERY, &pools.query),
      (pool::POOL_HEAVY, &pools.heavy_query),
      (pool::POOL_ADMIN, &pools.admin),
    ] {
      // `pg_settings.setting` is the value in the GUC's own base unit, with no
      // suffix: `current_setting('statement_timeout')` would render 5000ms as
      // "5s", which is unhelpful to compare against a configured number.
      let row: (String, String, String, String, String, String) = sqlx::query_as(
        r#"SELECT current_setting('application_name'),
                  current_setting('synchronous_commit'),
                  (SELECT setting FROM pg_settings WHERE name = 'statement_timeout'),
                  current_setting('work_mem'),
                  current_setting('maintenance_work_mem'),
                  current_setting('jit')"#,
      )
      .fetch_one(pool)
      .await
      .map_err(|e| HyDbError::pg(&format!("read {name} pool session settings"), e))?;
      out.push(HyDbQrPoolSession {
        name: name.to_owned(),
        app_name: row.0,
        synchronous_commit: row.1,
        statement_timeout: row.2,
        work_mem: row.3,
        maintenance_work_mem: row.4,
        jit: row.5,
      });
    }
    Ok(out)
  }

  pub fn metrics(&self) -> &dyn HyDbMetricIdx {
    self.metrics.as_ref()
  }

  pub fn search(&self) -> &dyn HyDbSearch {
    self.search.as_ref()
  }

  pub fn status(&self) -> &dyn HyDbStatus {
    self.status.as_ref()
  }

  pub fn txs(&self) -> &dyn HyDbTxIdx {
    self.txs.as_ref()
  }

  pub fn validators(&self) -> &dyn HyDbVlIdx {
    self.validators.as_ref()
  }
}

impl fmt::Debug for HyphenDb {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("HyphenDb").finish()
  }
}
