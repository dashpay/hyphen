//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Address REST API endpoints (summary, txs, utxos, balances).

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::rest::error::ApiError;
use crate::rest::openapi::{
  AddressSummary, AddressTxDirection, AddressTxListResponse, AddressTxSummary, BalanceEntry,
  BalancePageMeta, BalanceResponse, ErrorResponse, SearchAddressResult, SearchResponse, TxTags,
  Utxo, UtxoListResponse,
};
use crate::rest::{
  decode_cursor, encode_cursor, rfc3339_time, AppState, CursorParams, IndexedCursorPage,
};
use crate::utils::{self, HashEndian};

use super::cache::{
  cached_balances_row, cached_balances_summary, cached_mined_tx_from_row, cached_summary_from_row,
  cached_utxo_from_row, AddressSummaryKey, AddressTxPageKey, AddressUtxoPageKey, BalancesPageKey,
  CachedAddressSummary, CachedAddressTx, CachedAddressTxPage, CachedAddressUtxoPage,
  CachedBalancesPage,
};

const MAX_ADDRESS_PAGE_LIMIT: i64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AddressTxCursor {
  as_of_height: i32,
  address: String,
  block_height: i32,
  block_pos: i32,
  txid: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct AddressUtxoCursor {
  as_of_height: i32,
  address: String,
  block_height: i32,
  txid: String,
  vout_index: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct BalancesCursor {
  as_of_height: i32,
  rank: i64,
  balance: i64,
  address: String,
}

/// Get address balance summary.
///
/// # Errors
///
/// Returns `ApiError` when the spent stage is unavailable, the address does
/// not exist, or a backing query fails.
#[utoipa::path(
  get,
  path = "/api/v1/address/{addr}",
  description = "Requires the optional address/spent index. Returns a derived address summary assembled from indexed chain facts rather than a Core RPC passthrough payload.",
  params(("addr" = String, Path, description = "Base58 address")),
  responses(
    (status = 200, description = "Address summary", body = AddressSummary),
    (status = 404, description = "Address not found", body = ErrorResponse),
    (status = 501, description = "Address index not enabled", body = ErrorResponse),
    (status = 503, description = "Address index not ready yet", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Addresses"
)]
pub async fn get_address(
  State(state): State<AppState>,
  Path(addr): Path<String>,
) -> Result<Json<AddressSummary>, ApiError> {
  let stage = state.stage_status_typed(crate::index::Stage::Spent).await?;
  state.check_address_summary_stage(stage)?;
  let row = load_address_summary(&state, stage.height, &addr)
    .await?
    .ok_or_else(|| ApiError::not_found("address not found"))?;
  let first_seen = match row.first_seen_height {
    Some(height) => super::sql::block_time_by_height(&state.pool, height)
      .await
      .map_err(|e| ApiError::from_daemon(e, "query address first-seen block time"))?
      .map(rfc3339_time)
      .transpose()?,
    None => None,
  };
  let last_seen = match row.last_seen_height {
    Some(height) => super::sql::block_time_by_height(&state.pool, height)
      .await
      .map_err(|e| ApiError::from_daemon(e, "query address last-seen block time"))?
      .map(rfc3339_time)
      .transpose()?,
    None => None,
  };

  Ok(Json(AddressSummary {
    as_of_height: stage.height,
    address: addr,
    balance: row.balance,
    total_received: row.total_received,
    total_sent: row.total_sent,
    tx_count: row.tx_count,
    first_seen,
    last_seen,
  }))
}

/// List transactions for an address.
///
/// # Errors
///
/// Returns `ApiError` when the spent stage is unavailable or any live or
/// indexed address transaction query fails.
#[utoipa::path(
  get,
  path = "/api/v1/address/{addr}/txs",
  description = "Requires the optional address/spent index. Returns mined address transaction history from one committed spent-index snapshot, including address-relative movement and block time, and does not mix in live mempool state.",
  params(
    ("addr" = String, Path, description = "Base58 address"),
    CursorParams,
  ),
  responses(
    (status = 200, description = "Address transactions", body = AddressTxListResponse),
    (status = 400, description = "Invalid pagination query", body = ErrorResponse),
    (status = 501, description = "Address index not enabled", body = ErrorResponse),
    (status = 503, description = "Address index not ready yet", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Addresses"
)]
pub async fn get_address_txs(
  State(state): State<AppState>,
  Path(addr): Path<String>,
  Query(params): Query<CursorParams>,
) -> Result<Json<IndexedCursorPage<AddressTxSummary>>, ApiError> {
  let stage = state.stage_status_typed(crate::index::Stage::Spent).await?;
  let params = params.validate(MAX_ADDRESS_PAGE_LIMIT)?;
  let limit = params.limit;
  let cursor = decode_address_tx_cursor(params.cursor.as_deref(), stage.height, &addr)?;
  let as_of_height = cursor
    .as_ref()
    .map_or(stage.height, |cursor| cursor.as_of_height);
  let key = AddressTxPageKey {
    address: addr.clone(),
    spent_height: as_of_height,
    limit,
    cursor: params.cursor.clone(),
  };
  let hot = cached_address_summary(&state, as_of_height, &addr)
    .await?
    .is_none_or(|row| row.tx_count > state.address_hot_cache_threshold);
  let page = if hot {
    state
      .spent_cache
      .tx_page_or_fetch(key, || async {
        load_address_tx_page(&state, &addr, as_of_height, limit, cursor.as_ref()).await
      })
      .await?
  } else {
    std::sync::Arc::new(
      load_address_tx_page(&state, &addr, as_of_height, limit, cursor.as_ref()).await?,
    )
  };
  let value: Vec<AddressTxSummary> = page.txs.iter().map(address_tx_summary).collect();
  let total_count = load_address_summary(&state, as_of_height, &addr)
    .await?
    .map_or(0, |row| i64::from(row.tx_count));
  Ok(Json(IndexedCursorPage {
    as_of_height,
    total_count,
    value,
    next_cursor: page.next_cursor.clone(),
  }))
}

async fn load_address_tx_page(
  state: &AppState,
  addr: &str,
  spent_height: i32,
  limit: i64,
  cursor: Option<&AddressTxCursor>,
) -> Result<CachedAddressTxPage, ApiError> {
  let before = decode_address_tx_cursor_key(cursor)?;
  let stores = state
    .secondary_stores
    .as_ref()
    .ok_or_else(|| ApiError::service_unavailable("address history store unavailable"))?;
  let page = crate::store::address_history::page_rows(
    stores
      .db(crate::store::StoreId::AddressHistory)
      .map_err(|e| ApiError::from_daemon(e, "open address history store"))?,
    addr,
    spent_height,
    (limit + 1) as usize,
    before
      .as_ref()
      .map(|(height, pos, txid)| (*height, *pos, txid.as_slice())),
  )
  .map_err(|e| ApiError::from_daemon(e, "page address history store"))?;
  let page_rows = page
    .iter()
    .map(|row| (row.txid.clone(), row.block_height, row.block_pos))
    .collect::<Vec<_>>();
  let mut rows = super::sql::hydrate_address_history_page(&state.address_pool, addr, &page_rows)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query address txs"))?;
  let has_more = rows.len() as i64 > limit;
  rows.truncate(limit as usize);
  let next_cursor = if has_more {
    rows
      .last()
      .map(|row| {
        encode_cursor(&AddressTxCursor {
          as_of_height: spent_height,
          address: addr.to_owned(),
          block_height: row.block_height,
          block_pos: row.block_pos,
          txid: utils::view_hex(&row.txid, HashEndian::Ser),
        })
      })
      .transpose()?
  } else {
    None
  };
  let txs: Vec<CachedAddressTx> = rows
    .into_iter()
    .map(cached_mined_tx_from_row)
    .collect::<Result<_, _>>()?;

  Ok(CachedAddressTxPage { txs, next_cursor })
}

async fn load_address_summary(
  state: &AppState,
  spent_height: i32,
  addr: &str,
) -> Result<Option<CachedAddressSummary>, ApiError> {
  let key = AddressSummaryKey {
    address: addr.to_owned(),
    spent_height,
  };
  let row = state
    .spent_cache
    .summary_or_fetch(key, || async {
      super::sql::address_summary(&state.address_pool, addr)
        .await
        .map_err(|e| ApiError::from_daemon(e, "query address summary"))
        .and_then(|row| row.ok_or_else(|| ApiError::not_found("address not found")))
        .map(cached_summary_from_row)
    })
    .await?;
  Ok(Some(row.as_ref().clone()))
}

async fn cached_address_summary(
  state: &AppState,
  spent_height: i32,
  addr: &str,
) -> Result<Option<CachedAddressSummary>, ApiError> {
  let key = AddressSummaryKey {
    address: addr.to_owned(),
    spent_height,
  };
  Ok(
    state
      .spent_cache
      .summary(&key)
      .await
      .map(|row| row.as_ref().clone()),
  )
}

fn address_tx_summary(row: &CachedAddressTx) -> AddressTxSummary {
  AddressTxSummary {
    txid: row.txid.clone(),
    block_height: row.block_height,
    block_pos: row.block_pos,
    block_time: row.block_time.clone(),
    version: row.version as i16,
    tx_type: row.tx_type.clone(),
    size: row.size,
    value_out: row.value_out,
    net_value: row.net_value,
    direction: match row.direction {
      AddressTxDirection::In => AddressTxDirection::In,
      AddressTxDirection::Out => AddressTxDirection::Out,
    },
    fee: row.fee,
    tags: TxTags {
      mined: row.mined,
      coinbase: row.mined && row.block_pos == Some(0),
    },
  }
}

pub(crate) async fn search_address(
  state: &AppState,
  addr: &str,
) -> Result<Option<SearchResponse>, ApiError> {
  if !state.indices.address {
    return Ok(None);
  }
  let exists = super::sql::address_exists(&state.address_pool, addr)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query address existence"))?;
  Ok(exists.then(|| {
    SearchResponse::Address(SearchAddressResult {
      address: addr.to_string(),
    })
  }))
}

pub(crate) async fn search_addresses_by_prefix(
  state: &AppState,
  prefix: &str,
  limit: i64,
) -> Result<Vec<SearchResponse>, ApiError> {
  if !state.indices.address {
    return Ok(Vec::new());
  }
  let rows = super::sql::search_addresses_by_prefix(&state.address_pool, prefix, limit)
    .await
    .map_err(|e| ApiError::from_daemon(e, "search address prefix"))?;
  Ok(
    rows
      .into_iter()
      .map(|row| {
        SearchResponse::Address(SearchAddressResult {
          address: row.address,
        })
      })
      .collect(),
  )
}

/// List unspent outputs for an address.
///
/// # Errors
///
/// Returns `ApiError` when the spent stage is unavailable or any backing UTXO
/// query fails.
#[utoipa::path(
  get,
  path = "/api/v1/address/{addr}/utxos",
  description = "Requires the optional address/spent index. Returns spendable outputs from one committed spent-index snapshot.",
  params(("addr" = String, Path, description = "Base58 address"), CursorParams),
  responses(
    (status = 200, description = "Address UTXOs", body = UtxoListResponse),
    (status = 400, description = "Invalid pagination query", body = ErrorResponse),
    (status = 501, description = "Address index not enabled", body = ErrorResponse),
    (status = 503, description = "Address index not ready yet", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Addresses"
)]
pub async fn get_address_utxos(
  State(state): State<AppState>,
  Path(addr): Path<String>,
  Query(params): Query<CursorParams>,
) -> Result<Json<IndexedCursorPage<Utxo>>, ApiError> {
  let stage = state.stage_status_typed(crate::index::Stage::Spent).await?;
  let params = params.validate(MAX_ADDRESS_PAGE_LIMIT)?;
  let limit = params.limit;
  let cursor = decode_address_utxo_cursor(params.cursor.as_deref(), stage.height, &addr)?;
  let as_of_height = cursor
    .as_ref()
    .map_or(stage.height, |cursor| cursor.as_of_height);

  let key = AddressUtxoPageKey {
    address: addr.clone(),
    spent_height: as_of_height,
    limit,
    cursor: params.cursor.clone(),
  };
  let hot = cached_address_summary(&state, as_of_height, &addr)
    .await?
    .is_none_or(|row| row.tx_count > state.address_hot_cache_threshold);
  let page = if hot {
    state
      .spent_cache
      .utxo_page_or_fetch(key, || async {
        load_address_utxo_page(&state, as_of_height, &addr, limit, cursor.as_ref()).await
      })
      .await?
  } else {
    std::sync::Arc::new(
      load_address_utxo_page(&state, as_of_height, &addr, limit, cursor.as_ref()).await?,
    )
  };

  let value: Vec<Utxo> = page
    .utxos
    .iter()
    .map(|row| Utxo {
      txid: row.txid.clone(),
      vout: row.vout_index,
      value: row.value,
      height: row.block_height,
      script_pub_key: row.script_pubkey.clone(),
    })
    .collect();
  let total_count = load_address_summary(&state, as_of_height, &addr)
    .await?
    .map_or(0, |row| i64::from(row.utxo_count));
  Ok(Json(IndexedCursorPage {
    as_of_height,
    total_count,
    value,
    next_cursor: page.next_cursor.clone(),
  }))
}

async fn load_address_utxo_page(
  state: &AppState,
  spent_height: i32,
  addr: &str,
  limit: i64,
  cursor: Option<&AddressUtxoCursor>,
) -> Result<CachedAddressUtxoPage, ApiError> {
  let before = decode_address_utxo_cursor_key(cursor)?;
  let before_ref = before
    .as_ref()
    .map(|(height, txid, vout_index)| (*height, txid.as_slice(), *vout_index));
  let mut rows = super::sql::address_utxos(&state.address_pool, addr, limit + 1, before_ref)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query address utxos"))?;
  let has_more = rows.len() as i64 > limit;
  rows.truncate(limit as usize);
  let next_cursor = if has_more {
    rows
      .last()
      .map(|row| {
        encode_cursor(&AddressUtxoCursor {
          as_of_height: spent_height,
          address: addr.to_owned(),
          block_height: row.block_height,
          txid: utils::view_hex(&row.txid, HashEndian::Ser),
          vout_index: row.vout_index,
        })
      })
      .transpose()?
  } else {
    None
  };

  Ok(CachedAddressUtxoPage {
    utxos: rows.into_iter().map(cached_utxo_from_row).collect(),
    next_cursor,
  })
}

/// Balances -- addresses ranked by balance.
///
/// # Errors
///
/// Returns `ApiError` when the spent stage is unavailable or any backing
/// balances query fails.
#[utoipa::path(
  get,
  path = "/api/v1/balances",
  description = "Requires the optional address/spent index. This is a server-side ranked balance view, not a Core RPC concept. `includedSupply` is derived from the same spent-ledger snapshot as the returned balances and should be used for concentration calculations instead of combining this endpoint with `/api/v1/metrics/latest`.",
  params(CursorParams),
  responses(
    (status = 200, description = "Balances", body = BalanceResponse),
    (status = 400, description = "Invalid pagination query", body = ErrorResponse),
    (status = 501, description = "Address index not enabled", body = ErrorResponse),
    (status = 503, description = "Address index not ready yet", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Addresses"
)]
pub async fn get_balances(
  State(state): State<AppState>,
  Query(params): Query<CursorParams>,
) -> Result<Json<BalanceResponse>, ApiError> {
  let stage = state.stage_status_typed(crate::index::Stage::Spent).await?;
  let params = params.validate(MAX_ADDRESS_PAGE_LIMIT)?;
  let limit = params.limit;
  let cursor = decode_balances_cursor(params.cursor.as_deref(), stage.height)?;
  let as_of_height = cursor
    .as_ref()
    .map_or(stage.height, |cursor| cursor.as_of_height);
  let key = BalancesPageKey {
    spent_height: as_of_height,
    limit,
    cursor: params.cursor.clone(),
  };
  let page = state
    .spent_cache
    .balances_page_or_fetch(key, || async {
      load_balances_page(&state, as_of_height, limit, cursor.as_ref()).await
    })
    .await?;

  let value: Vec<BalanceEntry> = page
    .addresses
    .iter()
    .enumerate()
    .map(|(index, row)| BalanceEntry {
      rank: page.start_rank + index as i64 + 1,
      address: row.address.clone(),
      balance: row.balance,
      pct_supply: pct_supply(row.balance, page.included_supply),
      total_received: row.total_received,
      total_sent: row.total_sent,
      tx_count: row.tx_count,
    })
    .collect();
  let total_count = state
    .spent_cache
    .balances_summary_or_fetch(as_of_height, || async {
      super::sql::balances_summary(&state.pool)
        .await
        .map(cached_balances_summary)
        .map_err(|e| ApiError::from_daemon(e, "query balances summary"))
    })
    .await?
    .total;
  Ok(Json(BalanceResponse {
    as_of_height,
    total_count,
    value,
    meta: BalancePageMeta {
      included_supply: page.included_supply,
      supply_definition: "sum of positive balances included by this balances snapshot".to_string(),
    },
    next_cursor: page.next_cursor.clone(),
  }))
}

fn pct_supply(balance: i64, included_supply: i64) -> f64 {
  if included_supply <= 0 {
    return 0.0;
  }
  balance as f64 / included_supply as f64
}

async fn load_balances_page(
  state: &AppState,
  spent_height: i32,
  limit: i64,
  cursor: Option<&BalancesCursor>,
) -> Result<CachedBalancesPage, ApiError> {
  let mut rows = super::sql::balances_page(
    &state.pool,
    limit + 1,
    cursor
      .as_ref()
      .map(|cursor| (cursor.balance, cursor.address.as_str())),
  )
  .await
  .map_err(|e| ApiError::from_daemon(e, "query balances"))?;
  let has_more = rows.len() as i64 > limit;
  rows.truncate(limit as usize);
  let summary = state
    .spent_cache
    .balances_summary_or_fetch(spent_height, || async {
      super::sql::balances_summary(&state.pool)
        .await
        .map(cached_balances_summary)
        .map_err(|e| ApiError::from_daemon(e, "query balances summary"))
    })
    .await?;
  let next_cursor = if has_more {
    rows
      .last()
      .map(|row| {
        encode_cursor(&BalancesCursor {
          as_of_height: spent_height,
          rank: cursor.map_or(0, |cursor| cursor.rank) + rows.len() as i64,
          balance: row.balance,
          address: row.address.clone(),
        })
      })
      .transpose()?
  } else {
    None
  };

  Ok(CachedBalancesPage {
    addresses: rows.into_iter().map(cached_balances_row).collect(),
    included_supply: summary.included_supply,
    start_rank: cursor.map_or(0, |cursor| cursor.rank),
    next_cursor,
  })
}

fn decode_address_tx_cursor(
  cursor: Option<&str>,
  current_height: i32,
  address: &str,
) -> Result<Option<AddressTxCursor>, ApiError> {
  let Some(cursor) = decode_cursor::<AddressTxCursor>(cursor)? else {
    return Ok(None);
  };
  if cursor.as_of_height > current_height || cursor.address != address {
    return Err(ApiError::bad_request(
      "cursor does not match current address request",
    ));
  }
  Ok(Some(cursor))
}

fn decode_address_tx_cursor_key(
  cursor: Option<&AddressTxCursor>,
) -> Result<Option<(i32, i32, Vec<u8>)>, ApiError> {
  cursor
    .map(|cursor| {
      utils::decode_hex_to(&cursor.txid, HashEndian::Ser)
        .map(|txid| (cursor.block_height, cursor.block_pos, txid))
        .map_err(|_| ApiError::bad_request("invalid cursor txid"))
    })
    .transpose()
}

fn decode_address_utxo_cursor(
  cursor: Option<&str>,
  current_height: i32,
  address: &str,
) -> Result<Option<AddressUtxoCursor>, ApiError> {
  let Some(cursor) = decode_cursor::<AddressUtxoCursor>(cursor)? else {
    return Ok(None);
  };
  if cursor.as_of_height > current_height || cursor.address != address {
    return Err(ApiError::bad_request(
      "cursor does not match current address request",
    ));
  }
  Ok(Some(cursor))
}

fn decode_address_utxo_cursor_key(
  cursor: Option<&AddressUtxoCursor>,
) -> Result<Option<(i32, Vec<u8>, i32)>, ApiError> {
  cursor
    .map(|cursor| {
      utils::decode_hex_to(&cursor.txid, HashEndian::Ser)
        .map(|txid| (cursor.block_height, txid, cursor.vout_index))
        .map_err(|_| ApiError::bad_request("invalid cursor txid"))
    })
    .transpose()
}

fn decode_balances_cursor(
  cursor: Option<&str>,
  current_height: i32,
) -> Result<Option<BalancesCursor>, ApiError> {
  let Some(cursor) = decode_cursor::<BalancesCursor>(cursor)? else {
    return Ok(None);
  };
  if cursor.as_of_height > current_height {
    return Err(ApiError::bad_request(
      "cursor snapshot is ahead of current request",
    ));
  }
  Ok(Some(cursor))
}

#[cfg(test)]
mod tests {
  use crate::rest::openapi::AddressTxDirection;
  use crate::utils::HashEndian;
  use std::sync::Arc;
  use std::time::{SystemTime, UNIX_EPOCH};

  #[test]
  fn address_tx_summary_keeps_address_relative_fields() {
    let row = super::CachedAddressTx {
      txid: "11".repeat(32),
      block_height: Some(7),
      block_pos: Some(0),
      block_time: Some("1970-01-01T00:00:07Z".to_owned()),
      version: 3,
      tx_type: "normal".to_owned(),
      size: 200,
      value_out: 90,
      net_value: -55,
      direction: AddressTxDirection::Out,
      fee: Some(3),
      mined: true,
      islock: false,
    };

    let summary = super::address_tx_summary(&row);
    assert_eq!(summary.block_time.as_deref(), Some("1970-01-01T00:00:07Z"));
    assert_eq!(summary.net_value, -55);
    assert!(matches!(summary.direction, AddressTxDirection::Out));
  }

  #[tokio::test]
  async fn load_address_tx_page_matches_postgres_when_store_is_present(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let address = "XstorePage11111111111111111111111111";
    let txid_a = vec![0x11; 32];
    let txid_b = vec![0x22; 32];
    let txid_c = vec![0x33; 32];
    crate::index::spent::sql::seed_address_history_tx_for_rest_tests(
      &pool, address, 1, 0, &txid_a, 50, 1, 50, 0,
    )
    .await?;
    crate::index::spent::sql::seed_address_history_tx_for_rest_tests(
      &pool, address, 2, 0, &txid_b, 60, 1, 0, 60,
    )
    .await?;
    crate::index::spent::sql::seed_address_history_tx_for_rest_tests(
      &pool, address, 3, 0, &txid_c, 70, 1, 70, 0,
    )
    .await?;

    let unique = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
    let datadir = crate::tests::temp_datadir(&format!("hyphen-address-store-page-{unique}"));
    let secondary_stores = Arc::new(crate::store::manager::SecondaryStoreSet::open(
      &datadir,
      &crate::config::ChainConfig::default(),
      None,
    )?);
    crate::store::address_history::insert_rows(
      secondary_stores.db(crate::store::StoreId::AddressHistory)?,
      &[address.to_owned(), address.to_owned(), address.to_owned()],
      &[txid_a.clone(), txid_b.clone(), txid_c.clone()],
      &[1, 2, 3],
      &[0, 0, 0],
      3,
      [0u8; 32],
    )?;

    let mut state = crate::tests::app_state();
    state.pool = pool.clone();
    state.address_pool = pool.clone();
    state.secondary_stores = Some(Arc::clone(&secondary_stores));

    let store_page = super::load_address_tx_page(&state, address, 3, 3, None)
      .await
      .map_err(|e| std::io::Error::other(e.message.clone()))?;

    let pg_rows = crate::index::spent::sql::address_history_txs(&pool, address, 3, None).await?;
    let pg_page = super::CachedAddressTxPage {
      txs: pg_rows
        .into_iter()
        .map(crate::index::spent::cache::cached_mined_tx_from_row)
        .collect::<Result<_, _>>()
        .map_err(|e| std::io::Error::other(e.message.clone()))?,
      next_cursor: None,
    };

    assert_eq!(store_page.txs, pg_page.txs);
    assert_eq!(store_page.next_cursor, pg_page.next_cursor);
    std::fs::remove_dir_all(datadir)?;
    Ok(())
  }

  #[tokio::test]
  async fn load_address_tx_page_rejects_store_rows_missing_postgres_hydration(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let address = "XstoreMismatch1111111111111111111111";
    let txid = vec![0x44; 32];
    let datadir = crate::tests::temp_datadir("hyphen-address-store-mismatch");
    let secondary_stores = Arc::new(crate::store::manager::SecondaryStoreSet::open(
      &datadir,
      &crate::config::ChainConfig::default(),
      None,
    )?);
    crate::store::address_history::insert_rows(
      secondary_stores.db(crate::store::StoreId::AddressHistory)?,
      &[address.to_owned()],
      std::slice::from_ref(&txid),
      &[3],
      &[0],
      3,
      [0u8; 32],
    )?;

    let mut state = crate::tests::app_state();
    state.pool = pool.clone();
    state.address_pool = pool;
    state.secondary_stores = Some(secondary_stores);

    let error = super::load_address_tx_page(&state, address, 3, 10, None)
      .await
      .expect_err("missing PostgreSQL hydration should fail closed");
    assert!(
      error.message.contains("query address txs"),
      "unexpected error: {:?}",
      error
    );
    std::fs::remove_dir_all(datadir)?;
    Ok(())
  }

  #[tokio::test]
  async fn load_address_tx_page_ignores_store_rows_ahead_of_spent_height(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let address = "XstoreHeightCap111111111111111111111";
    let txid3 = vec![0x53; 32];
    let txid5 = vec![0x55; 32];
    crate::index::spent::sql::seed_address_history_tx_for_rest_tests(
      &pool, address, 3, 0, &txid3, 50, 1, 50, 0,
    )
    .await?;
    crate::index::spent::sql::seed_address_history_tx_for_rest_tests(
      &pool, address, 5, 0, &txid5, 60, 1, 60, 0,
    )
    .await?;

    let datadir = crate::tests::temp_datadir("hyphen-address-store-height-cap");
    let secondary_stores = Arc::new(crate::store::manager::SecondaryStoreSet::open(
      &datadir,
      &crate::config::ChainConfig::default(),
      None,
    )?);
    crate::store::address_history::insert_rows(
      secondary_stores.db(crate::store::StoreId::AddressHistory)?,
      &[address.to_owned(), address.to_owned()],
      &[txid3.clone(), txid5.clone()],
      &[3, 5],
      &[0, 0],
      5,
      [0u8; 32],
    )?;

    let mut state = crate::tests::app_state();
    state.pool = pool.clone();
    state.address_pool = pool;
    state.secondary_stores = Some(secondary_stores);

    let page = super::load_address_tx_page(&state, address, 3, 10, None)
      .await
      .map_err(|e| std::io::Error::other(e.message.clone()))?;
    assert_eq!(page.txs.len(), 1);
    assert_eq!(
      page.txs[0].txid,
      crate::utils::view_hex(&txid3, HashEndian::Ser)
    );
    std::fs::remove_dir_all(datadir)?;
    Ok(())
  }
}
