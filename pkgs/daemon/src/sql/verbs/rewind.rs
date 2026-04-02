//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Rewind-owned rollback helpers used by the IPC control plane.

use std::sync::Arc;
use std::time::Instant;

use crate::db;
use crate::error::DaemonError;
use crate::index;
use crate::index::Stage;
use crate::policy::log::log_threshold_met;
use crate::sql;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartupReconcileReport {
  pub pg_block_before: i32,
  pub pg_block_after: i32,
  pub local_tip_before_repair: Option<u32>,
  pub local_tip_after_repair: Option<u32>,
  pub disconnected_heights: u32,
  pub clamped_sync_rows: u64,
}

async fn current_canonical_height(pool: &sqlx::PgPool) -> Result<i32, DaemonError> {
  match crate::sql::progress::chain_progress(pool).await {
    Ok(progress) => Ok(progress.canonical_tip_height),
    Err(DaemonError::Pg(_)) => sql::query::indexed_stage_height(pool, Stage::Block).await,
    Err(error) => Err(error),
  }
}

async fn clamp_all_stage_heights_to_local_floor(
  pool: &sqlx::PgPool,
  local_tip: Option<u32>,
) -> Result<u64, DaemonError> {
  let safe_height = local_tip.map_or(-1, |height| height as i32);
  let stage_names: Vec<&str> = Stage::Block
    .reset_stages()
    .iter()
    .copied()
    .map(Stage::name)
    .collect();
  let changed = sqlx::query(
    "UPDATE sync_state
        SET indexed_height = $1
      WHERE stage = ANY($2)
        AND indexed_height > $1",
  )
  .bind(safe_height)
  .bind(&stage_names)
  .execute(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("clamp sync_state to local floor: {e}")))?;

  Ok(changed.rows_affected())
}

fn reconcile_local_tip_metadata(db: &rocksdb::DB) -> Result<Option<u32>, DaemonError> {
  let Some(current_tip) = db::get_tip_height(db) else {
    return Ok(None);
  };
  if db::get_hash_for_height(db, current_tip).is_some() {
    return Ok(Some(current_tip));
  }

  let heights_cf = db
    .cf_handle(db::CF_HEIGHTS)
    .ok_or_else(|| DaemonError::Db("heights CF missing".to_owned()))?;
  let meta_cf = db
    .cf_handle(db::CF_META)
    .ok_or_else(|| DaemonError::Db("meta CF missing".to_owned()))?;
  let mut cursor = current_tip;
  loop {
    if let Some(hash) = db::get_hash_for_height(db, cursor) {
      let mut batch = rocksdb::WriteBatch::default();
      batch.delete_cf(heights_cf, current_tip.to_be_bytes());
      db::put_tip_hash(&mut batch, meta_cf, &hash);
      db::put_tip_height(&mut batch, meta_cf, cursor);
      db.write(batch)
        .map_err(|e| DaemonError::Db(format!("repair local RocksDB tip metadata: {e}")))?;
      return Ok(Some(cursor));
    }
    if cursor == 0 {
      let mut batch = rocksdb::WriteBatch::default();
      batch.delete_cf(meta_cf, db::META_TIP_HASH);
      batch.delete_cf(meta_cf, db::META_TIP_HEIGHT);
      db.write(batch)
        .map_err(|e| DaemonError::Db(format!("clear orphaned RocksDB tip metadata: {e}")))?;
      return Ok(None);
    }
    cursor = cursor.saturating_sub(1);
  }
}

fn block_present_at_height(db: &rocksdb::DB, height: u32) -> Result<bool, DaemonError> {
  let Some(hash) = db::get_hash_for_height(db, height) else {
    return Ok(false);
  };
  let blocks_cf = db
    .cf_handle(db::CF_BLOCKS)
    .ok_or_else(|| DaemonError::Db("blocks CF missing".to_owned()))?;
  Ok(
    db.get_cf(blocks_cf, hash)
      .map_err(|e| DaemonError::Db(format!("read block bytes at height {height}: {e}")))?
      .is_some(),
  )
}

fn set_local_tip_metadata(db: &rocksdb::DB, tip: Option<u32>) -> Result<Option<u32>, DaemonError> {
  let meta_cf = db
    .cf_handle(db::CF_META)
    .ok_or_else(|| DaemonError::Db("meta CF missing".to_owned()))?;
  let mut batch = rocksdb::WriteBatch::default();
  match tip {
    Some(height) => {
      let hash = db::get_hash_for_height(db, height).ok_or_else(|| {
        DaemonError::Db(format!(
          "missing height mapping while setting local tip {height}"
        ))
      })?;
      db::put_tip_hash(&mut batch, meta_cf, &hash);
      db::put_tip_height(&mut batch, meta_cf, height);
    }
    None => {
      batch.delete_cf(meta_cf, db::META_TIP_HASH);
      batch.delete_cf(meta_cf, db::META_TIP_HEIGHT);
    }
  }
  db.write(batch)
    .map_err(|e| DaemonError::Db(format!("update local RocksDB tip metadata: {e}")))?;
  Ok(tip)
}

