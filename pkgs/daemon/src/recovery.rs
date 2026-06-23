//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Startup recovery helpers for RocksDB and sync-state reconciliation.
//!
//! These checks are intentionally conservative: they allow normal ingestion
//! lag, but reject startup states that imply PostgreSQL references blocks
//! RocksDB does not have.

use std::path::Path;
use std::sync::Arc;

use crate::sql::SqlPool;
use crate::store::{self, RecoveryState, StoreId};
use crate::store::manager::SecondaryStoreSet;

use crate::db;
use crate::error::DaemonError;

/// Verify that RocksDB contains every block height referenced by PostgreSQL.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when the block stage height cannot be read, or
/// `DaemonError::Db` when PostgreSQL is ahead of RocksDB.
pub async fn verify_rocks_covers_pg(
  pool: &SqlPool,
  db: &Arc<rocksdb::DB>,
) -> Result<(), DaemonError> {
  let Some(rocks_tip) = db::get_tip_height(db) else {
    return Ok(());
  };
  let pg_block_height =
    crate::sql::query::indexed_stage_height(pool, crate::index::Stage::Block).await?;

  if pg_block_height < 0 {
    tracing::debug!(
      rocks_tip,
      "skipping RocksDB/PG startup coverage check while PostgreSQL block index is empty"
    );
    return Ok(());
  }

  let pg_block_height = pg_block_height as u32;
  if pg_block_height > rocks_tip {
    let divergence = pg_block_height - rocks_tip;
    return Err(DaemonError::Db(format!(
      "PostgreSQL block index ahead of RocksDB by {divergence} heights"
    )));
  }

  tracing::debug!(
    rocks_tip,
    pg_block_height,
    "RocksDB tip covers PostgreSQL block index"
  );
  Ok(())
}

/// Verify that PostgreSQL either has no indexed blocks or has undo coverage up
/// to its canonical tip.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when chain-progress reads fail, or
/// `DaemonError::Index` when indexed PostgreSQL state exists without matching
/// undo rows.
pub async fn verify_pg_undo_coverage_or_empty(pool: &SqlPool) -> Result<(), DaemonError> {
  let progress = crate::sql::progress::chain_progress(pool).await?;
  if progress.canonical_tip_height < 0 {
    tracing::debug!("skipping undo coverage check while PostgreSQL block index is empty");
    return Ok(());
  }

  let expected_rows = i64::from(progress.canonical_tip_height) + 1;
  let covered_rows =
    crate::sql::progress::count_block_undo_rows_through(pool, progress.canonical_tip_height)
      .await?;

  if progress.undo_tip_height >= progress.canonical_tip_height && covered_rows == expected_rows {
    tracing::debug!(
      canonical_tip = progress.canonical_tip_height,
      undo_tip = progress.undo_tip_height,
      covered_rows,
      "undo coverage reaches canonical PostgreSQL tip"
    );
    return Ok(());
  }

  Err(DaemonError::Index(format!(
    "postgres canonical tip {} lacks contiguous undo coverage (undo_tip={}, covered_rows={}, expected_rows={}); wipe or reindex postgres before starting this build",
    progress.canonical_tip_height, progress.undo_tip_height, covered_rows, expected_rows
  )))
}

/// Clamp durable stages when `sync_state` overclaims applied height.
///
/// Sparse stages such as spent/script/validator are intentionally skipped
/// because a missing durable row does not mean the stage failed to process
/// a block.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects a read or update, or
/// `DaemonError::Index` when a stage has no durable-height query.
pub(crate) async fn clamp_sync_state(
  pool: &SqlPool,
  index_flags: crate::index::IndexStageFlags,
) -> Result<(), DaemonError> {
  for stage in clampable_stages(index_flags) {
    let stage_name = stage.name();
    let claimed = crate::sql::query::indexed_stage_height(pool, stage).await?;
    let actual = actual_stage_height(pool, stage).await?;
    if claimed <= actual {
      continue;
    }

    tracing::warn!(
      stage = stage_name,
      claimed,
      actual,
      "sync_state ahead of durable data, clamping"
    );
    crate::sql::query::update_indexed_stage_height(pool, stage, actual).await?;
  }

  if index_flags.address || index_flags.validator {
    tracing::debug!(
      address_enabled = index_flags.address,
      validator_enabled = index_flags.validator,
      "skipping sparse-stage sync_state clamp during startup"
    );
  }

  Ok(())
}

fn clampable_stages(index_flags: crate::index::IndexStageFlags) -> Vec<crate::index::Stage> {
  let mut stages = vec![crate::index::Stage::Block, crate::index::Stage::Transaction];
  if index_flags.metrics {
    stages.push(crate::index::Stage::Metrics);
  }
  stages
}

