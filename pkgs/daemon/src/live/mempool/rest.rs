//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! REST endpoints for live mempool state.

use axum::extract::{Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::rest::error::ApiError;
use crate::rest::openapi::{
  ErrorResponse, MempoolResponse, MempoolTxListResponse, MempoolTxSummary, TxTags,
};
use crate::rest::{
  decode_cursor, encode_cursor, rfc3339_time, AppState, CursorPage, CursorParams, LiveFeature,
};
use crate::utils::{self, HashEndian};

const MAX_MEMPOOL_TX_LIMIT: i64 = 100;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct MempoolTxCursor {
  last_seen_at: i64,
  txid: String,
}

/// Get the current mempool summary snapshot.
///
/// # Errors
///
/// Returns `ApiError` when the live mempool capability is disabled, no summary
/// has been persisted yet, a backing query fails, or one timestamp cannot be
/// normalized.
#[utoipa::path(
  get,
  path = "/api/v1/mempool",
  description = "Requires the live mempool capability. Returns the current live mempool snapshot and is not backed by the historical chain index.",
  responses(
    (status = 200, description = "Current mempool summary", body = MempoolResponse),
    (status = 501, description = "Mempool feature disabled", body = ErrorResponse),
    (status = 503, description = "Mempool summary not ready yet", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Mempool"
)]
pub async fn summary(State(state): State<AppState>) -> Result<Json<MempoolResponse>, ApiError> {
  state.check_live_feature(LiveFeature::Mempool)?;

  let row = crate::sql::live::mempool::summary(&state.pool)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query mempool summary"))?
    .ok_or_else(|| ApiError::service_unavailable("mempool summary is not ready"))?;
  let recent = crate::sql::live::mempool::recent_txs(&state.pool, MAX_MEMPOOL_TX_LIMIT)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query mempool recent txs"))?;

  Ok(Json(MempoolResponse {
    count: row.tx_count,
    total_bytes: row.total_bytes,
    usage_bytes: row.usage_bytes,
    max_bytes: row.max_bytes,
    updated_at: rfc3339_time(row.updated_at)?,
    recent_hashes: recent
      .into_iter()
      .map(|row| utils::view_hex(&row.txid, HashEndian::Ser))
      .collect(),
  }))
}

/// Get recently seen mempool transactions.
///
/// # Errors
///
/// Returns `ApiError` when the live mempool capability is disabled, a backing
/// query fails, or one timestamp cannot be normalized.
#[utoipa::path(
  get,
  path = "/api/v1/mempool/txs",
  description = "Requires the live mempool capability. Returns live mempool transactions only, not indexed chain history. If the live ISLock capability is enabled, transaction tags include current ISLock state.",
  params(CursorParams),
  responses(
    (status = 200, description = "Recent mempool transactions", body = MempoolTxListResponse),
    (status = 400, description = "Invalid pagination query", body = ErrorResponse),
    (status = 501, description = "Mempool feature disabled", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Mempool"
)]
pub async fn recent_txs(
  State(state): State<AppState>,
  Query(params): Query<CursorParams>,
) -> Result<Json<CursorPage<MempoolTxSummary>>, ApiError> {
  state.check_live_feature(LiveFeature::Mempool)?;

  let params = params.validate(MAX_MEMPOOL_TX_LIMIT)?;
  let limit = params.limit;
  let cursor = decode_mempool_tx_cursor(params.cursor.as_deref())?;
  let after = cursor
    .as_ref()
    .map(|cursor| {
      utils::decode_hex_to(&cursor.txid, HashEndian::Ser)
        .map(|txid| (cursor.last_seen_at, txid))
        .map_err(|_| ApiError::bad_request("invalid cursor txid"))
    })
    .transpose()?;
  let after_ref = after
    .as_ref()
    .map(|(last_seen_at, txid)| (*last_seen_at, txid.as_slice()));
  let mut rows = crate::sql::live::mempool::recent_tx_details(&state.pool, limit + 1, after_ref)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query mempool recent tx details"))?;
  let has_more = rows.len() as i64 > limit;
  rows.truncate(limit as usize);
  let mut value = Vec::with_capacity(rows.len());
  for row in &rows {
    value.push(MempoolTxSummary {
      txid: utils::view_hex(&row.txid, HashEndian::Ser),
      size: row.size as usize,
      value_out: row.value_out,
      first_seen_at: rfc3339_time(row.first_seen_at)?,
      last_seen_at: rfc3339_time(row.last_seen_at)?,
      tags: TxTags {
        mined: false,
        coinbase: false,
      },
    });
  }
  let total_count = crate::sql::live::mempool::summary(&state.pool)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query mempool summary"))?
    .map_or(value.len() as i64, |row| row.tx_count);

  Ok(Json(CursorPage {
    total_count,
    value,
    next_cursor: if has_more {
      rows
        .last()
        .map(|row| {
          encode_cursor(&MempoolTxCursor {
            last_seen_at: row.last_seen_at,
            txid: utils::view_hex(&row.txid, HashEndian::Ser),
          })
        })
        .transpose()?
    } else {
      None
    },
  }))
}

fn decode_mempool_tx_cursor(cursor: Option<&str>) -> Result<Option<MempoolTxCursor>, ApiError> {
  decode_cursor::<MempoolTxCursor>(cursor)
}
