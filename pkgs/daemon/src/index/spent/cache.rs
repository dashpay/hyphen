//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Address query caches owned by the spent index.

use std::sync::Arc;

use crate::cache::WeightedLru;

use super::sql::{
  AddressSummaryRow, AddressTxRow, AddressUtxoRow, BalancesRow, BalancesSummaryRow,
};

/// One cached address summary response model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedAddressSummary {
  pub balance: i64,
  pub total_received: i64,
  pub total_sent: i64,
  pub tx_count: i32,
  pub utxo_count: i32,
  pub first_seen_height: Option<i32>,
  pub last_seen_height: Option<i32>,
}

/// One cached address transaction summary row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedAddressTx {
  pub txid: String,
  pub block_height: Option<i32>,
  pub block_pos: Option<i32>,
  pub block_time: Option<String>,
  pub version: i32,
  pub tx_type: String,
  pub size: i32,
  pub value_out: i64,
  pub net_value: i64,
  pub direction: crate::rest::openapi::AddressTxDirection,
  pub fee: Option<i64>,
  pub mined: bool,
  pub islock: bool,
}

/// One cached address transactions page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedAddressTxPage {
  pub txs: Vec<CachedAddressTx>,
  pub next_cursor: Option<String>,
}

/// One cached address UTXO row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedAddressUtxo {
  pub txid: String,
  pub vout_index: i32,
  pub value: i64,
  pub block_height: i32,
  pub script_pubkey: String,
}

/// One cached address UTXO page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedAddressUtxoPage {
  pub utxos: Vec<CachedAddressUtxo>,
  pub next_cursor: Option<String>,
}

/// One cached balances row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedBalancesEntry {
  pub address: String,
  pub balance: i64,
  pub total_received: i64,
  pub total_sent: i64,
  pub tx_count: i32,
}

/// One cached balances page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedBalancesPage {
  pub addresses: Vec<CachedBalancesEntry>,
  pub included_supply: i64,
  pub start_rank: i64,
  pub next_cursor: Option<String>,
}