async fn actual_stage_height(
  pool: &SqlPool,
  stage: crate::index::Stage,
) -> Result<i32, DaemonError> {
  crate::sql::query::durable_stage_height(pool, stage).await
}

/// Centralised startup recovery sequence that replaces the ad-hoc checks
/// previously scattered across `ServeRuntime::build()` and `CoreVisor::start()`.
///
/// The coordinator runs all recovery phases in the correct order:
///
/// 1. Resume an interrupted PG rebuild (if `chain_progress.recovery_state = 'rebuilding'`)
/// 2. Reconcile PG with RocksDB (fixes PG-ahead states)
/// 3. Post-reconcile assertions (`verify_rocks_covers_pg`, `verify_pg_undo_coverage_or_empty`)
/// 4. Repair dirty / ahead secondary stores
/// 5. Clamp `sync_state` to durable heights
/// 6. Catch up lagging secondary stores
/// 7. Verify recent store data
pub(crate) struct RecoveryCoordinator<'a> {
  pub db: &'a Arc<rocksdb::DB>,
  pub pool: &'a SqlPool,
  pub stores: Arc<SecondaryStoreSet>,
  pub datadir: &'a Path,
  pub chain: &'a crate::config::ChainConfig,
  pub rocksdb_cfg: Option<&'a crate::config::RocksdbConfig>,
  pub index_flags: crate::index::IndexStageFlags,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub block_time_secs: f64,
}

impl<'a> RecoveryCoordinator<'a> {
  /// Run the full startup recovery sequence.
  ///
  /// Returns the (possibly reopened) secondary store set.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` when any recovery phase fails.
  pub async fn run(self) -> Result<Arc<SecondaryStoreSet>, DaemonError> {
    // Phase 0: Clean up stale rebuild staging directories from a previous crash.
    crate::store::manager::cleanup_stale_rebuild_dirs(self.datadir);

    // Phase 1: Resume interrupted PG rebuild.
    // If chain_progress.recovery_state = 'rebuilding', the previous reindex
    // was interrupted. The verify_pg_undo_coverage_or_empty check in Phase 3
    // will catch this and refuse startup with a "wipe or reindex" message,
    // which is safe behaviour until an automatic resume is implemented.

    // Phase 2: Reconcile PG with RocksDB — fixes PG-ahead states before
    // verify_rocks_covers_pg can reject them.
    let reconcile = crate::sql::verbs::rewind::reconcile_postgres_with_local_rocks(
      self.pool,
      Arc::clone(self.db),
      Some(Arc::clone(&self.stores)),
      self.p2pkh_version,
      self.p2sh_version,
      self.block_time_secs,
    )
    .await?;
    tracing::info!(
      pg_block_before = reconcile.pg_block_before,
      pg_block_after = reconcile.pg_block_after,
      local_tip_before_repair = reconcile.local_tip_before_repair,
      local_tip_after_repair = reconcile.local_tip_after_repair,
      disconnected_heights = reconcile.disconnected_heights,
      clamped_sync_rows = reconcile.clamped_sync_rows,
      stores_invalidated = reconcile.stores_invalidated,
      "startup reconciliation complete"
    );

    // Phase 3: Post-reconcile assertions (now succeed because PG was rewound).
    verify_rocks_covers_pg(self.pool, self.db).await?;
    verify_pg_undo_coverage_or_empty(self.pool).await?;

    // Phase 4: Repair dirty / ahead secondary stores.
    let pg_tip = {
      let progress = crate::sql::progress::chain_progress(self.pool).await?;
      (progress.canonical_tip_height >= 0).then_some(progress.canonical_tip_height as u32)
    };

    // If stores were invalidated during reconcile, drop handles and reset dirs.
    let stores = if reconcile.stores_invalidated {
      let datadir = self.stores.datadir().to_path_buf();
      drop(self.stores);
      for id in StoreId::ALL {
        store::reset_store_dir(&datadir, id)?;
      }
      tracing::warn!(
        "secondary store directories reset after reconciliation; rebuilding from RocksDB"
      );
      let reopened = SecondaryStoreSet::open(&datadir, self.chain, self.rocksdb_cfg)?;
      Arc::new(reopened)
    } else {
      let owned_stores = Arc::try_unwrap(self.stores).map_err(|_| {
        DaemonError::Index(
          "cannot unwrap Arc<SecondaryStoreSet> for repair — other references still held".into(),
        )
      })?;
      match repair_secondary_stores(
        owned_stores,
        pg_tip,
        self.datadir,
        self.chain,
        self.rocksdb_cfg,
        false,
      )? {
        Some(reopened) => Arc::new(reopened),
        None => Arc::new(SecondaryStoreSet::open(self.datadir, self.chain, self.rocksdb_cfg)?),
      }
    };

    // Phase 5: Clamp sync_state to durable heights.
    clamp_sync_state(self.pool, self.index_flags).await?;

    // Phase 6: Catch up lagging secondary stores.
    crate::store::manager::catch_up_recent(self.db, &stores, pg_tip, self.chain)?;

    // Phase 7: Verify recent store data.
    crate::store::manager::verify_recent(self.db, &stores, pg_tip, self.chain, 500)?;

    // Refresh summaries that disconnect operations may have invalidated.
    crate::sql::index::tx::refresh_transactions_summary(self.pool).await?;
    crate::sql::index::spent::refresh_address_balances_summary_pool(self.pool).await?;

    Ok(stores)
  }
}

