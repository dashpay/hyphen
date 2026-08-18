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
use crate::rest::hydb::{hydb_err, hydb_hash, hydb_height, tx_type_str};
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
  let before = cursor
    .as_ref()
    .map(|cursor| hyphen_db::HyDbCursor(cursor.height.to_string()));

  let page = state
    .hydb()?
    .blocks()
    .list(limit as u32, before)
    .await
    .map_err(|e| hydb_err(e, "list blocks"))?;
  let next_cursor = if page.next_cursor.is_some() {
    page
      .items
      .last()
      .map(|row| {
        encode_cursor(&BlockCursor {
          as_of_height,
          height: row.height.0 as i32,
        })
      })
      .transpose()?
  } else {
    None
  };

  let chainlock_snapshot = super::overlay::fetch_chainlock_snapshot(&state);
  let value: Vec<BlockSummary> = page
    .items
    .iter()
    .map(|row| {
      Ok(BlockSummary {
        height: row.height.0 as i32,
        hash: utils::view_hex(row.hash.as_bytes(), HashEndian::Ser),
        previousblockhash: utils::view_hex(row.prev_hash.as_bytes(), HashEndian::Ser),
        time: rfc3339_time(i64::from(row.time))?,
        bits: format!("{:08x}", row.bits as u32),
        nonce: row.nonce,
        version: row.version,
        size: row.size,
        tx_count: row.tx_count,
        tags: block_tags(false),
        live: super::overlay::block_live_from_snapshot(
          chainlock_snapshot.as_deref(),
          row.hash.as_bytes(),
        )?,
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
  let blocks = state.hydb()?.blocks();

  if let Ok(height) = hash_or_height.parse::<i32>() {
    if height < 0 {
      return Err(ApiError::bad_request("block height must be non-negative"));
    }
    let detail = blocks
      .by_height(hydb_height(height))
      .await
      .map_err(|e| hydb_err(e, "block by height"))?
      .ok_or_else(|| ApiError::not_found("block not found"))?;
    return Ok(Json(canonical_block_detail(&state, stage.height, detail)?));
  }

  if hash_or_height.len() != HASH_HEX_LEN {
    return Err(ApiError::bad_request("invalid block identifier"));
  }
  let hash_lookup = utils::HashLookup::from_hex(&hash_or_height)
    .map_err(|_| ApiError::bad_request("invalid hash hex"))?;
  let (primary_hash, secondary_hash) = hash_lookup.ordered_candidates(HashEndian::Ser);
  let candidates: Vec<Vec<u8>> = [Some(primary_hash), secondary_hash]
    .into_iter()
    .flatten()
    .collect();

  // Canonical hit first.
  for candidate in &candidates {
    if let Some(detail) = blocks
      .by_hash(hydb_hash(candidate)?)
      .await
      .map_err(|e| hydb_err(e, "block by hash"))?
    {
      return Ok(Json(canonical_block_detail(&state, stage.height, detail)?));
    }
  }
  // Otherwise an orphan the node once relayed — decode detail from raw bytes.
  for candidate in &candidates {
    if let Some(topo) = blocks
      .topology(hydb_hash(candidate)?)
      .await
      .map_err(|e| hydb_err(e, "block topology"))?
    {
      let raw = blocks
        .raw_bytes(topo.hash)
        .await
        .map_err(|e| hydb_err(e, "raw orphan block"))?
        .ok_or_else(|| ApiError::not_found("block not found"))?;
      return Ok(Json(orphan_block_detail(
        &state,
        stage.height,
        &topo,
        &raw,
      )?));
    }
  }
  Err(ApiError::not_found("block not found"))
}

/// Assemble the block-detail DTO for a canonical block.
fn canonical_block_detail(
  state: &AppState,
  as_of_height: i32,
  row: hyphen_db::query::HyDbQrBlockDetail,
) -> Result<BlockDetail, ApiError> {
  let chainlock_snapshot = super::overlay::fetch_chainlock_snapshot(state);
  let live =
    super::overlay::block_live_from_snapshot(chainlock_snapshot.as_deref(), row.hash.as_bytes())?;
  Ok(BlockDetail {
    as_of_height,
    height: row.height.0 as i32,
    hash: utils::view_hex(row.hash.as_bytes(), HashEndian::Ser),
    previousblockhash: utils::view_hex(row.prev_hash.as_bytes(), HashEndian::Ser),
    merkleroot: utils::view_hex(row.merkle_root.as_bytes(), HashEndian::Ser),
    time: rfc3339_time(i64::from(row.time))?,
    bits: format!("{:08x}", row.bits as u32),
    nonce: row.nonce,
    version: row.version,
    size: row.size,
    tx_count: row.tx_count,
    status: "canonical".to_string(),
    tags: block_tags(false),
    live,
    orphaned_at: None,
    chainwork: row.chainwork,
    cb_tx: row.cb_height.map(|height| CbTxFields {
      height,
      merkle_root_mn_list: row.cb_merkle_root.as_ref().map(hex::encode),
      merkle_flags: row.cb_merkle_flags,
      best_cl_height: row.cb_best_cl_height,
      best_cl_signature: row.cb_best_cl_sig.as_ref().map(hex::encode),
      credit_pool_balance: row.cb_credit_pool,
    }),
  })
}

/// Assemble the block-detail DTO for an orphan block by decoding its raw bytes.
/// The new schema keeps no derived rows for orphans, so `orphaned_at` and
/// `chainwork` are unavailable here.
fn orphan_block_detail(
  state: &AppState,
  as_of_height: i32,
  topo: &hyphen_db::query::HyDbQrBlockRef,
  raw: &[u8],
) -> Result<BlockDetail, ApiError> {
  let ctx = crate::index::block_ctx::BlockCtx::from_raw(topo.height.0, raw.to_vec())
    .map_err(|e| ApiError::internal(format!("decode orphan block: {e}")))?;
  let block = ctx.block();
  let header = &block.header;
  let cb = crate::index::block::util::extract_cbtx_fields(block, topo.height.0);
  let hash = ctx.block_hash();
  let bits = i32::try_from(header.bits).map_err(|_| ApiError::internal("block bits exceed i32"))?;
  let time =
    i32::try_from(header.time).map_err(|_| ApiError::internal("block time exceeds i32"))?;
  let chainlock_snapshot = super::overlay::fetch_chainlock_snapshot(state);
  let live = super::overlay::block_live_from_snapshot(chainlock_snapshot.as_deref(), &hash)?;
  Ok(BlockDetail {
    as_of_height,
    height: topo.height.0 as i32,
    hash: utils::view_hex(&hash, HashEndian::Ser),
    previousblockhash: utils::view_hex(&header.prev_hash.to_bytes(), HashEndian::Ser),
    merkleroot: utils::view_hex(&header.merkle_root.to_bytes(), HashEndian::Ser),
    time: rfc3339_time(i64::from(time))?,
    bits: format!("{:08x}", bits as u32),
    nonce: i64::from(header.nonce),
    version: header.version,
    size: raw.len() as i32,
    tx_count: block.transactions.len() as i32,
    status: "orphan".to_string(),
    tags: block_tags(true),
    live,
    orphaned_at: None,
    chainwork: None,
    cb_tx: cb.height.map(|height| CbTxFields {
      height,
      merkle_root_mn_list: cb.merkle_root.as_ref().map(hex::encode),
      merkle_flags: cb.merkle_flags,
      best_cl_height: cb.best_cl_height,
      best_cl_signature: cb.best_cl_sig.as_ref().map(hex::encode),
      credit_pool_balance: cb.credit_pool,
    }),
  })
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
  let height = resolve_block_height(&state, &hash_or_height).await?;
  let params = params.validate(MAX_BLOCK_PAGE_LIMIT)?;
  let limit = params.limit;
  let cursor = decode_block_tx_cursor(params.cursor.as_deref(), stage.height, height)?;
  let as_of_height = cursor
    .as_ref()
    .map_or(stage.height, |cursor| cursor.as_of_height);
  let after = decode_block_tx_cursor_key(cursor.as_ref())?;
  let after_cursor = after
    .as_ref()
    .map(|(block_pos, txid)| hyphen_db::HyDbCursor(format!("{block_pos}:{}", hex::encode(txid))));

  let blocks = state.hydb()?.blocks();
  let page = blocks
    .txs(hydb_height(height), limit as u32, after_cursor)
    .await
    .map_err(|e| hydb_err(e, "block txs"))?;
  let next_cursor = if page.next_cursor.is_some() {
    page
      .items
      .last()
      .map(|row| {
        encode_cursor(&BlockTxCursor {
          as_of_height,
          block_height: height,
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
      block_height: height,
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
  let total_count = i64::from(
    blocks
      .by_height(hydb_height(height))
      .await
      .map_err(|e| hydb_err(e, "block summary"))?
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
  if height < 0 {
    return Ok(None);
  }
  let hash = state
    .hydb()?
    .blocks()
    .hash_for_height(hydb_height(height))
    .await
    .map_err(|e| hydb_err(e, "search block height"))?;
  let Some(hash) = hash else {
    return Ok(None);
  };
  Ok(Some(SearchResponse::Block(SearchBlockResult {
    block_hash: utils::view_hex(hash.as_bytes(), HashEndian::Ser),
    status: "canonical".to_string(),
    block_tags: block_tags(false),
  })))
}

pub async fn search_by_hash(
  state: &AppState,
  hash_bytes: &[u8],
) -> Result<Option<SearchResponse>, ApiError> {
  let blocks = state.hydb()?.blocks();
  let hash = hydb_hash(hash_bytes)?;
  if blocks
    .height_for_hash(hash)
    .await
    .map_err(|e| hydb_err(e, "search block hash height"))?
    .is_some()
  {
    return Ok(Some(SearchResponse::Block(SearchBlockResult {
      block_hash: utils::view_hex(hash_bytes, HashEndian::Ser),
      status: "canonical".to_string(),
      block_tags: block_tags(false),
    })));
  }
  if blocks
    .topology(hash)
    .await
    .map_err(|e| hydb_err(e, "search orphan block hash"))?
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
  let rows = state
    .hydb()?
    .blocks()
    .search_prefix(prefix.to_owned(), limit.max(0) as u32)
    .await
    .map_err(|e| hydb_err(e, "search block prefix"))?;
  Ok(
    rows
      .into_iter()
      .map(|row| {
        SearchResponse::Block(SearchBlockResult {
          block_hash: utils::view_hex(row.hash.as_bytes(), HashEndian::Ser),
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
/// be resolved, or raw block storage is missing.
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
  let hash_bytes = resolve_block_hash(&state, &hash_or_height).await?;

  let raw = state
    .hydb()?
    .blocks()
    .raw_bytes(hydb_hash(&hash_bytes)?)
    .await
    .map_err(|e| hydb_err(e, "raw block"))?
    .ok_or_else(|| ApiError::not_found("raw block not in store"))?;

  Ok(Json(RawBlockResponse {
    as_of_height: stage.height,
    source: crate::rest::openapi::RawValueSource::Indexed,
    rawblock: hex::encode(&raw),
  }))
}

async fn resolve_block_height(state: &AppState, hash_or_height: &str) -> Result<i32, ApiError> {
  let blocks = state.hydb()?.blocks();
  if let Ok(height) = hash_or_height.parse::<i32>() {
    if height < 0 {
      return Err(ApiError::bad_request("block height must be non-negative"));
    }
    let exists = blocks
      .exists(hydb_height(height))
      .await
      .map_err(|e| hydb_err(e, "block exists"))?;
    if !exists {
      return Err(ApiError::not_found("block not found"));
    }
    return Ok(height);
  }
  if hash_or_height.len() == HASH_HEX_LEN {
    let hash_lookup = utils::HashLookup::from_hex(hash_or_height)
      .map_err(|_| ApiError::bad_request("invalid hash hex"))?;
    let (primary_hash, secondary_hash) = hash_lookup.ordered_candidates(HashEndian::Ser);
    for candidate in [Some(primary_hash), secondary_hash].into_iter().flatten() {
      if let Some(height) = blocks
        .height_for_hash(hydb_hash(&candidate)?)
        .await
        .map_err(|e| hydb_err(e, "block height for hash"))?
      {
        return Ok(height.0 as i32);
      }
    }
    Err(ApiError::not_found("block not found"))
  } else {
    Err(ApiError::bad_request("invalid block identifier"))
  }
}

async fn resolve_block_hash(state: &AppState, hash_or_height: &str) -> Result<Vec<u8>, ApiError> {
  let blocks = state.hydb()?.blocks();
  if let Ok(height) = hash_or_height.parse::<i32>() {
    if height < 0 {
      return Err(ApiError::bad_request("block height must be non-negative"));
    }
    let hash = blocks
      .hash_for_height(hydb_height(height))
      .await
      .map_err(|e| hydb_err(e, "block hash for height"))?
      .ok_or_else(|| ApiError::not_found("block not found"))?;
    Ok(hash.as_bytes().to_vec())
  } else if hash_or_height.len() == HASH_HEX_LEN {
    let hash_lookup = utils::HashLookup::from_hex(hash_or_height)
      .map_err(|_| ApiError::bad_request("invalid hash hex"))?;
    let (primary_hash, secondary_hash) = hash_lookup.ordered_candidates(HashEndian::Ser);
    for candidate in [Some(primary_hash), secondary_hash].into_iter().flatten() {
      if blocks
        .height_for_hash(hydb_hash(&candidate)?)
        .await
        .map_err(|e| hydb_err(e, "block height for hash"))?
        .is_some()
      {
        return Ok(candidate);
      }
    }
    Err(ApiError::not_found("block not found"))
  } else {
    Err(ApiError::bad_request("invalid block identifier"))
  }
}
