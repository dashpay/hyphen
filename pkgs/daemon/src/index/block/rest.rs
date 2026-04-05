//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block REST API endpoints.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};

use crate::rest::error::ApiError;
use crate::rest::openapi::{
  BlockDetail, BlockListResponse, BlockSummary, BlockTxListResponse, CbTxFields, ErrorResponse,
  RawBlockResponse, SearchBlockResult, SearchResponse, TxSummary, TxTags,
};
use crate::rest::{
  decode_cursor, encode_cursor, rfc3339_time, AppState, CursorParams, IndexedCursorPage,
};
use crate::utils::{self, HashEndian};

const MAX_BLOCK_PAGE_LIMIT: i64 = 100;
const HASH_HEX_LEN: usize = 64;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct BlockCursor {
  as_of_height: i32,
  height: i32,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct BlockTxCursor {
  as_of_height: i32,
  block_height: i32,
  block_pos: i32,
  txid: String,
}

/// List blocks (newest first).
///
/// # Errors
///
/// Returns `ApiError` when a backing query fails or one block timestamp cannot
/// be normalized to RFC 3339.
#[utoipa::path(
  get,
  path = "/api/v1/blocks",
  description = "Backed by the committed block index. Returns canonical block summaries from one committed index snapshot. The nested live object is nullable, not snapshot-bound, and may be ahead of asOfHeight.",
  params(CursorParams),
  responses(
    (status = 200, description = "Paginated blocks", body = BlockListResponse),
    (status = 400, description = "Invalid pagination query", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Blocks"
)]
pub async fn list_blocks(
  State(state): State<AppState>,
  Query(params): Query<CursorParams>,
) -> Result<Json<IndexedCursorPage<BlockSummary>>, ApiError> {
  let stage = state.stage_status_typed(crate::index::Stage::Block).await?;
  let params = params.validate(MAX_BLOCK_PAGE_LIMIT)?;
  let limit = params.limit;
  let cursor = decode_block_cursor(params.cursor.as_deref(), stage.height)?;
  let as_of_height = cursor
    .as_ref()
    .map_or(stage.height, |cursor| cursor.as_of_height);

  let mut rows = super::sql::list_blocks(
    &state.pool,
    limit + 1,
    cursor.as_ref().map(|cursor| cursor.height),
  )
  .await
  .map_err(|e| ApiError::internal(format!("query: {e}")))?;
  let has_more = rows.len() as i64 > limit;
  rows.truncate(limit as usize);
  let next_cursor = if has_more {
    rows
      .last()
      .map(|row| {
        encode_cursor(&BlockCursor {
          as_of_height,
          height: row.height,
        })
      })
      .transpose()?
  } else {
    None
  };

  let chainlock_snapshot = super::overlay::fetch_chainlock_snapshot(&state).await?;
  let value: Vec<BlockSummary> = rows
    .iter()
    .map(|row| {
      Ok(BlockSummary {
        height: row.height,
        hash: utils::view_hex(&row.hash, HashEndian::Ser),
        previousblockhash: utils::view_hex(&row.prev_hash, HashEndian::Ser),
        time: rfc3339_time(i64::from(row.time))?,
        bits: format!("{:08x}", row.bits as u32),
        nonce: row.nonce,
        version: row.version,
        size: row.size,
        tx_count: row.tx_count,
        tags: block_tags(false),
        live: super::overlay::block_live_from_snapshot(chainlock_snapshot.as_ref(), &row.hash)?,
      })
    })
    .collect::<Result<Vec<_>, ApiError>>()?;
  Ok(Json(IndexedCursorPage {
    as_of_height,
    total_count: i64::from(as_of_height) + 1,
    next_cursor,
    value,
  }))
}

/// Get a single block by height or hash.
///
/// # Errors
///
/// Returns `ApiError` when the identifier is invalid, the block does not
/// exist, a backing query fails, or one block timestamp cannot be normalized.
#[utoipa::path(
  get,
  path = "/api/v1/block/{hashOrHeight}",
  description = "Backed by the committed block index. Hash lookups can also resolve stored orphan blocks from one committed index snapshot. The nested live object is nullable, not snapshot-bound, and may be ahead of asOfHeight.",
  params(("hashOrHeight" = String, Path, description = "Block height or 64-char hash")),
  responses(
    (status = 200, description = "Block detail", body = BlockDetail),
    (status = 400, description = "Invalid block identifier", body = ErrorResponse),
    (status = 404, description = "Block not found", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Blocks"
)]
pub async fn get_block(
  State(state): State<AppState>,
  Path(hash_or_height): Path<String>,
) -> Result<Json<BlockDetail>, ApiError> {
  let stage = state.stage_status_typed(crate::index::Stage::Block).await?;
  let height_lookup = hash_or_height.parse::<i32>().ok();
  let row = if let Some(height) = height_lookup {
    if height < 0 {
      return Err(ApiError::bad_request("block height must be non-negative"));
    }
    super::sql::block_by_height(&state.pool, height).await
  } else if hash_or_height.len() == HASH_HEX_LEN {
    let hash_lookup = utils::HashLookup::from_hex(&hash_or_height)
      .map_err(|_| ApiError::bad_request("invalid hash hex"))?;
    let (primary_hash, secondary_hash) = hash_lookup.ordered_candidates(HashEndian::Ser);
    match super::sql::block_by_hash(&state.pool, &primary_hash).await {
      Ok(Some(row)) => Ok(Some(row)),
      Ok(None) => match super::sql::orphan_block_by_hash(&state.pool, &primary_hash).await {
        Ok(Some(row)) => Ok(Some(row)),
        Ok(None) => {
          if let Some(secondary_hash) = secondary_hash.as_ref() {
            match super::sql::block_by_hash(&state.pool, secondary_hash).await {
              Ok(Some(row)) => Ok(Some(row)),
              Ok(None) => super::sql::orphan_block_by_hash(&state.pool, secondary_hash).await,
              Err(err) => Err(err),
            }
          } else {
            Ok(None)
          }
        }
        Err(err) => Err(err),
      },
      Err(err) => Err(err),
    }
  } else {
    return Err(ApiError::bad_request("invalid block identifier"));
  }
  .map_err(|e| ApiError::internal(format!("query: {e}")))?;

  let row = row.ok_or_else(|| ApiError::not_found("block not found"))?;
  let is_orphan = row.orphaned_at_epoch.is_some();
  let chainlock_snapshot = super::overlay::fetch_chainlock_snapshot(&state).await?;
  let live = super::overlay::block_live_from_snapshot(chainlock_snapshot.as_ref(), &row.hash)?;

  Ok(Json(BlockDetail {
    as_of_height: stage.height,
    height: row.height,
    hash: utils::view_hex(&row.hash, HashEndian::Ser),
    previousblockhash: utils::view_hex(&row.prev_hash, HashEndian::Ser),
    merkleroot: utils::view_hex(&row.merkle_root, HashEndian::Ser),
    time: rfc3339_time(i64::from(row.time))?,
    bits: format!("{:08x}", row.bits as u32),
    nonce: row.nonce,
    version: row.version,
    size: row.size,
    tx_count: row.tx_count,
    status: if is_orphan {
      "orphan".to_string()
    } else {
      "canonical".to_string()
    },
    tags: block_tags(is_orphan),
    live,
    orphaned_at: row.orphaned_at_epoch.map(rfc3339_time).transpose()?,
    chainwork: row.chainwork,
    cb_tx: row.cb_height.map(|height| CbTxFields {
      height,
      merkle_root_mn_list: row.cb_merkle_root.as_ref().map(hex::encode),
      merkle_flags: row.cb_merkle_flags,
      best_cl_height: row.cb_best_cl_height,
      best_cl_signature: row.cb_best_cl_sig.as_ref().map(hex::encode),
      credit_pool_balance: row.cb_credit_pool,
    }),
  }))
}

/// List transactions in a block.
///
/// # Errors
///
/// Returns `ApiError` when the block identifier is invalid, the block does
/// not exist, or a backing query fails.
#[utoipa::path(
  get,
  path = "/api/v1/block/{hashOrHeight}/txs",
  description = "Backed by the committed block and transaction indices. Returns canonical mined block transactions only from one committed snapshot.",
  params(
    ("hashOrHeight" = String, Path, description = "Block height or 64-char hash"),
    CursorParams,
  ),
  responses(
    (status = 200, description = "Block transactions", body = BlockTxListResponse),
    (status = 400, description = "Invalid block identifier", body = ErrorResponse),
    (status = 404, description = "Block not found", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Blocks"
)]
pub async fn get_block_txs(
  State(state): State<AppState>,
  Path(hash_or_height): Path<String>,
  Query(params): Query<CursorParams>,
) -> Result<Json<IndexedCursorPage<TxSummary>>, ApiError> {
  let stage = state
    .stage_status_typed(crate::index::Stage::Transaction)
    .await?;
  let height = resolve_block_height(&state.pool, &hash_or_height).await?;
  let params = params.validate(MAX_BLOCK_PAGE_LIMIT)?;
  let limit = params.limit;
  let cursor = decode_block_tx_cursor(params.cursor.as_deref(), stage.height, height)?;
  let as_of_height = cursor
    .as_ref()
    .map_or(stage.height, |cursor| cursor.as_of_height);
  let after = decode_block_tx_cursor_key(cursor.as_ref())?;

  let after_ref = after
    .as_ref()
    .map(|(block_pos, txid)| (*block_pos, txid.as_slice()));
  let mut rows = super::sql::block_txs(&state.pool, height, limit + 1, after_ref)
    .await
    .map_err(|e| ApiError::internal(format!("query: {e}")))?;
  let has_more = rows.len() as i64 > limit;
  rows.truncate(limit as usize);
  let next_cursor = if has_more {
    rows
      .last()
      .map(|row| {
        encode_cursor(&BlockTxCursor {
          as_of_height,
          block_height: height,
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
      block_height: height,
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
  let total_count = i64::from(
    super::sql::block_by_height(&state.pool, height)
      .await
      .map_err(|e| ApiError::internal(format!("block summary: {e}")))?
      .ok_or_else(|| ApiError::not_found("block not found"))?
      .tx_count,
  );
  Ok(Json(IndexedCursorPage {
    as_of_height,
    total_count,
    next_cursor,
    value,
  }))
}

fn decode_block_cursor(
  cursor: Option<&str>,
  current_height: i32,
) -> Result<Option<BlockCursor>, ApiError> {
  let Some(cursor) = decode_cursor::<BlockCursor>(cursor)? else {
    return Ok(None);
  };
  if cursor.as_of_height > current_height {
    return Err(ApiError::bad_request(
      "cursor snapshot is ahead of current request",
    ));
  }
  Ok(Some(cursor))
}

fn decode_block_tx_cursor(
  cursor: Option<&str>,
  current_height: i32,
  block_height: i32,
) -> Result<Option<BlockTxCursor>, ApiError> {
  let Some(cursor) = decode_cursor::<BlockTxCursor>(cursor)? else {
    return Ok(None);
  };
  if cursor.as_of_height > current_height || cursor.block_height != block_height {
    return Err(ApiError::bad_request(
      "cursor does not match current block request",
    ));
  }
  Ok(Some(cursor))
}

fn decode_block_tx_cursor_key(
  cursor: Option<&BlockTxCursor>,
) -> Result<Option<(i32, Vec<u8>)>, ApiError> {
  cursor
    .map(|cursor| {
      utils::decode_hex_to(&cursor.txid, HashEndian::Ser)
        .map(|txid| (cursor.block_pos, txid))
        .map_err(|_| ApiError::bad_request("invalid cursor txid"))
    })
    .transpose()
}

pub(crate) fn block_tags(orphan: bool) -> crate::rest::openapi::BlockTags {
  crate::rest::openapi::BlockTags { orphan }
}

pub async fn search_by_height(
  state: &AppState,
  height: i32,
) -> Result<Option<SearchResponse>, ApiError> {
  let hash = super::sql::block_hash_for_height(&state.pool, height)
    .await
    .map_err(|e| ApiError::from_daemon(e, "search block height"))?;
  let Some(hash) = hash else {
    return Ok(None);
  };
  Ok(Some(SearchResponse::Block(SearchBlockResult {
    block_hash: utils::view_hex(&hash, HashEndian::Ser),
    status: "canonical".to_string(),
    block_tags: block_tags(false),
  })))
}

pub async fn search_by_hash(
  state: &AppState,
  hash_bytes: &[u8],
) -> Result<Option<SearchResponse>, ApiError> {
  if super::sql::block_height_for_hash(&state.pool, hash_bytes)
    .await
    .map_err(|e| ApiError::from_daemon(e, "search block hash height"))?
    .is_some()
  {
    return Ok(Some(SearchResponse::Block(SearchBlockResult {
      block_hash: utils::view_hex(hash_bytes, HashEndian::Ser),
      status: "canonical".to_string(),
      block_tags: block_tags(false),
    })));
  }
  if super::sql::orphan_block_by_hash(&state.pool, hash_bytes)
    .await
    .map_err(|e| ApiError::from_daemon(e, "search orphan block hash"))?
    .is_some()
  {
    return Ok(Some(SearchResponse::Block(SearchBlockResult {
      block_hash: utils::view_hex(hash_bytes, HashEndian::Ser),
      status: "orphan".to_string(),
      block_tags: block_tags(true),
    })));
  }
  Ok(None)
}

pub(crate) async fn search_by_hash_prefix(
  state: &AppState,
  prefix: &str,
  limit: i64,
) -> Result<Vec<SearchResponse>, ApiError> {
  let rows = super::sql::search_blocks_by_prefix(&state.pool, prefix, limit)
    .await
    .map_err(|e| ApiError::from_daemon(e, "search block prefix"))?;
  Ok(
    rows
      .into_iter()
      .map(|row| {
        SearchResponse::Block(SearchBlockResult {
          block_hash: utils::view_hex(&row.hash, HashEndian::Ser),
          status: "canonical".to_owned(),
          block_tags: block_tags(false),
        })
      })
      .collect(),
  )
}

#[cfg(test)]
mod tests {
  use super::block_tags;

  #[test]
  fn block_tags_mark_orphans_without_chainlock() {
    let tags = block_tags(true);
    assert_eq!(tags, crate::rest::openapi::BlockTags { orphan: true });
  }

  #[test]
  fn canonical_block_tags_never_report_chainlock() {
    let tags = block_tags(false);
    assert_eq!(tags, crate::rest::openapi::BlockTags { orphan: false });
  }
}

/// Get the raw serialized block as a hex string.
///
/// # Errors
///
/// Returns `ApiError` when the block identifier is invalid, the block cannot
/// be resolved from PostgreSQL or RocksDB, or raw block storage is missing.
#[utoipa::path(
  get,
  path = "/api/v1/rawblock/{hashOrHeight}",
  description = "Backed by the block index plus local raw block storage. Returns serialized raw blocks from local storage rather than live Core passthrough RPC. This is a raw payload route, not a live overlay route.",
  params(("hashOrHeight" = String, Path, description = "Block height or 64-char hash")),
  responses(
    (status = 200, description = "Raw block hex", body = RawBlockResponse),
    (status = 400, description = "Invalid block identifier", body = ErrorResponse),
    (status = 404, description = "Block not found", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Blocks"
)]
pub async fn get_rawblock(
  State(state): State<AppState>,
  Path(hash_or_height): Path<String>,
) -> Result<Json<RawBlockResponse>, ApiError> {
  let stage = state.stage_status_typed(crate::index::Stage::Block).await?;
  let hash_bytes = resolve_block_hash(&state.pool, &hash_or_height).await?;

  let db = &state.db;
  let blocks_cf = db
    .cf_handle(crate::db::CF_BLOCKS)
    .ok_or_else(|| ApiError::internal("blocks CF missing"))?;

  let raw = db
    .get_cf(blocks_cf, &hash_bytes)
    .map_err(|e| ApiError::internal(format!("rocksdb: {e}")))?
    .ok_or_else(|| ApiError::not_found("raw block not in store"))?;

  Ok(Json(RawBlockResponse {
    as_of_height: stage.height,
    source: crate::rest::openapi::RawValueSource::Indexed,
    rawblock: hex::encode(&raw),
  }))
}

async fn resolve_block_height(
  pool: &crate::sql::SqlPool,
  hash_or_height: &str,
) -> Result<i32, ApiError> {
  if let Ok(height) = hash_or_height.parse::<i32>() {
    if height < 0 {
      return Err(ApiError::bad_request("block height must be non-negative"));
    }
    let exists = super::sql::block_exists(pool, height)
      .await
      .map_err(|e| ApiError::internal(format!("query: {e}")))?;
    if !exists {
      return Err(ApiError::not_found("block not found"));
    }
    return Ok(height);
  }
  if hash_or_height.len() == HASH_HEX_LEN {
    let hash_lookup = utils::HashLookup::from_hex(hash_or_height)
      .map_err(|_| ApiError::bad_request("invalid hash hex"))?;
    let (primary_hash, secondary_hash) = hash_lookup.ordered_candidates(HashEndian::Ser);
    let height = match super::sql::block_height_for_hash(pool, &primary_hash).await {
      Ok(Some(height)) => Some(height),
      Ok(None) => {
        if let Some(secondary_hash) = secondary_hash.as_ref() {
          super::sql::block_height_for_hash(pool, secondary_hash)
            .await
            .map_err(|e| ApiError::internal(format!("query: {e}")))?
        } else {
          None
        }
      }
      Err(e) => return Err(ApiError::internal(format!("query: {e}"))),
    }
    .ok_or_else(|| ApiError::not_found("block not found"))?;
    Ok(height)
  } else {
    Err(ApiError::bad_request("invalid block identifier"))
  }
}

async fn resolve_block_hash(
  pool: &crate::sql::SqlPool,
  hash_or_height: &str,
) -> Result<Vec<u8>, ApiError> {
  if let Ok(height) = hash_or_height.parse::<i32>() {
    if height < 0 {
      return Err(ApiError::bad_request("block height must be non-negative"));
    }
    let hash = super::sql::block_hash_for_height(pool, height)
      .await
      .map_err(|e| ApiError::internal(format!("query: {e}")))?
      .ok_or_else(|| ApiError::not_found("block not found"))?;
    Ok(hash)
  } else if hash_or_height.len() == HASH_HEX_LEN {
    let hash_lookup = utils::HashLookup::from_hex(hash_or_height)
      .map_err(|_| ApiError::bad_request("invalid hash hex"))?;
    let (primary_hash, secondary_hash) = hash_lookup.ordered_candidates(HashEndian::Ser);
    match super::sql::block_height_for_hash(pool, &primary_hash).await {
      Ok(Some(_)) => Ok(primary_hash),
      Ok(None) => {
        if let Some(secondary_hash) = secondary_hash {
          match super::sql::block_height_for_hash(pool, &secondary_hash).await {
            Ok(Some(_)) => Ok(secondary_hash),
            Ok(None) => Err(ApiError::not_found("block not found")),
            Err(e) => Err(ApiError::internal(format!("query: {e}"))),
          }
        } else {
          Err(ApiError::not_found("block not found"))
        }
      }
      Err(e) => Err(ApiError::internal(format!("query: {e}"))),
    }
  } else {
    Err(ApiError::bad_request("invalid block identifier"))
  }
}
