//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Tip reconciliation and local reorg repair.

use rocksdb::{WriteBatch, DB};
use std::time::Instant;

use crate::consensus;

use crate::db::{self, CF_META};
use crate::error::DaemonError;
use crate::utils::{self, HashEndian};

use super::IngestCtx;

/// Get the current stored tip height from RocksDB tip metadata.
pub(crate) fn stored_tip_height(db: &DB) -> Result<Option<u32>, DaemonError> {
  db::try_get_tip_height(db)
}

fn repair_tip_metadata_gap(db: &DB, current_tip: u32) -> Result<u32, DaemonError> {
  let meta_cf = db
    .cf_handle(CF_META)
    .ok_or_else(|| DaemonError::Db("meta CF missing".to_owned()))?;
  let mut cursor = current_tip;

  loop {
    if let Some(hash) = db::try_get_hash_for_height(db, cursor)? {
      let mut batch = WriteBatch::default();
      db::put_tip_hash(&mut batch, meta_cf, &hash);
      db::put_tip_height(&mut batch, meta_cf, cursor);
      db.write(batch)
        .map_err(|error| DaemonError::Db(format!("repair tip metadata gap: {error}")))?;
      return Ok(cursor.saturating_add(1));
    }

    if cursor == 0 {
      let mut batch = WriteBatch::default();
      db::delete_tip(&mut batch, meta_cf);
      db.write(batch)
        .map_err(|error| DaemonError::Db(format!("clear orphaned tip metadata: {error}")))?;
      return Ok(0);
    }

    cursor = cursor.saturating_sub(1);
  }
}

async fn canonical_hash_at_height(ctx: &IngestCtx, height: u32) -> Result<[u8; 32], DaemonError> {
  let requested_hash = ctx.source.block_hash(height).await?;
  let raw = ctx.source.raw_block(&requested_hash).await?;
  let computed_hash = consensus::block_hash(&raw)
    .map_err(|error| DaemonError::RestSource(format!("hash raw block {height}: {error}")))?
    .to_bytes();
  let computed_hex = utils::view_hex(&computed_hash, HashEndian::Ser);
  if computed_hex != requested_hash {
    return Err(DaemonError::RestSource(format!(
      "raw block hash mismatch at {height}: source returned {requested_hash}, raw hashes to {computed_hex}"
    )));
  }
  Ok(computed_hash)
}

async fn repair_reorg(ctx: &IngestCtx, current_tip: u32) -> Result<u32, DaemonError> {
  let mut cursor = current_tip;
  loop {
    let Some(stored_hash) = db::try_get_hash_for_height(&ctx.db, cursor)? else {
      if cursor == 0 {
        break;
      }
      cursor = cursor.saturating_sub(1);
      continue;
    };
    let canonical_hash = canonical_hash_at_height(ctx, cursor).await?;
    if stored_hash == canonical_hash {
      break;
    }
    if cursor == 0 {
      break;
    }
    cursor = cursor.saturating_sub(1);
  }

  tracing::warn!(
    rewind_to = cursor,
    "canonical reorg detected, rewinding ingest state"
  );
  let wait_started = Instant::now();
  let guard = ctx.chain_state_lock.clone().write_owned().await;
  let wait_ms = wait_started.elapsed().as_millis() as u64;
  if wait_ms >= 1_000 {
    tracing::warn!(
      reason = "reconcile_repair_reorg",
      wait_ms,
      "chain-state write lock waited"
    );
  } else {
    tracing::debug!(
      reason = "reconcile_repair_reorg",
      wait_ms,
      "chain-state write lock acquired"
    );
  }
  let hold_started = Instant::now();
  let result = crate::sql::verbs::rewind::rewind_to_height(
    &ctx.pool,
    &ctx.db,
    ctx.secondary_stores.clone(),
    cursor,
    ctx.p2pkh_version,
    ctx.p2sh_version,
    ctx.block_time_secs,
  )
  .await;
  let hold_ms = hold_started.elapsed().as_millis() as u64;
  drop(guard);
  tracing::info!(
    reason = "reconcile_repair_reorg",
    hold_ms,
    "chain-state write lock released"
  );
  result?;
  Ok(cursor.saturating_add(1))
}

/// Reconcile local RocksDB tip metadata against the canonical upstream view.
///
/// # Errors
///
/// Returns `DaemonError` when canonical hash lookups fail, local tip metadata
/// repair fails, or rewind execution fails.
pub async fn reconcile_start(ctx: &IngestCtx, start: u32) -> Result<u32, DaemonError> {
  let Some(current_tip) = stored_tip_height(&ctx.db)? else {
    return Ok(start);
  };
  ctx.runtime.record_stored_tip(current_tip);
  let Some(stored_hash) = db::try_get_hash_for_height(&ctx.db, current_tip)? else {
    tracing::warn!(
      current_tip,
      "tip metadata missing height mapping, repairing local ingest tip"
    );
    return repair_tip_metadata_gap(&ctx.db, current_tip);
  };
  let canonical_hash = canonical_hash_at_height(ctx, current_tip).await?;
  if stored_hash == canonical_hash {
    return Ok(start.min(current_tip.saturating_add(1)));
  }
  repair_reorg(ctx, current_tip).await
}