/// Exact balances summary for one spent height.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CachedBalancesSummary {
  pub total: i64,
  pub included_supply: i64,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AddressSummaryKey {
  pub address: String,
  pub spent_height: i32,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AddressTxPageKey {
  pub address: String,
  pub spent_height: i32,
  pub limit: i64,
  pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct AddressUtxoPageKey {
  pub address: String,
  pub spent_height: i32,
  pub limit: i64,
  pub cursor: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BalancesPageKey {
  pub spent_height: i32,
  pub limit: i64,
  pub cursor: Option<String>,
}

/// Shared spent-index query caches.
pub struct SpentQueryCache {
  summaries: Arc<WeightedLru<AddressSummaryKey, CachedAddressSummary>>,
  tx_pages: Arc<WeightedLru<AddressTxPageKey, CachedAddressTxPage>>,
  utxo_pages: Arc<WeightedLru<AddressUtxoPageKey, CachedAddressUtxoPage>>,
  balances_pages: Arc<WeightedLru<BalancesPageKey, CachedBalancesPage>>,
  balances_summaries: Arc<WeightedLru<i32, CachedBalancesSummary>>,
}

impl std::fmt::Debug for SpentQueryCache {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("SpentQueryCache").finish()
  }
}

impl SpentQueryCache {
  /// Build one disabled spent cache for tests and bypass mode.
  pub fn disabled() -> Self {
    Self {
      summaries: Arc::new(WeightedLru::new(false, 1)),
      tx_pages: Arc::new(WeightedLru::new(false, 1)),
      utxo_pages: Arc::new(WeightedLru::new(false, 1)),
      balances_pages: Arc::new(WeightedLru::new(false, 1)),
      balances_summaries: Arc::new(WeightedLru::new(false, 1)),
    }
  }

  /// Build one spent cache with a shared byte budget for address reads.
  pub fn new(enabled: bool, budget_bytes: usize) -> Self {
    Self {
      summaries: Arc::new(WeightedLru::new(enabled, budget_bytes)),
      tx_pages: Arc::new(WeightedLru::new(enabled, budget_bytes)),
      utxo_pages: Arc::new(WeightedLru::new(enabled, budget_bytes)),
      balances_pages: Arc::new(WeightedLru::new(enabled, budget_bytes)),
      balances_summaries: Arc::new(WeightedLru::new(enabled, budget_bytes)),
    }
  }

  /// Compatibility no-op for callers invalidating live-sensitive address pages.
  pub fn bump_live_generation(&self) {}

  /// Compatibility no-op for callers invalidating spent-stage address pages.
  pub fn reset_stage_generation(&self) {}

  /// Read or fetch one address transactions page.
  pub async fn summary(&self, key: &AddressSummaryKey) -> Option<Arc<CachedAddressSummary>> {
    self.summaries.get(key).await
  }

  /// Read or fetch one address summary.
  pub async fn summary_or_fetch<E, F, Fut>(
    &self,
    key: AddressSummaryKey,
    fetch: F,
  ) -> Result<Arc<CachedAddressSummary>, E>
  where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CachedAddressSummary, E>>,
  {
    self
      .summaries
      .get_or_fetch(key, fetch, summary_weight)
      .await
  }

  /// Read or fetch one address transactions page.
  pub async fn tx_page_or_fetch<E, F, Fut>(
    &self,
    key: AddressTxPageKey,
    fetch: F,
  ) -> Result<Arc<CachedAddressTxPage>, E>
  where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CachedAddressTxPage, E>>,
  {
    self.tx_pages.get_or_fetch(key, fetch, tx_page_weight).await
  }

  /// Read or fetch one address UTXO page.
  pub async fn utxo_page_or_fetch<E, F, Fut>(
    &self,
    key: AddressUtxoPageKey,
    fetch: F,
  ) -> Result<Arc<CachedAddressUtxoPage>, E>
  where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CachedAddressUtxoPage, E>>,
  {
    self
      .utxo_pages
      .get_or_fetch(key, fetch, utxo_page_weight)
      .await
  }

  /// Read or fetch one balances page.
  pub async fn balances_page_or_fetch<E, F, Fut>(
    &self,
    key: BalancesPageKey,
    fetch: F,
  ) -> Result<Arc<CachedBalancesPage>, E>
  where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CachedBalancesPage, E>>,
  {
    self
      .balances_pages
      .get_or_fetch(key, fetch, balances_page_weight)
      .await
  }

  /// Read or fetch one exact balances summary for the spent stage height.
  pub async fn balances_summary_or_fetch<E, F, Fut>(
    &self,
    spent_height: i32,
    fetch: F,
  ) -> Result<Arc<CachedBalancesSummary>, E>
  where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Result<CachedBalancesSummary, E>>,
  {
    self
      .balances_summaries
      .get_or_fetch(spent_height, fetch, balances_summary_weight)
      .await
  }
}

pub fn cached_summary_from_row(row: AddressSummaryRow) -> CachedAddressSummary {
  CachedAddressSummary {
    balance: row.balance,
    total_received: row.total_received,
    total_sent: row.total_sent,
    tx_count: row.tx_count,
    utxo_count: row.utxo_count,
    first_seen_height: row.first_seen_height,
    last_seen_height: row.last_seen_height,
  }
}

pub fn cached_mined_tx_from_row(
  row: AddressTxRow,
) -> Result<CachedAddressTx, crate::rest::error::ApiError> {
  Ok(CachedAddressTx {
    txid: crate::utils::view_hex(&row.txid, crate::utils::HashEndian::Ser),
    block_height: Some(row.block_height),
    block_pos: Some(row.block_pos),
    block_time: Some(crate::rest::rfc3339_time(row.block_time)?),
    version: row.version,
    tx_type: row.tx_type.to_string(),
    size: row.size,
    value_out: row.value_out,
    net_value: row.net_value,
    direction: row.direction,
    fee: row.fee,
    mined: true,
    islock: false,
  })
}

pub fn cached_utxo_from_row(row: AddressUtxoRow) -> CachedAddressUtxo {
  CachedAddressUtxo {
    txid: crate::utils::view_hex(&row.txid, crate::utils::HashEndian::Ser),
    vout_index: row.vout_index,
    value: row.value,
    block_height: row.block_height,
    script_pubkey: hex::encode(row.script_pubkey),
  }
}

pub fn cached_balances_row(row: BalancesRow) -> CachedBalancesEntry {
  CachedBalancesEntry {
    address: row.address,
    balance: row.balance,
    total_received: row.total_received,
    total_sent: row.total_sent,
    tx_count: row.tx_count,
  }
}

pub fn cached_balances_summary(row: BalancesSummaryRow) -> CachedBalancesSummary {
  CachedBalancesSummary {
    total: row.total,
    included_supply: row.included_supply,
  }
}

fn summary_weight(value: &CachedAddressSummary) -> usize {
  64 + std::mem::size_of_val(value)
    + value.first_seen_height.map_or(0, |_| 8)
    + value.last_seen_height.map_or(0, |_| 8)
}

fn tx_page_weight(value: &CachedAddressTxPage) -> usize {
  std::mem::size_of::<CachedAddressTxPage>()
    + value
      .txs
      .iter()
      .map(|tx| {
        std::mem::size_of::<CachedAddressTx>()
          + tx.txid.len()
          + tx.tx_type.len()
          + tx.block_time.as_ref().map_or(0, String::len)
      })
      .sum::<usize>()
}

fn utxo_page_weight(value: &CachedAddressUtxoPage) -> usize {
  std::mem::size_of::<CachedAddressUtxoPage>()
    + value
      .utxos
      .iter()
      .map(|utxo| {
        std::mem::size_of::<CachedAddressUtxo>() + utxo.txid.len() + utxo.script_pubkey.len()
      })
      .sum::<usize>()
}

fn balances_page_weight(value: &CachedBalancesPage) -> usize {
  std::mem::size_of::<CachedBalancesPage>()
    + value
      .addresses
      .iter()
      .map(|row| std::mem::size_of::<CachedBalancesEntry>() + row.address.len())
      .sum::<usize>()
}

fn balances_summary_weight(_: &CachedBalancesSummary) -> usize {
  std::mem::size_of::<CachedBalancesSummary>()
}
