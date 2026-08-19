//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Transaction REST API endpoints.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::rest::error::ApiError;
use crate::rest::hydb::{hydb_err, hydb_hash, tx_type_str};
use crate::rest::openapi::{
  ErrorResponse, RawTxResponse, SearchResponse, SearchTxResult, TxDetail, TxInput, TxListResponse,
  TxOutput, TxScriptPubKey, TxScriptSig, TxSummary, TxTags,
};
use crate::rest::{decode_cursor, encode_cursor, AppState, CursorParams, IndexedCursorPage};
use crate::utils::{self, HashEndian};

const MAX_TX_PAGE_LIMIT: i64 = 100;
const HASH_HEX_LEN: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct TxCursor {
  as_of_height: i32,
  block_height: i32,
  block_pos: i32,
  txid: String,
}

/// Get a transaction by txid.
///
/// # Errors
///
/// Returns `ApiError` when the txid is invalid, the transaction cannot be
/// found in the index or live mempool view, or any backing query fails.
#[utoipa::path(
  get,
  path = "/api/v1/tx/{txid}",
  description = "Primarily backed by the committed transaction index. The nested live object is nullable, not snapshot-bound, and may be ahead of asOfHeight. If a txid is not indexed and the live mempool capability is enabled, the server falls back to live mempool state.",
  params(("txid" = String, Path, description = "Transaction id (64 hex chars)")),
  responses(
    (status = 200, description = "Transaction detail", body = TxDetail),
    (status = 400, description = "Invalid txid", body = ErrorResponse),
    (status = 404, description = "Transaction not found", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Transactions"
)]
pub async fn get_tx(
  State(state): State<AppState>,
  Path(txid_hex): Path<String>,
) -> Result<Json<TxDetail>, ApiError> {
  if txid_hex.len() != HASH_HEX_LEN {
    return Err(ApiError::bad_request("txid must be 64 hex characters"));
  }
  let txid_lookup = utils::HashLookup::from_hex(&txid_hex)
    .map_err(|_| ApiError::bad_request("invalid txid hex"))?;
  let (primary_txid, secondary_txid) = txid_lookup.ordered_candidates(HashEndian::Ser);

  let txs = state.hydb()?.txs();
  let mut detail = txs
    .get(hydb_hash(&primary_txid)?)
    .await
    .map_err(|e| hydb_err(e, "transaction"))?;
  if detail.is_none() {
    if let Some(secondary_txid) = secondary_txid.as_ref() {
      detail = txs
        .get(hydb_hash(secondary_txid)?)
        .await
        .map_err(|e| hydb_err(e, "transaction"))?;
    }
  }

  if let Some(detail) = detail {
    let stage = state
      .stage_status_typed(crate::index::Stage::Transaction)
      .await?;
    let txid_bytes = detail.txid.as_bytes().to_vec();

    let vin: Vec<TxInput> = detail
      .inputs
      .iter()
      .map(|input| TxInput {
        n: input.vin_index,
        script_sig: TxScriptSig {
          hex: hex::encode(&input.script_sig),
        },
        sequence: input.sequence,
        address: input.prev_address.as_ref().map(|address| address.0.clone()),
        value: input.prev_value,
        txid: input
          .prev_outpoint
          .as_ref()
          .map(|outpoint| utils::view_hex(outpoint.txid.as_bytes(), HashEndian::Ser)),
        vout: input
          .prev_outpoint
          .as_ref()
          .map(|outpoint| outpoint.vout as i32),
        coinbase: input.coinbase.then(|| hex::encode(&input.script_sig)),
      })
      .collect();

    let vout: Vec<TxOutput> = detail
      .outputs
      .iter()
      .map(|output| TxOutput {
        n: output.vout_index,
        value: output.value,
        script_pub_key: TxScriptPubKey {
          hex: hex::encode(&output.script_pubkey),
        },
        address: output.address.as_ref().map(|address| address.0.clone()),
        spent: output.spent_by.is_some(),
      })
      .collect();

    let stored_tx_type = tx_type_str(detail.tx_type);
    let public_tx_type = crate::tx::public_tx_type(
      stored_tx_type,
      detail
        .outputs
        .iter()
        .map(|output| output.script_pubkey.as_slice()),
    );
    let payload = detail
      .payload
      .as_ref()
      .map(|payload| crate::tx::enrich_api(stored_tx_type, payload));

    let live = super::overlay::indexed_live(&state, &txid_bytes).await?;
    let is_coinbase = detail.block_pos == 0;
    return Ok(Json(TxDetail {
      as_of_height: Some(stage.height),
      txid: utils::view_hex(&txid_bytes, HashEndian::Ser),
      block_height: Some(detail.block_height.get() as i32),
      block_pos: Some(detail.block_pos),
      version: detail.version as i16,
      tx_type: public_tx_type.to_owned(),
      lock_time: detail.lock_time,
      size: detail.size,
      value_out: detail.value_out,
      input_count: detail.inputs.len() as i32,
      output_count: detail.outputs.len() as i32,
      fee: detail.fee,
      tags: TxTags {
        mined: true,
        coinbase: is_coinbase,
      },
      live,
      payload,
      vin,
      vout,
    }));
  }

  Ok(Json(
    super::overlay::live_tx_detail(&state, &txid_lookup).await?,
  ))
}

