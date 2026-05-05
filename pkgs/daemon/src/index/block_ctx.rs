//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Immutable per-block derived state shared across stage planning.

use dash_primitives::block::tx_byte_ranges;
use dash_primitives::hash;
use dash_primitives::Block;

use crate::error::DaemonError;

/// Immutable per-block derived state shared across stage planning.
#[derive(Clone)]
pub struct BlockCtx {
  block: Block,
  raw: Vec<u8>,
  height: u32,
  block_hash: [u8; 32],
  tx_byte_ranges: Vec<(usize, usize)>,
  txids: Vec<[u8; 32]>,
}

impl std::fmt::Debug for BlockCtx {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("BlockCtx")
      .field("height", &self.height)
      .field("txs", &self.txids.len())
      .finish()
  }
}

impl BlockCtx {
  /// Build a shared block context from raw RocksDB bytes.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` if the block cannot be decoded or any
  /// transaction byte range cannot be derived.
  pub fn from_raw(height: u32, raw: Vec<u8>) -> Result<Self, DaemonError> {
    let block = bitcoin_consensus_encoding::decode_from_slice::<Block>(&raw)
      .map_err(|e| DaemonError::Index(format!("parse block {height}: {e}")))?;
    let ranges = tx_byte_ranges(&raw)
      .map_err(|e| DaemonError::Index(format!("scan txs {height}: {e}")))?;
    if ranges.len() != block.transactions.len() {
      return Err(DaemonError::Index(format!(
        "block {height} tx range count {} does not match decoded tx count {}",
        ranges.len(),
        block.transactions.len()
      )));
    }

    let mut txids = Vec::with_capacity(ranges.len());
    for (start, end) in &ranges {
      txids.push(hash::tx_hash(&raw[*start..*end]).to_bytes());
    }

    Ok(Self {
      block_hash: crate::consensus::block_hash(&raw)
        .map_err(|e| DaemonError::Index(format!("hash block {height}: {e}")))?
        .to_bytes(),
      block,
      raw,
      height,
      tx_byte_ranges: ranges,
      txids,
    })
  }

  /// Return the decoded block shared across stage planning.
  pub fn block(&self) -> &Block {
    &self.block
  }

  /// Return the raw block bytes as read from RocksDB.
  pub fn raw(&self) -> &[u8] {
    &self.raw
  }

  /// Return the chain height of this block.
  pub fn height(&self) -> u32 {
    self.height
  }

  /// Return the canonical block hash for this block.
  pub fn block_hash(&self) -> [u8; 32] {
    self.block_hash
  }

  /// Return the byte range of the indexed transaction within the raw block.
  pub fn tx_byte_range(&self, index: usize) -> Option<(usize, usize)> {
    self.tx_byte_ranges.get(index).copied()
  }

  /// Return the precomputed txid for the indexed transaction.
  pub fn txid(&self, index: usize) -> Option<&[u8; 32]> {
    self.txids.get(index)
  }
}
