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

pub use config::{HyDbConfig, DEFAULT_POSTGRES_URL};
pub use error::{HyDbError, HyDbResult, Outcome};
pub use ingest::{HyDbBlockSeen, HyDbIngest, HyDbProgress, HyDbReorg, HyDbStage, HyDbTipReport};
pub use records::{
  HyDbBlock, HyDbBlockId, HyDbCbFields, HyDbDataSrc, HyDbHeader, HyDbPayload, HyDbTx, HyDbTxIn,
  HyDbTxOut, HyDbVlEvent, HyDbVlRegister, HyDbVlRevoke, HyDbVlUpdRegistrar, HyDbVlUpdService,
};
// The query surface is broad; rather than mirror every row type at the crate
// root, consumers reach them through `hyphen_db::query::*` (re-exported by
// `query/mod.rs`). Only the domain traits are pulled in here, for the facade.
pub use query::HyDbQrPoolStat;
use query::{
  HyDbAddrIdx, HyDbBlockIdx, HyDbMetricIdx, HyDbSearch, HyDbStatus, HyDbTxIdx, HyDbVlIdx,
};
pub use types::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbCursor, HyDbHash, HyDbHeight, HyDbOutpoint, HyDbPage,
  HyDbTxType, HyDbVlType,
};

use query::PgRead;

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
  /// pool cannot connect, or [`HyDbError::Migrate`] if the schema fails to apply.
  pub async fn connect(cfg: HyDbConfig) -> HyDbResult<Self> {
    let pools = pool::HyDbPools::connect(&cfg).await?;
    migrate::apply(&pools.write).await?;

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