/// Get the raw serialized transaction as a hex string.
///
/// # Errors
///
/// Returns `ApiError` when the txid is invalid, the transaction cannot be
/// resolved from indexed blocks or the live mempool, or raw block storage
/// cannot be read.
#[utoipa::path(
  get,
  path = "/api/v1/rawtx/{txid}",
  description = "Backed by the transaction index plus local raw block storage. This is a raw payload route. If a txid is not indexed and the live mempool capability is enabled, the server can fall back to a live raw mempool transaction.",
  params(("txid" = String, Path, description = "Transaction id (64 hex chars)")),
  responses(
    (status = 200, description = "Raw transaction hex", body = RawTxResponse),
    (status = 400, description = "Invalid txid", body = ErrorResponse),
    (status = 404, description = "Transaction not found", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Transactions"
)]
pub async fn get_rawtx(
  State(state): State<AppState>,
  Path(txid_hex): Path<String>,
) -> Result<Json<RawTxResponse>, ApiError> {
  if txid_hex.len() != HASH_HEX_LEN {
    return Err(ApiError::bad_request("txid must be 64 hex characters"));
  }
  let txid_lookup = utils::HashLookup::from_hex(&txid_hex)
    .map_err(|_| ApiError::bad_request("invalid txid hex"))?;
  let (primary_txid, secondary_txid) = txid_lookup.ordered_candidates(HashEndian::Ser);

  let hydb = state.hydb()?;
  let mut detail = hydb
    .txs()
    .get(hydb_hash(&primary_txid)?)
    .await
    .map_err(|e| hydb_err(e, "transaction location"))?;
  if detail.is_none() {
    if let Some(secondary_txid) = secondary_txid.as_ref() {
      detail = hydb
        .txs()
        .get(hydb_hash(secondary_txid)?)
        .await
        .map_err(|e| hydb_err(e, "transaction location"))?;
    }
  }

  if let Some(detail) = detail {
    let stage = state
      .stage_status_typed(crate::index::Stage::Transaction)
      .await?;
    let block_hash = hydb
      .blocks()
      .hash_for_height(detail.block_height)
      .await
      .map_err(|e| hydb_err(e, "block hash for tx"))?
      .ok_or_else(|| ApiError::internal("block not found for tx"))?;
    let raw_block = hydb
      .blocks()
      .raw_bytes(block_hash)
      .await
      .map_err(|e| hydb_err(e, "raw block"))?
      .ok_or_else(|| ApiError::not_found("raw block not in store"))?;

    let ranges = dash_primitives::block::tx_byte_ranges(&raw_block)
      .map_err(|e| ApiError::internal(format!("tx ranges: {e}")))?;
    let (start, end) = ranges
      .get(detail.block_pos as usize)
      .ok_or_else(|| ApiError::internal("tx position out of range"))?;

    return Ok(Json(RawTxResponse {
      as_of_height: Some(stage.height),
      source: crate::rest::openapi::RawValueSource::Indexed,
      rawtx: hex::encode(&raw_block[*start..*end]),
    }));
  }

  Ok(Json(
    super::overlay::live_rawtx(&state, &txid_lookup).await?,
  ))
}