#[cfg(test)]
mod tests {
  use super::{canonical_hash_at_height, reconcile_start};
  use crate::index::BlockIndexerWorker;
  use crate::ingest::{IngestCtx, IngestRuntime};
  use crate::lifecycle::Lifecycle;
  use crate::sources::RestSource;
  use crate::utils::{self, HashEndian};
  use async_trait::async_trait;
  use rocksdb::WriteBatch;
  use std::sync::Arc;

  #[derive(Clone, Debug)]
  struct MismatchSource {
    hash: String,
    raw: Vec<u8>,
  }

  #[async_trait]
  impl RestSource for MismatchSource {
    async fn block_count(&self) -> Result<u32, crate::error::DaemonError> {
      Ok(1)
    }

    async fn block_hash(&self, _height: u32) -> Result<String, crate::error::DaemonError> {
      Ok(self.hash.clone())
    }

    async fn raw_block(&self, _hash: &str) -> Result<Vec<u8>, crate::error::DaemonError> {
      Ok(self.raw.clone())
    }

    async fn fetch_blocks_batch(
      &self,
      _heights: &[u32],
    ) -> Result<Vec<(String, Vec<u8>)>, crate::error::DaemonError> {
      Ok(vec![(self.hash.clone(), self.raw.clone())])
    }
  }

  #[tokio::test]
  async fn canonical_hash_at_height_rejects_mismatched_source_hash(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let db = Arc::new(crate::tests::open_temp_db()?);
    let runtime = Arc::new(IngestRuntime::default());
    let lifecycle = Lifecycle::start();
    let scheduler_state = Arc::new(std::sync::Mutex::new(
      crate::index::runner::IndexSchedulerState::default(),
    ));
    let worker = Arc::new(BlockIndexerWorker::new(
      Arc::clone(&db),
      Arc::clone(&runtime),
      Arc::clone(&lifecycle),
      scheduler_state,
      Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ));
    let ctx = IngestCtx {
      db,
      secondary_stores: None,
      pool: crate::sql::SqlPool::connect_lazy("postgresql://unused")?,
      source: Arc::new(MismatchSource {
        hash: "00".repeat(32),
        raw: crate::tests::minimal_block_bytes(1),
      }),
      lifecycle,
      runtime,
      block_worker: worker,
      chain_state_lock: Arc::new(tokio::sync::RwLock::new(())),
      batch_size: 1,
      max_lead_blocks: 1,
      p2pkh_version: 0x4c,
      p2sh_version: 0x10,
      block_time_secs: 150.0,
    };

    let err = canonical_hash_at_height(&ctx, 0)
      .await
      .expect_err("mismatched source hash should fail");
    assert!(err.to_string().contains("raw block hash mismatch"));
    Ok(())
  }

  #[tokio::test]
  async fn canonical_hash_at_height_accepts_display_endian_source_hash(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let raw = crate::tests::minimal_block_bytes(7);
    let computed = crate::consensus::block_hash(&raw)?.to_bytes();
    let db = Arc::new(crate::tests::open_temp_db()?);
    let runtime = Arc::new(IngestRuntime::default());
    let lifecycle = Lifecycle::start();
    let scheduler_state = Arc::new(std::sync::Mutex::new(
      crate::index::runner::IndexSchedulerState::default(),
    ));
    let worker = Arc::new(BlockIndexerWorker::new(
      Arc::clone(&db),
      Arc::clone(&runtime),
      Arc::clone(&lifecycle),
      scheduler_state,
      Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ));
    let ctx = IngestCtx {
      db,
      secondary_stores: None,
      pool: crate::sql::SqlPool::connect_lazy("postgresql://unused")?,
      source: Arc::new(MismatchSource {
        hash: utils::view_hex(&computed, HashEndian::Ser),
        raw,
      }),
      lifecycle,
      runtime,
      block_worker: worker,
      chain_state_lock: Arc::new(tokio::sync::RwLock::new(())),
      batch_size: 1,
      max_lead_blocks: 1,
      p2pkh_version: 0x4c,
      p2sh_version: 0x10,
      block_time_secs: 150.0,
    };

    assert_eq!(canonical_hash_at_height(&ctx, 0).await?, computed);
    Ok(())
  }

