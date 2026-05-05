//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block index: stores block headers and coinbase fields in PostgreSQL.

pub(crate) mod ipc;
pub(crate) mod overlay;
#[cfg(feature = "tests")]
pub mod rest;
#[cfg(not(feature = "tests"))]
pub(crate) mod rest;
pub mod util;

use std::sync::Mutex;

use num_bigint::BigUint;

use crate::error::DaemonError;
pub(crate) use crate::sql::index::block as sql;

use super::{BlockCtx, IndexOps};

pub struct BlockIndex {
  cache: Mutex<Option<(u32, BigUint)>>,
}

impl BlockIndex {
  pub fn new() -> Self {
    Self {
      cache: Mutex::new(None),
    }
  }
}

impl std::fmt::Debug for BlockIndex {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("BlockIndex").finish()
  }
}

#[async_trait::async_trait]
impl IndexOps for BlockIndex {
  async fn disconnect_block(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    height: u32,
  ) -> Result<(), DaemonError> {
    sql::delete_block(tx, height).await?;
    *self
      .cache
      .lock()
      .map_err(|e| DaemonError::Index(format!("cache lock: {e}")))? = None;
    Ok(())
  }

  fn reset_cache(&self) {
    if let Ok(mut c) = self.cache.lock() {
      *c = None;
    }
  }
}

impl BlockIndex {
  pub(crate) async fn plan_connect(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    block: &BlockCtx,
  ) -> Result<BlockInsertPlan, DaemonError> {
    let previous_chainwork = match self.cached_chainwork(block.height())? {
      Some(chainwork) => chainwork,
      None => sql::prev_chainwork(tx, block.height()).await?,
    };
    BlockInsertPlan::build(block, previous_chainwork)
  }

  pub(crate) async fn apply_plan(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    plan: &BlockInsertPlan,
  ) -> Result<(), DaemonError> {
    plan.apply(tx).await?;
    self.store_chainwork(plan.height, plan.total_chainwork.clone())?;
    Ok(())
  }

  fn cached_chainwork(&self, height: u32) -> Result<Option<BigUint>, DaemonError> {
    if height == 0 {
      return Ok(Some(BigUint::ZERO));
    }

    let cached = self
      .cache
      .lock()
      .map_err(|e| DaemonError::Index(format!("cache lock: {e}")))?;
    Ok(cached.as_ref().and_then(|(cached_height, chainwork)| {
      if *cached_height == height - 1 {
        Some(chainwork.clone())
      } else {
        None
      }
    }))
  }

  fn store_chainwork(&self, height: u32, total_chainwork: BigUint) -> Result<(), DaemonError> {
    *self
      .cache
      .lock()
      .map_err(|e| DaemonError::Index(format!("cache lock: {e}")))? =
      Some((height, total_chainwork));
    Ok(())
  }
}

pub(crate) struct BlockInsertPlan {
  height: u32,
  block_hash: [u8; 32],
  prev_hash: [u8; 32],
  merkle_root: [u8; 32],
  time: i32,
  bits: i32,
  nonce: i64,
  version: i32,
  size: i32,
  tx_count: i32,
  chainwork: String,
  total_chainwork: BigUint,
  cbtx: util::CbTxFields,
}

impl BlockInsertPlan {
  fn build(block: &BlockCtx, previous_chainwork: BigUint) -> Result<Self, DaemonError> {
    let header = &block.block().header;
    let height = block.height();
    let time = i32::try_from(header.time)
      .map_err(|_| DaemonError::Index(format!("block {height} time exceeds i32")))?;
    let bits = i32::try_from(header.bits)
      .map_err(|_| DaemonError::Index(format!("block {height} bits exceeds i32")))?;
    let nonce = i64::from(header.nonce);
    let total_chainwork = previous_chainwork + util::block_proof(header.bits);

    Ok(Self {
      height,
      block_hash: block.block_hash(),
      prev_hash: header.prev_hash.to_bytes(),
      merkle_root: header.merkle_root.to_bytes(),
      time,
      bits,
      nonce,
      version: header.version,
      size: block.raw().len() as i32,
      tx_count: block.block().transactions.len() as i32,
      chainwork: format!("{total_chainwork:064x}"),
      total_chainwork,
      cbtx: util::extract_cbtx_fields(block.block(), height),
    })
  }

  async fn apply(&self, tx: &mut crate::sql::SqlTx<'_>) -> Result<(), DaemonError> {
    let created_at = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|d| d.as_secs() as i64)
      .unwrap_or(0);
    crate::sql::undo::upsert_block_undo_meta(
      tx,
      self.height,
      &self.block_hash,
      &self.prev_hash,
      created_at,
    )
    .await?;
    sql::insert_block(
      tx,
      self.height,
      &self.block_hash,
      &self.prev_hash,
      &self.merkle_root,
      self.time,
      self.bits,
      self.nonce,
      self.version,
      self.size,
      self.tx_count,
      &self.chainwork,
      &self.cbtx,
    )
    .await
  }
}