fn finalize_local_disconnect(db: &rocksdb::DB, height: u32) -> Result<Option<u32>, DaemonError> {
  let heights_cf = db
    .cf_handle(db::CF_HEIGHTS)
    .ok_or_else(|| DaemonError::Db("heights CF missing".to_owned()))?;
  let meta_cf = db
    .cf_handle(db::CF_META)
    .ok_or_else(|| DaemonError::Db("meta CF missing".to_owned()))?;

  let mut batch = rocksdb::WriteBatch::default();
  batch.delete_cf(heights_cf, height.to_be_bytes());

  if height == 0 {
    batch.delete_cf(meta_cf, db::META_TIP_HASH);
    batch.delete_cf(meta_cf, db::META_TIP_HEIGHT);
    db.write(batch)
      .map_err(|e| DaemonError::Db(format!("rewind rocksdb write: {e}")))?;
    return Ok(None);
  }

  let prev_height = height - 1;
  let prev_hash = db::get_hash_for_height(db, prev_height).ok_or_else(|| {
    DaemonError::Index(format!(
      "missing RocksDB block hash for height {prev_height}"
    ))
  })?;
  db::put_tip_hash(&mut batch, meta_cf, &prev_hash);
  db::put_tip_height(&mut batch, meta_cf, prev_height);
  db.write(batch)
    .map_err(|e| DaemonError::Db(format!("rewind rocksdb write: {e}")))?;
  Ok(Some(prev_height))
}

async fn finish_pending_local_disconnect(
  pool: &sqlx::PgPool,
  db: &rocksdb::DB,
) -> Result<(), DaemonError> {
  let progress = crate::sql::progress::chain_progress(pool).await?;
  if progress.recovery_state != "disconnecting" {
    return Ok(());
  }
  let Some(height) = progress.recovery_height else {
    return Err(DaemonError::Index(
      "chain_progress disconnecting state missing recovery height".to_owned(),
    ));
  };
  let height = height as u32;
  let local_tip = db::get_tip_height(db);
  if local_tip.is_some_and(|tip| tip >= height) {
    finalize_local_disconnect(db, height)?;
  }
  crate::sql::progress::mark_recovery_state(pool, "clean", None).await?;
  Ok(())
}

fn reconcile_forward_frontier(
  db: &rocksdb::DB,
  current_pg: i32,
  local_tip: Option<u32>,
) -> Result<Option<u32>, DaemonError> {
  let Some(local_tip) = local_tip else {
    return Ok(None);
  };
  let start = if current_pg < 0 { 0 } else { current_pg as u32 };
  if local_tip <= start {
    return Ok(Some(local_tip));
  }

  let next_height = start.saturating_add(1);
  if !block_present_at_height(db, next_height)? {
    tracing::warn!(
      gap_height = next_height,
      local_tip,
      usable_tip = start,
      "local RocksDB tip skipped missing next block height; lowering tip metadata to current PostgreSQL frontier"
    );
    return set_local_tip_metadata(db, Some(start));
  }

  Ok(Some(local_tip))
}

async fn disconnect_height_postgres_only(
  pool: &sqlx::PgPool,
  db: Option<Arc<rocksdb::DB>>,
  secondary_stores: Option<Arc<crate::store::manager::SecondaryStoreSet>>,
  height: u32,
  p2pkh_version: u8,
  p2sh_version: u8,
  block_time_secs: f64,
) -> Result<(), DaemonError> {
  let pipeline = index::DisconnectPipeline::new(
    db,
    secondary_stores,
    p2pkh_version,
    p2sh_version,
    block_time_secs,
    Arc::new(crate::index::spent::cache::SpentQueryCache::disabled()),
  );
  let plan = pipeline.plan_disconnect(pool, height).await?;
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("rewind begin: {e}")))?;
  plan.apply(&pipeline, &mut tx).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("rewind commit: {e}")))?;
  Ok(())
}

