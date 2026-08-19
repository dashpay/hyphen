//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! In-memory `HyphenDb` fake for handler tests.
//!
//! Assembled through [`hyphen_db::HyphenDb::from_parts`], it lets REST handler
//! tests inject seeded chain-read data without a live Postgres. Unseeded
//! domains return empty pages / `None`; the accumulation-heavy reads that no
//! current test exercises return [`HyDbError::MockGap`] so gaps surface loudly
//! rather than as silently-wrong data.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use hyphen_db::query::{
  HyDbAddrIdx, HyDbBlockIdx, HyDbMetricIdx, HyDbQrAddrSummary, HyDbQrAddrTx, HyDbQrBlockDetail,
  HyDbQrBlockRef, HyDbQrBlockSummary, HyDbQrBlockTxRef, HyDbQrChainTip, HyDbQrHealth,
  HyDbQrMetricPoint, HyDbQrRichEntry, HyDbQrSearchHit, HyDbQrSizeDiag, HyDbQrSupply,
  HyDbQrTxDetail, HyDbQrTxSummary, HyDbQrUtxo, HyDbQrValidator, HyDbQrVlCounts, HyDbQrVlHistory,
  HyDbSearch, HyDbStatus, HyDbTxIdx, HyDbVlIdx,
};
use hyphen_db::{
  HyDbAddress, HyDbCursor, HyDbError, HyDbHash, HyDbHeight, HyDbPage, HyDbResult, HyDbVlType,
  HyphenDb,
};

/// Seedable in-memory chain-read store implementing every domain trait.
#[derive(Debug, Default)]
pub(crate) struct MemDb {
  blocks: Vec<HyDbQrBlockDetail>,
  raw_blocks: HashMap<[u8; 32], Vec<u8>>,
  txs: HashMap<[u8; 32], HyDbQrTxDetail>,
  tip: Option<HyDbQrChainTip>,
}

impl MemDb {
  pub(crate) fn new() -> Self {
    Self::default()
  }

  /// Seed one canonical block's detail (also serves height/hash lookups).
  pub(crate) fn with_block(mut self, block: HyDbQrBlockDetail) -> Self {
    self.blocks.push(block);
    self
  }

  /// Seed raw bytes for a block hash (serves `raw_bytes`).
  pub(crate) fn with_raw_block(mut self, hash: [u8; 32], raw: Vec<u8>) -> Self {
    self.raw_blocks.insert(hash, raw);
    self
  }

  /// Seed one transaction's full detail.
  pub(crate) fn with_tx(mut self, tx: HyDbQrTxDetail) -> Self {
    self.txs.insert(*tx.txid.as_bytes(), tx);
    self
  }

  /// Seed an explicit canonical tip, marking every stage as ready.
  pub(crate) fn with_tip(mut self, height: u32, hash: [u8; 32]) -> Self {
    self.tip = Some(HyDbQrChainTip {
      height: HyDbHeight::new(height),
      hash: HyDbHash(hash),
    });
    self
  }

  /// Assemble a `HyphenDb` facade backed entirely by this fake.
  pub(crate) fn into_hydb(self) -> HyphenDb {
    let db = Arc::new(self);
    HyphenDb::from_parts(
      db.clone(),
      db.clone(),
      db.clone(),
      db.clone(),
      db.clone(),
      db.clone(),
      db,
    )
  }

  fn block_by_height(&self, height: u32) -> Option<&HyDbQrBlockDetail> {
    self
      .blocks
      .iter()
      .find(|block| block.height.get() == height)
  }

  fn block_by_hash(&self, hash: &HyDbHash) -> Option<&HyDbQrBlockDetail> {
    self.blocks.iter().find(|block| &block.hash == hash)
  }
}

