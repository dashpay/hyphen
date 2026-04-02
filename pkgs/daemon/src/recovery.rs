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

use std::sync::Arc;

use crate::sql::SqlPool;

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
    crate::sql::progress::count_block_undo_rows_through(pool, progress.canonical_tip_height).await?;

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