/// Rewind PostgreSQL only until it matches the locally available RocksDB tip.
///
/// # Errors
///
/// Returns `DaemonError` when RocksDB metadata repair fails or when PostgreSQL
/// disconnect planning/application fails.
pub async fn reconcile_postgres_with_local_rocks(
  pool: &sqlx::PgPool,
  db: Arc<rocksdb::DB>,
  secondary_stores: Option<Arc<crate::store::manager::SecondaryStoreSet>>,
  p2pkh_version: u8,
  p2sh_version: u8,
  block_time_secs: f64,
) -> Result<StartupReconcileReport, DaemonError> {
  finish_pending_local_disconnect(pool, &db).await?;
  let current_pg = current_canonical_height(pool).await?;
  let local_tip_before_repair = db::get_tip_height(&db);
  let repaired_tip = reconcile_local_tip_metadata(&db)?;
  let local_tip = if current_pg < 0 {
    repaired_tip
  } else {
    reconcile_forward_frontier(&db, current_pg, repaired_tip)?
  };

  let Some(local_tip) = local_tip else {
    if current_pg < 0 {
      let clamped_sync_rows = clamp_all_stage_heights_to_local_floor(pool, None).await?;
      return Ok(StartupReconcileReport {
        pg_block_before: current_pg,
        pg_block_after: -1,
        local_tip_before_repair,
        local_tip_after_repair: None,
        disconnected_heights: 0,
        clamped_sync_rows,
      });
    }
    sql::index::tx::refresh_transactions_summary(pool).await?;
    let mut disconnected_heights = 0;
    for height in (0..=current_pg as u32).rev() {
      disconnect_height_postgres_only(
        pool,
        Some(Arc::clone(&db)),
        secondary_stores.clone(),
        height,
        p2pkh_version,
        p2sh_version,
        block_time_secs,
      )
      .await?;
      disconnected_heights += 1;
    }
    let clamped_sync_rows = clamp_all_stage_heights_to_local_floor(pool, None).await?;
    return Ok(StartupReconcileReport {
      pg_block_before: current_pg,
      pg_block_after: -1,
      local_tip_before_repair,
      local_tip_after_repair: None,
      disconnected_heights,
      clamped_sync_rows,
    });
  };

  if current_pg < 0 || current_pg as u32 <= local_tip {
    let clamped_sync_rows = clamp_all_stage_heights_to_local_floor(pool, Some(local_tip)).await?;
    return Ok(StartupReconcileReport {
      pg_block_before: current_pg,
      pg_block_after: current_pg,
      local_tip_before_repair,
      local_tip_after_repair: Some(local_tip),
      disconnected_heights: 0,
      clamped_sync_rows,
    });
  }

  tracing::warn!(
    current_pg,
    local_tip,
    "postgres indexed height ahead of local RocksDB tip; rewinding postgres to match local state"
  );
  sql::index::tx::refresh_transactions_summary(pool).await?;
  let mut disconnected_heights = 0;
  for height in ((local_tip + 1)..=current_pg as u32).rev() {
    disconnect_height_postgres_only(
      pool,
      Some(Arc::clone(&db)),
      secondary_stores.clone(),
      height,
      p2pkh_version,
      p2sh_version,
      block_time_secs,
    )
    .await?;
    disconnected_heights += 1;
  }
  let clamped_sync_rows = clamp_all_stage_heights_to_local_floor(pool, Some(local_tip)).await?;
  Ok(StartupReconcileReport {
    pg_block_before: current_pg,
    pg_block_after: local_tip as i32,
    local_tip_before_repair,
    local_tip_after_repair: Some(local_tip),
    disconnected_heights,
    clamped_sync_rows,
  })
}

async fn disconnect_height(
  pool: &sqlx::PgPool,
  db: Arc<rocksdb::DB>,
  secondary_stores: Option<Arc<crate::store::manager::SecondaryStoreSet>>,
  height: u32,
  p2pkh_version: u8,
  p2sh_version: u8,
  block_time_secs: f64,
) -> Result<(), DaemonError> {
  let pipeline = index::DisconnectPipeline::new(
    Some(Arc::clone(&db)),
    secondary_stores,
    p2pkh_version,
    p2sh_version,
    block_time_secs,
    std::sync::Arc::new(crate::index::spent::cache::SpentQueryCache::disabled()),
  );
  let plan = pipeline.plan_disconnect(pool, height).await?;
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("rewind begin: {e}")))?;

  plan.apply(&pipeline, &mut tx).await?;

  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("rewind commit: {e}")))?;
  finalize_local_disconnect(&db, height)?;
  crate::sql::progress::mark_recovery_state(pool, "clean", None).await?;

  Ok(())
}

