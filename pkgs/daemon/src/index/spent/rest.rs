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
use crate::rest::hydb::{hydb_err, hydb_hash, tx_type_str};
use crate::rest::openapi::{
  AddressSummary, AddressTxDirection, AddressTxListResponse, AddressTxSummary, BalanceEntry,
  BalancePageMeta, BalanceResponse, ErrorResponse, SearchAddressResult, SearchResponse, TxTags,
  Utxo, UtxoListResponse,
};
use crate::rest::{
  decode_cursor, encode_cursor, rfc3339_time, AppState, CursorParams, IndexedCursorPage,
};
use crate::utils::{self, HashEndian};

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

/// Resolve a block's timestamp to an RFC 3339 string, if the block exists.
async fn block_time_rfc3339(
  state: &AppState,
  height: hyphen_db::HyDbHeight,
) -> Result<Option<String>, ApiError> {
  let time = state
    .hydb()?
    .blocks()
    .by_height(height)
    .await
    .map_err(|e| hydb_err(e, "address block time"))?
    .map(|block| block.time);
  time.map(|time| rfc3339_time(i64::from(time))).transpose()
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
  let row = state
    .hydb()?
    .addrs()
    .summary(hyphen_db::HyDbAddress(addr.clone()))
    .await
    .map_err(|e| hydb_err(e, "address summary"))?
    .ok_or_else(|| ApiError::not_found("address not found"))?;
  let first_seen = match row.first_seen_height {
    Some(height) => block_time_rfc3339(&state, height).await?,
    None => None,
  };
  let last_seen = match row.last_seen_height {
    Some(height) => block_time_rfc3339(&state, height).await?,
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
/// Returns `ApiError` when the spent stage is unavailable or any indexed
/// address transaction query fails.
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
  let before = decode_address_tx_cursor_key(cursor.as_ref())?;
  let before_cursor = before.as_ref().map(|(height, pos, txid)| {
    hyphen_db::HyDbCursor(format!("{height}:{pos}:{}", hex::encode(txid)))
  });

  let addrs = state.hydb()?.addrs();
  let page = addrs
    .history(
      hyphen_db::HyDbAddress(addr.clone()),
      limit as u32,
      before_cursor,
    )
    .await
    .map_err(|e| hydb_err(e, "address txs"))?;
  let next_cursor = if page.next_cursor.is_some() {
    page
      .items
      .last()
      .map(|row| {
        encode_cursor(&AddressTxCursor {
          as_of_height,
          address: addr.clone(),
          block_height: row.block_height.get() as i32,
          block_pos: row.block_pos,
          txid: utils::view_hex(row.txid.as_bytes(), HashEndian::Ser),
        })
      })
      .transpose()?
  } else {
    None
  };
  let value: Vec<AddressTxSummary> = page
    .items
    .iter()
    .map(hydb_addr_tx_summary)
    .collect::<Result<_, _>>()?;
  let total_count = addrs
    .summary(hyphen_db::HyDbAddress(addr))
    .await
    .map_err(|e| hydb_err(e, "address summary"))?
    .map_or(0, |row| i64::from(row.tx_count));
  Ok(Json(IndexedCursorPage {
    as_of_height,
    total_count,
    value,
    next_cursor,
  }))
}

/// Map one storage-layer address-history row onto its wire summary.
fn hydb_addr_tx_summary(
  row: &hyphen_db::query::HyDbQrAddrTx,
) -> Result<AddressTxSummary, ApiError> {
  Ok(AddressTxSummary {
    txid: utils::view_hex(row.txid.as_bytes(), HashEndian::Ser),
    block_height: Some(row.block_height.get() as i32),
    block_pos: Some(row.block_pos),
    block_time: Some(rfc3339_time(row.block_time)?),
    version: row.version as i16,
    tx_type: tx_type_str(row.tx_type).to_owned(),
    size: row.size,
    value_out: row.value_out,
    net_value: row.net_value,
    direction: match row.direction {
      hyphen_db::query::HyDbTxDir::In => AddressTxDirection::In,
      hyphen_db::query::HyDbTxDir::Out => AddressTxDirection::Out,
    },
    fee: row.fee,
    tags: TxTags {
      mined: true,
      coinbase: row.block_pos == 0,
    },
  })
}

pub(crate) async fn search_address(
  state: &AppState,
  addr: &str,
) -> Result<Option<SearchResponse>, ApiError> {
  if !state.indices.address {
    return Ok(None);
  }
  let exists = state
    .hydb()?
    .addrs()
    .exists(hyphen_db::HyDbAddress(addr.to_owned()))
    .await
    .map_err(|e| hydb_err(e, "address existence"))?;
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
  let addresses = state
    .hydb()?
    .addrs()
    .search_prefix(prefix.to_owned(), limit.max(0) as u32)
    .await
    .map_err(|e| hydb_err(e, "search address prefix"))?;
  Ok(
    addresses
      .into_iter()
      .map(|address| SearchResponse::Address(SearchAddressResult { address: address.0 }))
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
  let before_cursor = match decode_address_utxo_cursor_key(cursor.as_ref())? {
    Some((height, txid, vout_index)) => {
      let outpoint = hyphen_db::HyDbOutpoint {
        txid: hydb_hash(&txid)?,
        vout: vout_index as u32,
      };
      Some(hyphen_db::HyDbCursor(format!(
        "{height}:{}",
        hex::encode(outpoint.packed())
      )))
    }
    None => None,
  };

  let addrs = state.hydb()?.addrs();
  let page = addrs
    .utxos(
      hyphen_db::HyDbAddress(addr.clone()),
      limit as u32,
      before_cursor,
    )
    .await
    .map_err(|e| hydb_err(e, "address utxos"))?;
  let next_cursor = if page.next_cursor.is_some() {
    page
      .items
      .last()
      .map(|row| {
        encode_cursor(&AddressUtxoCursor {
          as_of_height,
          address: addr.clone(),
          block_height: row.block_height.get() as i32,
          txid: utils::view_hex(row.outpoint.txid.as_bytes(), HashEndian::Ser),
          vout_index: row.outpoint.vout as i32,
        })
      })
      .transpose()?
  } else {
    None
  };
  let value: Vec<Utxo> = page
    .items
    .iter()
    .map(|row| Utxo {
      txid: utils::view_hex(row.outpoint.txid.as_bytes(), HashEndian::Ser),
      vout: row.outpoint.vout as i32,
      value: row.value,
      height: row.block_height.get() as i32,
      script_pub_key: hex::encode(&row.script_pubkey),
    })
    .collect();
  let total_count = addrs
    .summary(hyphen_db::HyDbAddress(addr))
    .await
    .map_err(|e| hydb_err(e, "address summary"))?
    .map_or(0, |row| i64::from(row.utxo_count));
  Ok(Json(IndexedCursorPage {
    as_of_height,
    total_count,
    value,
    next_cursor,
  }))
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
  let start_rank = cursor.as_ref().map_or(0, |cursor| cursor.rank);
  let before_cursor = cursor
    .as_ref()
    .map(|cursor| hyphen_db::HyDbCursor(format!("{}:{}", cursor.balance, cursor.address)));

  let addrs = state.hydb()?.addrs();
  let page = addrs
    .rich_list(limit as u32, before_cursor)
    .await
    .map_err(|e| hydb_err(e, "balances"))?;
  let supply = addrs
    .supply()
    .await
    .map_err(|e| hydb_err(e, "balances supply"))?;
  let next_cursor = if page.next_cursor.is_some() {
    let advanced_rank = start_rank + page.items.len() as i64;
    page
      .items
      .last()
      .map(|row| {
        encode_cursor(&BalancesCursor {
          as_of_height,
          rank: advanced_rank,
          balance: row.balance,
          address: row.address.0.clone(),
        })
      })
      .transpose()?
  } else {
    None
  };
  let value: Vec<BalanceEntry> = page
    .items
    .iter()
    .enumerate()
    .map(|(index, row)| BalanceEntry {
      rank: start_rank + index as i64 + 1,
      address: row.address.0.clone(),
      balance: row.balance,
      pct_supply: pct_supply(row.balance, supply.included_supply),
      total_received: row.total_received,
      total_sent: row.total_sent,
      tx_count: row.tx_count,
    })
    .collect();
  let total_count = addrs
    .bal_count()
    .await
    .map_err(|e| hydb_err(e, "balances count"))? as i64;
  Ok(Json(BalanceResponse {
    as_of_height,
    total_count,
    value,
    meta: BalancePageMeta {
      included_supply: supply.included_supply,
      supply_definition: "sum of positive balances included by this balances snapshot".to_string(),
    },
    next_cursor,
  }))
}

fn pct_supply(balance: i64, included_supply: i64) -> f64 {
  if included_supply <= 0 {
    return 0.0;
  }
  balance as f64 / included_supply as f64
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

  #[test]
  fn hydb_addr_tx_summary_keeps_address_relative_fields() {
    let row = hyphen_db::query::HyDbQrAddrTx {
      txid: hyphen_db::HyDbHash([0x11; 32]),
      block_height: hyphen_db::HyDbHeight::new(7),
      block_pos: 0,
      block_time: 7,
      version: 3,
      tx_type: hyphen_db::HyDbTxType::Normal,
      size: 200,
      value_out: 90,
      net_value: -55,
      direction: hyphen_db::query::HyDbTxDir::Out,
      fee: Some(3),
    };

    let summary = super::hydb_addr_tx_summary(&row).expect("summary should build");
    assert_eq!(summary.block_time.as_deref(), Some("1970-01-01T00:00:07Z"));
    assert_eq!(summary.block_height, Some(7));
    assert_eq!(summary.net_value, -55);
    assert_eq!(summary.tx_type, "normal");
    assert!(matches!(summary.direction, AddressTxDirection::Out));
  }
}