#[async_trait]
impl HyDbBlockIdx for MemDb {
  async fn list(
    &self,
    _limit: u32,
    _before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrBlockSummary>> {
    Ok(HyDbPage::last(Vec::new()))
  }

  async fn by_height(&self, height: HyDbHeight) -> HyDbResult<Option<HyDbQrBlockDetail>> {
    Ok(self.block_by_height(height.get()).cloned())
  }

  async fn by_hash(&self, hash: HyDbHash) -> HyDbResult<Option<HyDbQrBlockDetail>> {
    Ok(self.block_by_hash(&hash).cloned())
  }

  async fn txs(
    &self,
    _height: HyDbHeight,
    _limit: u32,
    _after: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrBlockTxRef>> {
    Ok(HyDbPage::last(Vec::new()))
  }

  async fn raw_bytes(&self, hash: HyDbHash) -> HyDbResult<Option<Vec<u8>>> {
    Ok(self.raw_blocks.get(hash.as_bytes()).cloned())
  }

  async fn topology(&self, hash: HyDbHash) -> HyDbResult<Option<HyDbQrBlockRef>> {
    Ok(self.block_by_hash(&hash).map(|block| HyDbQrBlockRef {
      hash: block.hash,
      prev_hash: block.prev_hash,
      height: block.height,
      canonical: true,
    }))
  }

  async fn search_prefix(&self, _prefix: String, _limit: u32) -> HyDbResult<Vec<HyDbQrBlockRef>> {
    Ok(Vec::new())
  }

  async fn exists(&self, height: HyDbHeight) -> HyDbResult<bool> {
    Ok(self.block_by_height(height.get()).is_some())
  }

  async fn height_for_hash(&self, hash: HyDbHash) -> HyDbResult<Option<HyDbHeight>> {
    Ok(self.block_by_hash(&hash).map(|block| block.height))
  }

  async fn hash_for_height(&self, height: HyDbHeight) -> HyDbResult<Option<HyDbHash>> {
    Ok(self.block_by_height(height.get()).map(|block| block.hash))
  }
}

#[async_trait]
impl HyDbTxIdx for MemDb {
  async fn get(&self, txid: HyDbHash) -> HyDbResult<Option<HyDbQrTxDetail>> {
    Ok(self.txs.get(txid.as_bytes()).cloned())
  }

  async fn list(
    &self,
    _limit: u32,
    _before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrTxSummary>> {
    Ok(HyDbPage::last(Vec::new()))
  }

  async fn total(&self) -> HyDbResult<u64> {
    Ok(self.txs.len() as u64)
  }

  async fn search_prefix(&self, _prefix: String, _limit: u32) -> HyDbResult<Vec<HyDbHash>> {
    Ok(Vec::new())
  }
}

#[async_trait]
impl HyDbAddrIdx for MemDb {
  async fn summary(&self, _address: HyDbAddress) -> HyDbResult<Option<HyDbQrAddrSummary>> {
    Ok(None)
  }

  async fn exists(&self, _address: HyDbAddress) -> HyDbResult<bool> {
    Ok(false)
  }

  async fn history(
    &self,
    _address: HyDbAddress,
    _limit: u32,
    _before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrAddrTx>> {
    Ok(HyDbPage::last(Vec::new()))
  }

  async fn utxos(
    &self,
    _address: HyDbAddress,
    _limit: u32,
    _before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrUtxo>> {
    Ok(HyDbPage::last(Vec::new()))
  }

  async fn rich_list(
    &self,
    _limit: u32,
    _before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrRichEntry>> {
    Ok(HyDbPage::last(Vec::new()))
  }

  async fn bal_count(&self) -> HyDbResult<u64> {
    Ok(0)
  }

  async fn supply(&self) -> HyDbResult<HyDbQrSupply> {
    Err(HyDbError::MockGap("addr.supply"))
  }

  async fn search_prefix(&self, _prefix: String, _limit: u32) -> HyDbResult<Vec<HyDbAddress>> {
    Ok(Vec::new())
  }
}

#[async_trait]
impl HyDbVlIdx for MemDb {
  async fn list(
    &self,
    _status: Option<String>,
    _validator_type: Option<HyDbVlType>,
    _limit: u32,
    _before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrValidator>> {
    Ok(HyDbPage::last(Vec::new()))
  }

  async fn get(&self, _protx_hash: HyDbHash) -> HyDbResult<Option<HyDbQrValidator>> {
    Ok(None)
  }

  async fn history(&self, _protx_hash: HyDbHash) -> HyDbResult<Vec<HyDbQrVlHistory>> {
    Ok(Vec::new())
  }

  async fn counts(&self) -> HyDbResult<Vec<HyDbQrVlCounts>> {
    Ok(Vec::new())
  }

  async fn search_prefix(&self, _prefix: String, _limit: u32) -> HyDbResult<Vec<HyDbHash>> {
    Ok(Vec::new())
  }
}

#[async_trait]
impl HyDbMetricIdx for MemDb {
  async fn latest(&self) -> HyDbResult<Option<HyDbQrMetricPoint>> {
    Ok(None)
  }

  async fn range(
    &self,
    _from: HyDbHeight,
    _to: HyDbHeight,
    _step: u32,
  ) -> HyDbResult<Vec<HyDbQrMetricPoint>> {
    Ok(Vec::new())
  }
}

#[async_trait]
impl HyDbSearch for MemDb {
  async fn prefix(&self, _query: String, _limit: u32) -> HyDbResult<Vec<HyDbQrSearchHit>> {
    Ok(Vec::new())
  }
}

#[async_trait]
impl HyDbStatus for MemDb {
  async fn health(&self) -> HyDbResult<HyDbQrHealth> {
    Err(HyDbError::MockGap("status.health"))
  }

  async fn tip(&self) -> HyDbResult<Option<HyDbQrChainTip>> {
    if let Some(tip) = &self.tip {
      return Ok(Some(tip.clone()));
    }
    Ok(
      self
        .blocks
        .iter()
        .max_by_key(|block| block.height.get())
        .map(|block| HyDbQrChainTip {
          height: block.height,
          hash: block.hash,
        }),
    )
  }

  async fn sizes(&self) -> HyDbResult<Vec<HyDbQrSizeDiag>> {
    Ok(Vec::new())
  }
}