async fn disconnect_height_shared(
  pool: sqlx::PgPool,
  db: Arc<rocksdb::DB>,
  secondary_stores: Option<Arc<crate::store::manager::SecondaryStoreSet>>,
  height: u32,
  p2pkh_version: u8,
  p2sh_version: u8,
  block_time_secs: f64,
) -> Result<(), DaemonError> {
  let pipeline = index::DisconnectPipeline::new(
    Some(Arc::clone(&db)),
    secondary_stores,
    p2pkh_version,
    p2sh_version,
    block_time_secs,
    Arc::new(crate::index::spent::cache::SpentQueryCache::disabled()),
  );
  let plan = pipeline.plan_disconnect(&pool, height).await?;
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("rewind begin: {e}")))?;

  plan.apply(&pipeline, &mut tx).await?;

  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("rewind commit: {e}")))?;
  finalize_local_disconnect(&db, height)?;
  crate::sql::progress::mark_recovery_state(&pool, "clean", None).await?;

  Ok(())
}

pub(crate) fn rewind_target_after_disconnect(current: u32) -> Option<u32> {
  current.checked_sub(1)
}

fn rewind_start_height(current_pg: i32, current_rocks: Option<u32>) -> Option<u32> {
  if current_pg >= 0 {
    Some(current_pg as u32)
  } else {
    current_rocks
  }
}

/// Rewind all indices to `target_height`.
///
/// # Errors
///
/// Returns `DaemonError::Pg`, `DaemonError::Db`, or `DaemonError::Index`
/// when disconnect planning, SQL commit, or RocksDB tip rewrites fail.
pub async fn rewind_to_height(
  pool: &sqlx::PgPool,
  db: &Arc<rocksdb::DB>,
  secondary_stores: Option<Arc<crate::store::manager::SecondaryStoreSet>>,
  target_height: u32,
  p2pkh_version: u8,
  p2sh_version: u8,
  block_time_secs: f64,
) -> Result<String, DaemonError> {
  let started = Instant::now();
  let current_pg = current_canonical_height(pool).await?;
  let current_rocks = db::get_tip_height(db);

  if current_pg < 0 && current_rocks.is_none() {
    return Ok("nothing to rewind, block index is empty".to_string());
  }

  let Some(current) = rewind_start_height(current_pg, current_rocks) else {
    return Ok("nothing to rewind, block index is empty".to_string());
  };
  if target_height >= current {
    return Ok(format!("already at or below height {target_height}"));
  }

  tracing::info!(
    start_height = current,
    target_height,
    blocks = current.saturating_sub(target_height),
    "Starting block-range rewind"
  );

  let mut last_progress_log: Option<(u32, Instant)> = None;
  for height in ((target_height + 1)..=current).rev() {
    disconnect_height(
      pool,
      Arc::clone(db),
      secondary_stores.clone(),
      height,
      p2pkh_version,
      p2sh_version,
      block_time_secs,
    )
    .await?;
    let rewound_to = height.saturating_sub(1);
    if log_threshold_met(last_progress_log, rewound_to, target_height) {
      tracing::info!(
        current_height = rewound_to,
        target_height,
        remaining_blocks = rewound_to.saturating_sub(target_height),
        rewind_pct = ((current.saturating_sub(rewound_to)) * 100)
          / current.saturating_sub(target_height).max(1),
        "block-range rewind progress"
      );
      last_progress_log = Some((rewound_to, Instant::now()));
    }
  }

  tracing::info!(
    start_height = current,
    target_height,
    elapsed_ms = started.elapsed().as_millis() as u64,
    "Finished block-range rewind"
  );
  Ok(format!(
    "rewound {} blocks, now at height {target_height}",
    current - target_height
  ))
}