  fn test_ctx(
    db: Arc<rocksdb::DB>,
    source: Arc<dyn RestSource>,
  ) -> Result<IngestCtx, Box<dyn std::error::Error>> {
    let runtime = Arc::new(IngestRuntime::default());
    let lifecycle = Lifecycle::start();
    let scheduler_state = Arc::new(std::sync::Mutex::new(
      crate::index::runner::IndexSchedulerState::default(),
    ));
    let worker = Arc::new(BlockIndexerWorker::new(
      Arc::clone(&db),
      Arc::clone(&runtime),
      Arc::clone(&lifecycle),
      scheduler_state,
      Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ));
    Ok(IngestCtx {
      db,
      secondary_stores: None,
      pool: crate::sql::SqlPool::connect_lazy("postgresql://unused")?,
      source,
      lifecycle,
      runtime,
      block_worker: worker,
      chain_state_lock: Arc::new(tokio::sync::RwLock::new(())),
      batch_size: 1,
      max_lead_blocks: 1,
      p2pkh_version: 0x4c,
      p2sh_version: 0x10,
      block_time_secs: 150.0,
    })
  }

  #[tokio::test]
  async fn reconcile_start_rejects_malformed_tip_height_metadata(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let db = Arc::new(crate::tests::open_temp_db()?);
    let meta_cf = db
      .cf_handle(crate::db::CF_META)
      .ok_or("meta CF should exist")?;
    let mut batch = WriteBatch::default();
    batch.put_cf(meta_cf, crate::db::META_TIP_HEIGHT, [1_u8, 2, 3]);
    db.write(batch)?;

    let ctx = test_ctx(
      Arc::clone(&db),
      Arc::new(MismatchSource {
        hash: utils::view_hex(
          &crate::consensus::block_hash(&crate::tests::minimal_block_bytes(1))?.to_bytes(),
          HashEndian::Ser,
        ),
        raw: crate::tests::minimal_block_bytes(1),
      }),
    )?;
    let err = reconcile_start(&ctx, 0)
      .await
      .expect_err("malformed tip metadata should fail closed");
    assert!(err.to_string().contains("tip height"));
    Ok(())
  }

  #[tokio::test]
  async fn reconcile_start_rejects_malformed_height_mapping(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let raw = crate::tests::minimal_block_bytes(1);
    let computed = crate::consensus::block_hash(&raw)?.to_bytes();
    let db = Arc::new(crate::tests::open_temp_db()?);
    let meta_cf = db
      .cf_handle(crate::db::CF_META)
      .ok_or("meta CF should exist")?;
    let heights_cf = db
      .cf_handle(crate::db::CF_HEIGHTS)
      .ok_or("heights CF should exist")?;
    let mut batch = WriteBatch::default();
    crate::db::put_tip_height(&mut batch, meta_cf, 1);
    batch.put_cf(heights_cf, 1_u32.to_be_bytes(), [7_u8; 31]);
    db.write(batch)?;

    let ctx = test_ctx(
      Arc::clone(&db),
      Arc::new(MismatchSource {
        hash: utils::view_hex(&computed, HashEndian::Ser),
        raw,
      }),
    )?;
    let err = reconcile_start(&ctx, 0)
      .await
      .expect_err("malformed height mapping should fail closed");
    assert!(err.to_string().contains("height 1 mapping"));
    Ok(())
  }

  #[tokio::test]
  async fn reconcile_start_clamps_stale_resume_height_to_local_tip(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let raw = crate::tests::minimal_block_bytes(7);
    let computed = crate::consensus::block_hash(&raw)?.to_bytes();
    let db = Arc::new(crate::tests::open_temp_db()?);
    let meta_cf = db
      .cf_handle(crate::db::CF_META)
      .ok_or("meta CF should exist")?;
    let heights_cf = db
      .cf_handle(crate::db::CF_HEIGHTS)
      .ok_or("heights CF should exist")?;
    let blocks_cf = db
      .cf_handle(crate::db::CF_BLOCKS)
      .ok_or("blocks CF should exist")?;
    let mut batch = WriteBatch::default();
    batch.put_cf(heights_cf, 7_u32.to_be_bytes(), computed);
    batch.put_cf(blocks_cf, computed, raw.clone());
    crate::db::put_tip_hash(&mut batch, meta_cf, &computed);
    crate::db::put_tip_height(&mut batch, meta_cf, 7);
    db.write(batch)?;

    let ctx = test_ctx(
      Arc::clone(&db),
      Arc::new(MismatchSource {
        hash: utils::view_hex(&computed, HashEndian::Ser),
        raw,
      }),
    )?;

    assert_eq!(reconcile_start(&ctx, 42).await?, 8);
    Ok(())
  }
}