/// List transactions (newest first).
///
/// # Errors
///
/// Returns `ApiError` when the backing transaction list query fails.
#[utoipa::path(
  get,
  path = "/api/v1/txs",
  description = "Backed by the committed transaction index. Returns canonical mined transactions only from one committed snapshot.",
  params(CursorParams),
  responses(
    (status = 200, description = "Paginated transactions", body = TxListResponse),
    (status = 400, description = "Invalid pagination query", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Transactions"
)]
pub async fn list_txs(
  State(state): State<AppState>,
  Query(params): Query<CursorParams>,
) -> Result<Json<IndexedCursorPage<TxSummary>>, ApiError> {
  let stage = state
    .stage_status_typed(crate::index::Stage::Transaction)
    .await?;
  let params = params.validate(MAX_TX_PAGE_LIMIT)?;
  let limit = params.limit;
  let cursor = decode_tx_cursor(params.cursor.as_deref(), stage.height)?;
  let as_of_height = cursor
    .as_ref()
    .map_or(stage.height, |cursor| cursor.as_of_height);
  let before = decode_tx_cursor_key(cursor.as_ref())?;
  let before_cursor = before.as_ref().map(|(height, pos, txid)| {
    hyphen_db::HyDbCursor(format!("{height}:{pos}:{}", hex::encode(txid)))
  });

  let txs = state.hydb()?.txs();
  let page = txs
    .list(limit as u32, before_cursor)
    .await
    .map_err(|e| hydb_err(e, "list transactions"))?;
  let next_cursor = if page.next_cursor.is_some() {
    page
      .items
      .last()
      .map(|row| {
        encode_cursor(&TxCursor {
          as_of_height,
          block_height: row.block_height.get() as i32,
          block_pos: row.block_pos,
          txid: utils::view_hex(row.txid.as_bytes(), HashEndian::Ser),
        })
      })
      .transpose()?
  } else {
    None
  };

  let value: Vec<TxSummary> = page
    .items
    .iter()
    .map(|row| TxSummary {
      txid: utils::view_hex(row.txid.as_bytes(), HashEndian::Ser),
      block_height: row.block_height.get() as i32,
      block_pos: row.block_pos,
      version: row.version as i16,
      tx_type: tx_type_str(row.tx_type).to_owned(),
      lock_time: row.lock_time,
      size: row.size,
      value_out: row.value_out,
      input_count: row.input_count,
      output_count: row.output_count,
      tags: TxTags {
        mined: true,
        coinbase: row.block_pos == 0,
      },
    })
    .collect();
  let total_count = txs
    .total()
    .await
    .map_err(|e| hydb_err(e, "transactions summary"))? as i64;
  Ok(Json(IndexedCursorPage {
    as_of_height,
    total_count,
    next_cursor,
    value,
  }))
}

fn decode_tx_cursor(
  cursor: Option<&str>,
  current_height: i32,
) -> Result<Option<TxCursor>, ApiError> {
  let Some(cursor) = decode_cursor::<TxCursor>(cursor)? else {
    return Ok(None);
  };
  if cursor.as_of_height > current_height {
    return Err(ApiError::bad_request(
      "cursor snapshot is ahead of current request",
    ));
  }
  Ok(Some(cursor))
}

fn decode_tx_cursor_key(
  cursor: Option<&TxCursor>,
) -> Result<Option<(i32, i32, Vec<u8>)>, ApiError> {
  cursor
    .map(|cursor| {
      utils::decode_hex_to(&cursor.txid, HashEndian::Ser)
        .map(|txid| (cursor.block_height, cursor.block_pos, txid))
        .map_err(|_| ApiError::bad_request("invalid cursor txid"))
    })
    .transpose()
}

