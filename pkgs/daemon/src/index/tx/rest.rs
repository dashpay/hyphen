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

  let row = match super::sql::transaction(&state.pool, &primary_txid).await {
    Ok(Some(row)) => Some(row),
    Ok(None) => {
      if let Some(secondary_txid) = secondary_txid.as_ref() {
        super::sql::transaction(&state.pool, secondary_txid)
          .await
          .map_err(|e| ApiError::internal(format!("query: {e}")))?
      } else {
        None
      }
    }
    Err(e) => return Err(ApiError::internal(format!("query: {e}"))),
  };

  if let Some(row) = row {
    let stage = state
      .stage_status_typed(crate::index::Stage::Transaction)
      .await?;
    let has_address_index = state.indices.address;

    let input_rows = super::sql::transaction_inputs(&state.pool, &row.txid, has_address_index)
      .await
      .map_err(|e| ApiError::internal(format!("inputs: {e}")))?;

    let vin: Vec<TxInput> = input_rows
      .iter()
      .map(|row| TxInput {
        n: row.vin_index,
        script_sig: TxScriptSig {
          hex: hex::encode(&row.script_sig),
        },
        sequence: row.sequence,
        address: row.prev_address.clone(),
        value: row.prev_value,
        txid: row
          .prev_txid
          .as_ref()
          .map(|txid| utils::view_hex(txid, HashEndian::Ser)),
        vout: row.prev_vout,
        coinbase: row
          .prev_txid
          .is_none()
          .then(|| hex::encode(&row.script_sig)),
      })
      .collect();

    let output_rows = super::sql::transaction_outputs(&state.pool, &row.txid, has_address_index)
      .await
      .map_err(|e| ApiError::internal(format!("outputs: {e}")))?;

    let vout: Vec<TxOutput> = output_rows
      .iter()
      .map(|row| TxOutput {
        n: row.vout_index,
        value: row.value,
        script_pub_key: TxScriptPubKey {
          hex: hex::encode(&row.script_pubkey),
        },
        address: row.address.clone(),
        spent: row.spent_txid.is_some(),
      })
      .collect();

    let payload = super::sql::transaction_payload(&state.pool, &row.txid)
      .await
      .map_err(|e| ApiError::internal(format!("payload: {e}")))?
      .map(|row| crate::tx::enrich_api(&row.tx_type.to_string(), &row.payload));
    let stored_tx_type = row.tx_type.to_string();
    let public_tx_type = crate::tx::public_tx_type(
      &stored_tx_type,
      output_rows.iter().map(|row| row.script_pubkey.as_slice()),
    );

    let live = super::overlay::indexed_live(&state, &row.txid).await?;
    let is_coinbase = row.block_pos == 0;
    return Ok(Json(TxDetail {
      as_of_height: Some(stage.height),
      txid: utils::view_hex(&row.txid, HashEndian::Ser),
      block_height: Some(row.block_height),
      block_pos: Some(row.block_pos),
      version: row.version as i16,
      tx_type: public_tx_type.to_owned(),
      lock_time: row.lock_time,
      size: row.size,
      value_out: row.value_out,
      input_count: row.input_count,
      output_count: row.output_count,
      fee: row.fee,
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

  let row = match super::sql::transaction_location(&state.pool, &primary_txid).await {
    Ok(Some(row)) => Some(row),
    Ok(None) => {
      if let Some(secondary_txid) = secondary_txid.as_ref() {
        super::sql::transaction_location(&state.pool, secondary_txid)
          .await
          .map_err(|e| ApiError::internal(format!("query: {e}")))?
      } else {
        None
      }
    }
    Err(e) => return Err(ApiError::internal(format!("query: {e}"))),
  };

  if let Some(row) = row {
    let stage = state
      .stage_status_typed(crate::index::Stage::Transaction)
      .await?;
    let block_hash = crate::index::block::sql::block_hash_for_height(&state.pool, row.block_height)
      .await
      .map_err(|e| ApiError::internal(format!("block hash: {e}")))?
      .ok_or_else(|| ApiError::internal("block not found for tx"))?;

    let db = &state.db;
    let blocks_cf = db
      .cf_handle(crate::db::CF_BLOCKS)
      .ok_or_else(|| ApiError::internal("blocks CF missing"))?;

    let raw_block = db
      .get_cf(blocks_cf, &block_hash)
      .map_err(|e| ApiError::internal(format!("rocksdb: {e}")))?
      .ok_or_else(|| ApiError::not_found("raw block not in store"))?;

    let ranges = dash_primitives::hash::tx_byte_ranges(&raw_block)
      .map_err(|e| ApiError::internal(format!("tx ranges: {e}")))?;
    let (start, end) = ranges
      .get(row.block_pos as usize)
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

  let before_ref = before
    .as_ref()
    .map(|(height, pos, txid)| (*height, *pos, txid.as_slice()));
  let mut rows = super::sql::list_transactions(&state.pool, limit + 1, before_ref)
    .await
    .map_err(|e| ApiError::internal(format!("query: {e}")))?;
  let has_more = rows.len() as i64 > limit;
  rows.truncate(limit as usize);
  let next_cursor = if has_more {
    rows
      .last()
      .map(|row| {
        encode_cursor(&TxCursor {
          as_of_height,
          block_height: row.block_height,
          block_pos: row.block_pos,
          txid: utils::view_hex(&row.txid, HashEndian::Ser),
        })
      })
      .transpose()?
  } else {
    None
  };

  let value: Vec<TxSummary> = rows
    .iter()
    .map(|row| TxSummary {
      txid: utils::view_hex(&row.txid, HashEndian::Ser),
      block_height: row.block_height,
      block_pos: row.block_pos,
      version: row.version as i16,
      tx_type: row.tx_type.to_string(),
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
  let total_count = super::sql::transactions_summary(&state.pool)
    .await
    .map_err(|e| ApiError::internal(format!("summary: {e}")))?
    .total;
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
  if let Some(row) = super::sql::transaction(&state.pool, txid_bytes)
    .await
    .map_err(|e| ApiError::from_daemon(e, "search transaction"))?
  {
    return Ok(Some(SearchResponse::Tx(SearchTxResult {
      id: utils::view_hex(&row.txid, HashEndian::Ser),
      tx_tags: TxTags {
        mined: true,
        coinbase: row.block_pos == 0,
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
  let rows = super::sql::search_transactions_by_prefix(&state.pool, prefix, limit)
    .await
    .map_err(|e| ApiError::from_daemon(e, "search transaction prefix"))?;
  Ok(
    rows
      .into_iter()
      .map(|row| {
        SearchResponse::Tx(SearchTxResult {
          id: utils::view_hex(&row.txid, HashEndian::Ser),
          tx_tags: TxTags {
            mined: true,
            coinbase: row.block_pos == 0,
          },
        })
      })
      .collect(),
  )
}

#[cfg(test)]
mod tests {
  use std::fs;
  use std::sync::Arc;
  use std::time::{SystemTime, UNIX_EPOCH};

  use axum::extract::{Path, State};
  use rocksdb::DB;

  use crate::rest::{AppState, IndexCapabilities, LiveCapabilities, OverlayCapabilities};

  async fn seed_sync_state(pool: &crate::sql::SqlPool) -> Result<(), crate::error::DaemonError> {
    for stage in [
      "block",
      "transaction",
      "script",
      "spent",
      "metrics",
      "validator",
    ] {
      crate::sql::sync::update_indexed_stage_height(
        pool,
        crate::index::Stage::from_name(stage).expect("stage should be valid"),
        12,
      )
      .await?;
    }
    Ok(())
  }

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

  #[tokio::test]
  async fn mempool_tx_value_marks_unmined_and_islock_state() {
    let pool = crate::sql::SqlPool::connect_lazy("postgres://localhost/unused")
      .expect("lazy postgres pool should build");
    let value = crate::live::mempool::query::build_tx_value(
      &pool,
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
  async fn rawtx_falls_back_to_recent_mempool_cache_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };
    let pool = crate::sql::connect_test_pool(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
    )
    .await?;
    crate::sql::bootstrap::reset(&pool).await?;

    let txid = vec![0x11; 32];
    let raw_tx = normal_tx_bytes();
    let mut tx = pool.begin().await?;
    crate::sql::live::mempool::upsert_recent_tx(
      &mut tx,
      &crate::sql::live::mempool::MempoolRecentTxRow {
        txid: txid.clone(),
        raw_tx: raw_tx.clone(),
        first_seen_at: 10,
        last_seen_at: 20,
      },
    )
    .await?;
    tx.commit().await?;

    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_nanos())
      .unwrap_or_default();
    let db_path = std::env::temp_dir().join(format!("hyphen-rawtx-test-{unique}"));
    let db = DB::open_default(&db_path)?;
    let _ = fs::remove_dir_all(&db_path);

    let state = AppState {
      address_pool: pool.clone(),
      pool,
      db: Arc::new(db),
      secondary_stores: None,
      readiness_cache: Arc::new(crate::readiness::ReadinessCache::new(false, 1)),
      spent_cache: Arc::new(crate::index::spent::cache::SpentQueryCache::disabled()),
      address_hot_cache_threshold: crate::config::DEFAULT_ADDRESS_CACHE_HOT_THRESHOLD,
      address_summary_catchup_lag: crate::config::DEFAULT_ADDRESS_SUMMARY_CATCHUP_LAG,
      p2pkh_version: 76,
      p2sh_version: 16,
      indices: IndexCapabilities {
        address: false,
        validator: false,
        metrics: false,
      },
      live: LiveCapabilities {
        mempool: true,
        islock: false,
        network: false,
        governance: false,
        stream: false,
      },
      overlays: OverlayCapabilities {
        protx: false,
        chainlock: false,
      },
      recent_mempool_txs: 25,
      stream_hub: None,
      stream_heartbeat_secs: 1,
    };

    let value = super::get_rawtx(State(state), Path(hex::encode(txid)))
      .await
      .expect("mempool raw tx should resolve")
      .0;
    assert_eq!(value.rawtx, hex::encode(raw_tx));
    Ok(())
  }

  #[tokio::test]
  async fn get_tx_prefers_persisted_live_detail_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };
    let pool = crate::sql::connect_test_pool(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
    )
    .await?;
    crate::sql::bootstrap::reset(&pool).await?;

    let txid = vec![0x22; 32];
    let raw_tx = normal_tx_bytes();
    let mut tx = pool.begin().await?;
    crate::sql::live::mempool::upsert_recent_tx(
      &mut tx,
      &crate::sql::live::mempool::MempoolRecentTxRow {
        txid: txid.clone(),
        raw_tx,
        first_seen_at: 10,
        last_seen_at: 20,
      },
    )
    .await?;
    crate::sql::live::mempool::upsert_tx_detail(
      &mut tx,
      &crate::sql::live::mempool::MempoolTxDetailRow {
        txid: txid.clone(),
        version: 3,
        tx_type: "normal".to_owned(),
        lock_time: 5,
        size: 123,
        value_out: 42,
        input_count: 1,
        output_count: 1,
        fee: Some(7),
        payload: Some(serde_json::json!({"memo": "live"})),
        first_seen_at: 10,
        last_seen_at: 20,
      },
      &[crate::sql::live::mempool::MempoolTxInputRow {
        txid: txid.clone(),
        vin_index: 0,
        sequence: 99,
        prev_txid: Some(vec![0x33; 32]),
        prev_vout: Some(1),
        coinbase: None,
        script_sig: vec![0xaa],
        prev_address: Some("Xprev".to_owned()),
        prev_value: Some(50),
      }],
      &[crate::sql::live::mempool::MempoolTxOutputRow {
        txid: txid.clone(),
        vout_index: 0,
        value: 42,
        script_pubkey: vec![0xbb],
        address: Some("Xdest".to_owned()),
      }],
      &[crate::sql::live::mempool::MempoolAddressTxRow {
        address: "Xdest".to_owned(),
        txid: txid.clone(),
        last_seen_at: 20,
      }],
    )
    .await?;
    crate::sql::live::islock::upsert_current(
      &mut tx,
      &crate::sql::live::islock::IslockCurrentRow {
        txid: txid.clone(),
        raw_islock: vec![0xcc],
        seen_at: 25,
      },
    )
    .await?;
    tx.commit().await?;

    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_nanos())
      .unwrap_or_default();
    let db_path = std::env::temp_dir().join(format!("hyphen-gettx-test-{unique}"));
    let db = DB::open_default(&db_path)?;
    let _ = fs::remove_dir_all(&db_path);

    let state = AppState {
      address_pool: pool.clone(),
      pool,
      db: Arc::new(db),
      secondary_stores: None,
      readiness_cache: Arc::new(crate::readiness::ReadinessCache::new(false, 1)),
      spent_cache: Arc::new(crate::index::spent::cache::SpentQueryCache::disabled()),
      address_hot_cache_threshold: crate::config::DEFAULT_ADDRESS_CACHE_HOT_THRESHOLD,
      address_summary_catchup_lag: crate::config::DEFAULT_ADDRESS_SUMMARY_CATCHUP_LAG,
      p2pkh_version: 76,
      p2sh_version: 16,
      indices: IndexCapabilities {
        address: false,
        validator: false,
        metrics: false,
      },
      live: LiveCapabilities {
        mempool: true,
        islock: true,
        network: false,
        governance: false,
        stream: false,
      },
      overlays: OverlayCapabilities {
        protx: false,
        chainlock: false,
      },
      recent_mempool_txs: 25,
      stream_hub: None,
      stream_heartbeat_secs: 1,
    };

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
    Ok(())
  }

  #[tokio::test]
  async fn get_tx_uses_indexed_prevout_address_fallback_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };
    let pool = crate::sql::connect_test_pool(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
    )
    .await?;
    crate::sql::bootstrap::reset(&pool).await?;
    seed_sync_state(&pool).await?;

    let prev_txid = vec![0x33; 32];
    let txid = vec![0x44; 32];
    let mut block_tx = pool.begin().await?;
    crate::sql::index::block::insert_block(
      &mut block_tx,
      12,
      &[0x01; 32],
      &[0x02; 32],
      &[0x03; 32],
      0,
      0,
      0,
      1,
      160,
      2,
      "00",
      &crate::index::block::util::CbTxFields::default(),
    )
    .await?;
    block_tx.commit().await?;

    let mut sql_conn = pool.acquire().await?;
    crate::sql::index::tx::insert_transaction_row(
      &mut sql_conn,
      &crate::sql::index::tx::TxRow {
        txid: prev_txid.clone(),
        block_height: 12,
        block_pos: 0,
        version: 1,
        tx_type: crate::sql::types::TxTypeDb::Normal,
        lock_time: 0,
        size: 100,
        value_out: 50,
        input_count: 1,
        output_count: 1,
        fee: Some(0),
      },
    )
    .await?;
    crate::sql::index::tx::insert_transaction_row(
      &mut sql_conn,
      &crate::sql::index::tx::TxRow {
        txid: txid.clone(),
        block_height: 12,
        block_pos: 1,
        version: 1,
        tx_type: crate::sql::types::TxTypeDb::Normal,
        lock_time: 0,
        size: 100,
        value_out: 40,
        input_count: 1,
        output_count: 1,
        fee: Some(10),
      },
    )
    .await?;
    crate::sql::index::tx::insert_transaction_input_rows(
      &mut sql_conn,
      12,
      &txid,
      &[crate::sql::index::tx::TxInputRow {
        vin_index: 0,
        prev_txid: Some(prev_txid.clone()),
        prev_vout: Some(0),
        script_sig: vec![0xaa],
        sequence: 99,
        prev_address: None,
        prev_value: None,
      }],
    )
    .await?;
    crate::sql::index::tx::insert_transaction_output_rows(
      &mut sql_conn,
      &prev_txid,
      12,
      &[crate::sql::index::tx::TxOutputRow {
        vout_index: 0,
        value: 50,
        script_pubkey: vec![0xbb],
        address: Some("Xprev".to_owned()),
        spent_txid: None,
      }],
    )
    .await?;
    crate::sql::index::tx::insert_transaction_output_rows(
      &mut sql_conn,
      &txid,
      12,
      &[crate::sql::index::tx::TxOutputRow {
        vout_index: 0,
        value: 40,
        script_pubkey: vec![0xcc],
        address: Some("Xdest".to_owned()),
        spent_txid: None,
      }],
    )
    .await?;

    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_nanos())
      .unwrap_or_default();
    let db_path = std::env::temp_dir().join(format!("hyphen-indexed-gettx-test-{unique}"));
    let db = DB::open_default(&db_path)?;
    let _ = fs::remove_dir_all(&db_path);

    let state = AppState {
      address_pool: pool.clone(),
      pool,
      db: Arc::new(db),
      secondary_stores: None,
      readiness_cache: Arc::new(crate::readiness::ReadinessCache::new(false, 1)),
      spent_cache: Arc::new(crate::index::spent::cache::SpentQueryCache::disabled()),
      address_hot_cache_threshold: crate::config::DEFAULT_ADDRESS_CACHE_HOT_THRESHOLD,
      address_summary_catchup_lag: crate::config::DEFAULT_ADDRESS_SUMMARY_CATCHUP_LAG,
      p2pkh_version: 76,
      p2sh_version: 16,
      indices: IndexCapabilities {
        address: true,
        validator: false,
        metrics: false,
      },
      live: LiveCapabilities {
        mempool: false,
        islock: false,
        network: false,
        governance: false,
        stream: false,
      },
      overlays: OverlayCapabilities {
        protx: false,
        chainlock: false,
      },
      recent_mempool_txs: 25,
      stream_hub: None,
      stream_heartbeat_secs: 1,
    };

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