/// Auto-repair secondary stores that are dirty, ahead of PG, or stuck in a
/// recovery state after a crash.
///
/// Returns `Some(reopened)` if any store was reset and the caller should use the
/// new set, or `None` if the existing set is fine.
///
/// # Errors
///
/// Returns `DaemonError` when store metadata cannot be read or a store
/// directory cannot be reset/reopened.
pub(crate) fn repair_secondary_stores(
  stores: SecondaryStoreSet,
  pg_canonical_tip: Option<u32>,
  datadir: &Path,
  chain: &crate::config::ChainConfig,
  rocksdb_cfg: Option<&crate::config::RocksdbConfig>,
  stores_invalidated: bool,
) -> Result<Option<SecondaryStoreSet>, DaemonError> {
  if stores_invalidated {
    tracing::warn!("stores_invalidated: resetting all secondary store directories");
    drop(stores);
    for id in StoreId::ALL {
      store::reset_store_dir(datadir, id)?;
    }
    let reopened = SecondaryStoreSet::open(datadir, chain, rocksdb_cfg)?;
    return Ok(Some(reopened));
  }

  // First pass: collect decisions while we still hold the handles.
  let mut ids_to_reset: Vec<StoreId> = Vec::new();
  let mut ids_to_mark_clean: Vec<StoreId> = Vec::new();

  for id in StoreId::ALL {
    let meta = stores.meta(id)?;
    let state = meta
      .recovery_state
      .unwrap_or(RecoveryState::Clean);

    match state {
      RecoveryState::Disconnecting | RecoveryState::Rebuilding => {
        tracing::warn!(
          store = id.as_str(),
          recovery_state = state.as_str(),
          "secondary store stuck in recovery state; resetting"
        );
        ids_to_reset.push(id);
      }
      RecoveryState::Verifying => {
        tracing::info!(
          store = id.as_str(),
          "secondary store stuck in verifying state; marking clean (verify is read-only)"
        );
        ids_to_mark_clean.push(id);
      }
      RecoveryState::Clean => {
        // Check if store tip is ahead of PG canonical tip.
        if let Some(pg_tip) = pg_canonical_tip {
          if meta.tip_height.is_some_and(|store_tip| store_tip > pg_tip) {
            tracing::warn!(
              store = id.as_str(),
              store_tip = ?meta.tip_height,
              pg_tip,
              "secondary store tip ahead of PG canonical tip; resetting"
            );
            ids_to_reset.push(id);
          }
        }
      }
    }
  }

  // Mark verifying stores clean while handles are still open.
  for id in &ids_to_mark_clean {
    store::mark_recovery_state(stores.db(*id)?, RecoveryState::Clean, None)?;
  }

  if ids_to_reset.is_empty() {
    return Ok(None);
  }

  // Drop all RocksDB handles before resetting directories on disk.
  drop(stores);

  for id in &ids_to_reset {
    store::reset_store_dir(datadir, *id)?;
  }

  let reopened = SecondaryStoreSet::open(datadir, chain, rocksdb_cfg)?;
  Ok(Some(reopened))
}

#[cfg(test)]
mod tests {
  use super::clampable_stages;

  #[test]
  fn clampable_stages_skip_sparse_indices() {
    assert_eq!(
      clampable_stages(crate::index::IndexStageFlags {
        address: false,
        metrics: false,
        validator: false,
      }),
      vec![crate::index::Stage::Block, crate::index::Stage::Transaction]
    );
    assert_eq!(
      clampable_stages(crate::index::IndexStageFlags {
        address: false,
        metrics: true,
        validator: false,
      }),
      vec![
        crate::index::Stage::Block,
        crate::index::Stage::Transaction,
        crate::index::Stage::Metrics
      ]
    );
  }
}