pub(crate) async fn search_by_txid(
  state: &AppState,
  query: &str,
  txid_bytes: &[u8],
) -> Result<Option<SearchResponse>, ApiError> {
  if let Some(detail) = state
    .hydb()?
    .txs()
    .get(hydb_hash(txid_bytes)?)
    .await
    .map_err(|e| hydb_err(e, "search transaction"))?
  {
    return Ok(Some(SearchResponse::Tx(SearchTxResult {
      tx_hash: utils::view_hex(detail.txid.as_bytes(), HashEndian::Ser),
      tx_tags: TxTags {
        mined: true,
        coinbase: detail.block_pos == 0,
      },
    })));
  }

  let txid_lookup =
    utils::HashLookup::from_hex(query).map_err(|_| ApiError::bad_request("invalid txid hex"))?;
  if let Some(response) = super::overlay::live_search_response(state, query, &txid_lookup).await? {
    return Ok(Some(response));
  }

  Ok(None)
}

pub(crate) async fn search_by_txid_prefix(
  state: &AppState,
  prefix: &str,
  limit: i64,
) -> Result<Vec<SearchResponse>, ApiError> {
  let hashes = state
    .hydb()?
    .txs()
    .search_prefix(prefix.to_owned(), limit.max(0) as u32)
    .await
    .map_err(|e| hydb_err(e, "search transaction prefix"))?;
  Ok(
    hashes
      .into_iter()
      .map(|txid| {
        SearchResponse::Tx(SearchTxResult {
          tx_hash: utils::view_hex(txid.as_bytes(), HashEndian::Ser),
          // Prefix hits carry no positional detail; coinbase can't be inferred.
          tx_tags: TxTags {
            mined: true,
            coinbase: false,
          },
        })
      })
      .collect(),
  )
}

#[cfg(test)]
mod tests {
  use axum::extract::{Path, State};

