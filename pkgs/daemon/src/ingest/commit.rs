//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! RocksDB commit task.

use rocksdb::WriteBatch;

use crate::consensus;

use crate::db::{self, CF_BLOCKS, CF_HEIGHTS, CF_META};
use crate::error::DaemonError;

use super::state::{FetchedBlock, InFlightBatch};
use super::IngestCtx;

/// Commit one fetched block batch into RocksDB.
///
/// # Errors
///
/// Returns `DaemonError::Db` when one required column family is missing or
/// RocksDB rejects the write batch.
pub fn commit_batch(
  ctx: &IngestCtx,
  batch: &InFlightBatch,
  blocks: &[FetchedBlock],
) -> Result<(), DaemonError> {
  let blocks_cf = ctx
    .db
    .cf_handle(CF_BLOCKS)
    .ok_or_else(|| DaemonError::Db("blocks CF missing".to_owned()))?;
  let heights_cf = ctx
    .db
    .cf_handle(CF_HEIGHTS)
    .ok_or_else(|| DaemonError::Db("heights CF missing".to_owned()))?;
  let meta_cf = ctx
    .db
    .cf_handle(CF_META)
    .ok_or_else(|| DaemonError::Db("meta CF missing".to_owned()))?;

  ctx.runtime.record_commit_start(batch);
  let mut write_batch = WriteBatch::default();
  let mut last_hash_arr = [0u8; 32];
  for (offset, block) in blocks.iter().enumerate() {
    let height = batch.start_height.saturating_add(offset as u32);
    let hash_arr = consensus::block_hash(&block.raw)
      .map_err(|error| DaemonError::Index(format!("hash fetched block {height}: {error}")))?
      .to_bytes();
    write_batch.put_cf(blocks_cf, hash_arr, &block.raw);
    write_batch.put_cf(heights_cf, height.to_be_bytes(), hash_arr);
    last_hash_arr = hash_arr;
  }

  db::put_tip_hash(&mut write_batch, meta_cf, &last_hash_arr);
  db::put_tip_height(&mut write_batch, meta_cf, batch.end_height);
  ctx
    .db
    .write(write_batch)
    .map_err(|error| DaemonError::Db(format!("RocksDB batch write: {error}")))?;
  ctx.runtime.record_progress(batch);
  Ok(())
}