/// Rewind all indices to `target_height` using owned shared handles.
///
/// # Errors
///
/// Returns the same errors as [`rewind_to_height`].
pub async fn rewind_to_height_shared(
  pool: sqlx::PgPool,
  db: Arc<rocksdb::DB>,
  secondary_stores: Option<Arc<crate::store::manager::SecondaryStoreSet>>,
  target_height: u32,
  p2pkh_version: u8,
  p2sh_version: u8,
  block_time_secs: f64,
) -> Result<String, DaemonError> {
  let started = Instant::now();
  let current_pg = current_canonical_height(&pool).await?;
  let current_rocks = db::get_tip_height(&db);

  if current_pg < 0 && current_rocks.is_none() {
    return Ok("nothing to rewind, block index is empty".to_string());
  }

  let Some(current) = rewind_start_height(current_pg, current_rocks) else {
    return Ok("nothing to rewind, block index is empty".to_string());
  };
  if target_height >= current {
    return Ok(format!("already at or below height {target_height}"));
  }

  // Rewind deletes rely on incremental summary deltas; refresh once up front
  // so a stale singleton cannot fail the first block disconnect.
  sql::index::tx::refresh_transactions_summary(&pool).await?;

  tracing::info!(
    start_height = current,
    target_height,
    blocks = current.saturating_sub(target_height),
    "Starting block-range rewind"
  );

  let mut last_progress_log: Option<(u32, Instant)> = None;
  for height in ((target_height + 1)..=current).rev() {
    disconnect_height_shared(
      pool.clone(),
      Arc::clone(&db),
      secondary_stores.clone(),
      height,
      p2pkh_version,
      p2sh_version,
      block_time_secs,
    )
    .await?;
    let rewound_to = height.saturating_sub(1);
    if log_threshold_met(last_progress_log, rewound_to, target_height) {
      tracing::info!(
        current_height = rewound_to,
        target_height,
        remaining_blocks = rewound_to.saturating_sub(target_height),
        rewind_pct = ((current.saturating_sub(rewound_to)) * 100)
          / current.saturating_sub(target_height).max(1),
        "block-range rewind progress"
      );
      last_progress_log = Some((rewound_to, Instant::now()));
    }
  }

  tracing::info!(
    start_height = current,
    target_height,
    elapsed_ms = started.elapsed().as_millis() as u64,
    "Finished block-range rewind"
  );
  Ok(format!(
    "rewound {} blocks, now at height {target_height}",
    current - target_height
  ))
}

/// Disconnect the single block at the current tip.
///
/// # Errors
///
/// Returns the same errors as [`rewind_to_height`] when planning or applying
/// the disconnect fails.
#[cfg_attr(not(test), allow(dead_code))]
pub async fn disconnect_tip(
  pool: &sqlx::PgPool,
  db: &Arc<rocksdb::DB>,
  secondary_stores: Option<Arc<crate::store::manager::SecondaryStoreSet>>,
  p2pkh_version: u8,
  p2sh_version: u8,
  block_time_secs: f64,
) -> Result<String, DaemonError> {
  let current = current_canonical_height(pool).await?;

  if current < 0 {
    return Ok("nothing to disconnect, block index is empty".to_string());
  }

  let current = current as u32;
  let Some(target) = rewind_target_after_disconnect(current) else {
    disconnect_height(
      pool,
      Arc::clone(db),
      secondary_stores.clone(),
      current,
      p2pkh_version,
      p2sh_version,
      block_time_secs,
    )
    .await?;
    return Ok("disconnected 1 block, block index is now empty".to_string());
  };

  rewind_to_height(
    pool,
    db,
    secondary_stores,
    target,
    p2pkh_version,
    p2sh_version,
    block_time_secs,
  )
  .await
}

/// Disconnect the single block at the current tip using owned shared handles.
///
/// # Errors
///
/// Returns the same errors as [`disconnect_tip`].
pub async fn disconnect_tip_shared(
  pool: sqlx::PgPool,
  db: Arc<rocksdb::DB>,
  secondary_stores: Option<Arc<crate::store::manager::SecondaryStoreSet>>,
  p2pkh_version: u8,
  p2sh_version: u8,
  block_time_secs: f64,
) -> Result<String, DaemonError> {
  let current = current_canonical_height(&pool).await?;

  if current < 0 {
    return Ok("nothing to disconnect, block index is empty".to_string());
  }

  sql::index::tx::refresh_transactions_summary(&pool).await?;

  let current = current as u32;
  let Some(target) = rewind_target_after_disconnect(current) else {
    disconnect_height_shared(
      pool,
      db,
      secondary_stores.clone(),
      current,
      p2pkh_version,
      p2sh_version,
      block_time_secs,
    )
    .await?;
    return Ok("disconnected 1 block, block index is now empty".to_string());
  };

  rewind_to_height_shared(
    pool,
    db,
    secondary_stores,
    target,
    p2pkh_version,
    p2sh_version,
    block_time_secs,
  )
  .await
}

#[cfg(test)]
mod tests {
  use super::{rewind_start_height, rewind_target_after_disconnect};

  #[test]
  fn disconnect_tip_target_handles_genesis() {
    assert_eq!(rewind_target_after_disconnect(0), None);
    assert_eq!(rewind_target_after_disconnect(7), Some(6));
  }

  #[test]
  fn rewind_prefers_current_postgres_height_over_future_rocks_tip() {
    assert_eq!(rewind_start_height(720_353, Some(1_873_921)), Some(720_353));
    assert_eq!(rewind_start_height(-1, Some(1_873_921)), Some(1_873_921));
    assert_eq!(rewind_start_height(-1, None), None);
  }
}