  fn normal_tx_bytes() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1i32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&[0x00; 32]);
    buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&42i64.to_le_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
  }

  fn mempool_tx_fixture(txid: Vec<u8>, raw_tx: Vec<u8>) -> crate::live::state::MempoolTx {
    let size = raw_tx.len() as i32;
    crate::live::state::MempoolTx {
      txid,
      raw_tx,
      version: 1,
      tx_type: "normal".to_owned(),
      lock_time: 0,
      size,
      value_out: 42,
      input_count: 1,
      output_count: 1,
      fee: None,
      payload: None,
      inputs: Vec::new(),
      outputs: Vec::new(),
      first_seen_at: 10,
      last_seen_at: 20,
    }
  }

  #[tokio::test]
  async fn mempool_tx_value_marks_unmined_and_islock_state() {
    let db = crate::rest::hydb_fake::MemDb::new().into_hydb();
    let value = crate::live::mempool::query::build_tx_value(
      &db,
      &"11".repeat(32),
      &[0x11; 32],
      &normal_tx_bytes(),
      76,
      16,
      true,
    )
    .await
    .expect("mempool tx should decode");
    assert_eq!(
      value.tags,
      crate::rest::openapi::TxTags {
        mined: false,
        coinbase: false,
      }
    );
    assert_eq!(value.block_height, None);
    assert_eq!(value.tx_type, "normal");
  }

  #[tokio::test]
  async fn rawtx_falls_back_to_recent_mempool_cache() {
    let txid = vec![0x11; 32];
    let raw_tx = normal_tx_bytes();
    let live_state = crate::live::state::LiveState::new();
    live_state.insert_mempool_tx(mempool_tx_fixture(txid.clone(), raw_tx.clone()));

    let mut state = crate::tests::app_state();
    state.live.mempool = true;
    state.live_state = std::sync::Arc::new(live_state);
    state.hydb = crate::rest::hydb_fake::MemDb::new().into_hydb();

    let value = super::get_rawtx(State(state), Path(hex::encode(txid)))
      .await
      .expect("mempool raw tx should resolve")
      .0;
    assert_eq!(value.rawtx, hex::encode(raw_tx));
  }

  #[tokio::test]
  async fn get_tx_prefers_live_detail_over_chain_miss() {
    let txid = vec![0x22; 32];
    let live_state = crate::live::state::LiveState::new();
    live_state.insert_mempool_tx(crate::live::state::MempoolTx {
      txid: txid.clone(),
      raw_tx: normal_tx_bytes(),
      version: 3,
      tx_type: "normal".to_owned(),
      lock_time: 5,
      size: 123,
      value_out: 42,
      input_count: 1,
      output_count: 1,
      fee: Some(7),
      payload: Some(serde_json::json!({"memo": "live"})),
      inputs: vec![crate::live::state::MempoolTxInput {
        vin_index: 0,
        sequence: 99,
        prev_txid: Some(vec![0x33; 32]),
        prev_vout: Some(1),
        coinbase: None,
        script_sig: vec![0xaa],
        prev_address: Some("Xprev".to_owned()),
        prev_value: Some(50),
      }],
      outputs: vec![crate::live::state::MempoolTxOutput {
        vout_index: 0,
        value: 42,
        script_pubkey: vec![0xbb],
        address: Some("Xdest".to_owned()),
      }],
      first_seen_at: 10,
      last_seen_at: 20,
    });
    live_state.upsert_islock(crate::live::state::IsLock {
      txid: txid.clone(),
      raw_islock: vec![0xcc],
      seen_at: 25,
    });

    let mut state = crate::tests::app_state();
    state.live.mempool = true;
    state.live.islock = true;
    state.live_state = std::sync::Arc::new(live_state);
    state.hydb = crate::rest::hydb_fake::MemDb::new().into_hydb();

    let value = super::get_tx(State(state), Path(hex::encode(&txid)))
      .await
      .expect("mempool tx detail should resolve")
      .0;
    assert_eq!(
      value.live.tags,
      Some(crate::rest::openapi::TxLiveTags {
        mempool: Some(true),
        islock: Some(true),
      })
    );
    assert!(!value.tags.mined);
    assert!(!value.tags.coinbase);
    assert_eq!(value.fee, Some(7));
    assert_eq!(value.payload, Some(serde_json::json!({"memo": "live"})));
    assert_eq!(value.vin[0].address.as_deref(), Some("Xprev"));
    assert_eq!(value.vout[0].address.as_deref(), Some("Xdest"));
  }

  // The indexed `get_tx` path serves chain data from the storage engine; a
  // seeded in-memory `HyphenDb` fake stands in for a live Postgres.
  #[tokio::test]
  async fn get_tx_uses_indexed_prevout_address_fallback() -> Result<(), Box<dyn std::error::Error>>
  {
    let prev_txid = vec![0x33; 32];
    let txid = vec![0x44; 32];
    let detail = hyphen_db::query::HyDbQrTxDetail {
      txid: hyphen_db::HyDbHash::from_slice(&txid)?,
      block_height: hyphen_db::HyDbHeight::new(12),
      block_pos: 1,
      version: 1,
      tx_type: hyphen_db::HyDbTxType::Normal,
      lock_time: 0,
      size: 100,
      value_out: 40,
      fee: Some(10),
      inputs: vec![hyphen_db::query::HyDbQrTxIn {
        vin_index: 0,
        prev_outpoint: Some(hyphen_db::HyDbOutpoint {
          txid: hyphen_db::HyDbHash::from_slice(&prev_txid)?,
          vout: 0,
        }),
        coinbase: false,
        prev_address: Some(hyphen_db::HyDbAddress("Xprev".to_owned())),
        prev_value: Some(50),
        script_sig: vec![0xaa],
        sequence: 99,
      }],
      outputs: vec![hyphen_db::query::HyDbQrTxOut {
        vout_index: 0,
        value: 40,
        address: Some(hyphen_db::HyDbAddress("Xdest".to_owned())),
        address_type: None,
        script_pubkey: vec![0xcc],
        spent_by: None,
      }],
      payload: None,
    };

    let mut state = crate::tests::app_state();
    state.indices.address = true;
    state.hydb = crate::rest::hydb_fake::MemDb::new()
      .with_tx(detail)
      .with_tip(12, [0x12; 32])
      .into_hydb();

    let value = super::get_tx(State(state), Path(hex::encode(&txid)))
      .await
      .expect("indexed tx detail should resolve")
      .0;
    assert!(value.tags.mined);
    assert_eq!(value.vin[0].address.as_deref(), Some("Xprev"));
    assert_eq!(value.vin[0].value, Some(50));
    assert_eq!(value.vout[0].address.as_deref(), Some("Xdest"));
    Ok(())
  }
}
