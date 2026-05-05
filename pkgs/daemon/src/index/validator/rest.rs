//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! REST endpoints for validators.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use utoipa::IntoParams;

use crate::rest::error::ApiError;
use crate::rest::openapi::{
  ErrorResponse, SearchResponse, SearchValidatorResult, ValidatorAddresses, ValidatorDetail,
  ValidatorHistoryEntry, ValidatorListResponse, ValidatorLive, ValidatorPose, ValidatorServices,
  ValidatorSummary,
};
use crate::rest::{
  decode_cursor, encode_cursor, rfc3339_time, AppState, IndexedCursorPage, OverlayFeature,
  DEFAULT_PAGINATION_LIMIT,
};
use crate::utils::{self, HashEndian};

const MAX_VALIDATOR_PAGE_LIMIT: i64 = 100;
const PRO_TX_HASH_HEX_LEN: usize = 64;

#[derive(Debug, Deserialize, IntoParams)]
#[serde(default)]
pub struct ListParams {
  #[param(minimum = 1, maximum = 100, default = 25)]
  pub limit: i64,
  pub cursor: Option<String>,
  /// Filter by status (e.g. "registered", "revoked").
  pub status: Option<String>,
  /// Filter by type (canonical values: "regular", "evonode").
  #[serde(rename = "type")]
  pub validator_type: Option<String>,
}

impl Default for ListParams {
  fn default() -> Self {
    Self {
      limit: DEFAULT_PAGINATION_LIMIT,
      cursor: None,
      status: None,
      validator_type: None,
    }
  }
}

impl ListParams {
  fn validate(mut self) -> Result<Self, ApiError> {
    if self.limit < 1 || self.limit > MAX_VALIDATOR_PAGE_LIMIT {
      return Err(ApiError::bad_request(format!(
        "limit must be between 1 and {MAX_VALIDATOR_PAGE_LIMIT}",
      )));
    }
    self.validator_type = self
      .validator_type
      .as_deref()
      .map(public_validator_type)
      .map(str::to_owned);
    Ok(self)
  }
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ValidatorCursor {
  as_of_height: i32,
  registered_height: i32,
  pro_tx_hash: String,
  status: Option<String>,
  validator_type: Option<String>,
}

/// List validators.
///
/// # Errors
///
/// Returns `ApiError` when the validator index is disabled or the backing
/// query fails.
#[utoipa::path(
  get,
  path = "/api/v1/validators",
  description = "Requires the optional validator index. Returns validator registry state materialized from indexed chain data. The nested live object is nullable, not snapshot-bound, may be ahead of asOfHeight, and may include live type, valid/status, pose, and last-paid overlay fields. Validator type is normalized for API responses.",
  params(ListParams),
  responses(
    (status = 200, description = "Paginated validator list", body = ValidatorListResponse),
    (status = 400, description = "Invalid pagination query", body = ErrorResponse),
    (status = 501, description = "Validator Index not enabled", body = ErrorResponse),
    (status = 503, description = "Validator Index not ready yet", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Validators"
)]
pub async fn list_validators(
  State(state): State<AppState>,
  Query(params): Query<ListParams>,
) -> Result<Json<IndexedCursorPage<ValidatorSummary>>, ApiError> {
  let params = params.validate()?;
  let stage = state
    .stage_status_typed(crate::index::Stage::Validator)
    .await?;
  if !state.indices.validator {
    return Err(ApiError::not_implemented("Validator Index is not enabled"));
  }

  let limit = params.limit;
  let cursor = decode_validator_cursor(params.cursor.as_deref(), stage.height, &params)?;
  let as_of_height = cursor
    .as_ref()
    .map_or(stage.height, |cursor| cursor.as_of_height);
  let before = decode_validator_cursor_key(cursor.as_ref())?;

  let before_ref = before
    .as_ref()
    .map(|(registered_height, pro_tx_hash)| (*registered_height, pro_tx_hash.as_slice()));
  let mut rows = super::sql::list_validators(
    &state.pool,
    params.status.as_deref(),
    params.validator_type.as_deref(),
    limit + 1,
    before_ref,
  )
  .await
  .map_err(|e| ApiError::internal(format!("query: {e}")))?;
  let has_more = rows.len() as i64 > limit;
  rows.truncate(limit as usize);
  let overlay = overlay_map(&state).await?;
  let live_times = overlay_time_map(&state, overlay.as_ref()).await?;

  let value: Vec<ValidatorSummary> = rows
    .iter()
    .map(|row| row_to_summary(row, overlay.as_ref(), &live_times))
    .collect();
  let total_count = super::sql::validator_count(
    &state.pool,
    params.status.as_deref(),
    params.validator_type.as_deref(),
  )
  .await
  .map_err(|e| ApiError::internal(format!("count: {e}")))?
  .total;
  Ok(Json(IndexedCursorPage {
    as_of_height,
    total_count,
    next_cursor: encode_validator_cursor(as_of_height, &params, rows.last(), has_more)?,
    value,
  }))
}

/// Get a validator by proTxHash.
///
/// # Errors
///
/// Returns `ApiError` when the validator index is disabled, the hash is
/// invalid, the validator does not exist, or a backing query fails.
#[utoipa::path(
  get,
  path = "/api/v1/validator/{proTxHash}",
  description = "Requires the optional validator index. Returns validator detail and special transaction history from the validator index. The nested live object is nullable, not snapshot-bound, may be ahead of asOfHeight, and may include live type, valid/status, pose, and last-paid overlay fields. Validator type is normalized for API responses.",
  params(("proTxHash" = String, Path, description = "ProRegTx hash (64 hex chars)")),
  responses(
    (status = 200, description = "Validator detail", body = ValidatorDetail),
    (status = 400, description = "Invalid hash", body = ErrorResponse),
    (status = 404, description = "Validator not found", body = ErrorResponse),
    (status = 501, description = "Validator Index not enabled", body = ErrorResponse),
    (status = 503, description = "Validator Index not ready yet", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Validators"
)]
pub async fn get_validator(
  State(state): State<AppState>,
  Path(pro_tx_hash_hex): Path<String>,
) -> Result<Json<ValidatorDetail>, ApiError> {
  let stage = state
    .stage_status_typed(crate::index::Stage::Validator)
    .await?;
  if !state.indices.validator {
    return Err(ApiError::not_implemented("Validator Index is not enabled"));
  }

  if pro_tx_hash_hex.len() != PRO_TX_HASH_HEX_LEN {
    return Err(ApiError::bad_request("proTxHash must be 64 hex characters"));
  }
  let pth_lookup = utils::HashLookup::from_hex(&pro_tx_hash_hex)
    .map_err(|_| ApiError::bad_request("invalid hex"))?;
  let (primary_hash, secondary_hash) = pth_lookup.ordered_candidates(HashEndian::Ser);

  let current_row = match super::sql::validator(&state.pool, &primary_hash).await {
    Ok(Some(row)) => Some(row),
    Ok(None) => {
      if let Some(secondary_hash) = secondary_hash.as_ref() {
        super::sql::validator(&state.pool, secondary_hash)
          .await
          .map_err(|e| ApiError::internal(format!("query: {e}")))?
      } else {
        None
      }
    }
    Err(e) => return Err(ApiError::internal(format!("query: {e}"))),
  };
  let reaped = current_row.is_none();
  let row = match current_row {
    Some(row) => row,
    None => match super::sql::historical_validator(&state.pool, &primary_hash).await {
      Ok(Some(row)) => row,
      Ok(None) => {
        if let Some(secondary_hash) = secondary_hash.as_ref() {
          super::sql::historical_validator(&state.pool, secondary_hash)
            .await
            .map_err(|e| ApiError::internal(format!("query: {e}")))?
            .ok_or_else(|| ApiError::not_found("validator not found"))?
        } else {
          return Err(ApiError::not_found("validator not found"));
        }
      }
      Err(e) => return Err(ApiError::internal(format!("query: {e}"))),
    },
  };

  let history_rows = super::sql::validator_history(&state.pool, &row.pro_tx_hash)
    .await
    .map_err(|e| ApiError::internal(format!("history: {e}")))?;
  let overlay = overlay_map(&state).await?;
  let overlay = overlay
    .as_ref()
    .and_then(|map| map.get(&utils::view_hex(&row.pro_tx_hash, HashEndian::Ser)));
  let live_times = overlay_time_map_for_one(&state, overlay).await?;

  let mut history: Vec<ValidatorHistoryEntry> = history_rows
    .iter()
    .map(|row| ValidatorHistoryEntry {
      txid: utils::view_hex(&row.txid, HashEndian::Ser),
      tx_type: row.tx_type.to_string(),
      payload: row.payload.clone(),
      block_height: row.block_height,
    })
    .collect();
  if reaped {
    if let Some(spend) = super::sql::validator_collateral_spend(
      &state.pool,
      &row.collateral_hash,
      row.collateral_index,
    )
    .await
    .map_err(|e| ApiError::internal(format!("collateral spend: {e}")))?
    {
      history.push(ValidatorHistoryEntry {
        txid: utils::view_hex(&spend.spent_txid, HashEndian::Ser),
        tx_type: "collateral_spent".to_owned(),
        payload: serde_json::json!({
          "collateralHash": utils::view_hex(&row.collateral_hash, HashEndian::Ser),
          "collateralIndex": row.collateral_index,
        }),
        block_height: spend.spent_height,
      });
    }
  }
  let addresses = row_addresses(&row);
  let services = row_services(&row);

  Ok(Json(ValidatorDetail {
    as_of_height: stage.height,
    pro_tx_hash: utils::view_hex(&row.pro_tx_hash, HashEndian::Ser),
    collateral_hash: utils::view_hex(&row.collateral_hash, HashEndian::Ser),
    collateral_index: row.collateral_index,
    addresses,
    live: overlay_to_live(overlay, &live_times),
    operator_pub_key: row.operator_pubkey.as_ref().map(hex::encode),
    operator_reward: row.operator_reward,
    validator_type: public_validator_type(&row.validator_type).to_owned(),
    status: if reaped {
      "spent".to_owned()
    } else {
      row.status
    },
    services,
    registered_height: row.registered_height,
    last_updated_height: row.last_updated_height,
    history,
  }))
}

fn row_to_summary(
  row: &super::sql::ValidatorRow,
  overlay: Option<&std::collections::HashMap<String, super::overlay::ValidatorOverlay>>,
  live_times: &std::collections::HashMap<i32, String>,
) -> ValidatorSummary {
  let overlay =
    overlay.and_then(|map| map.get(&utils::view_hex(&row.pro_tx_hash, HashEndian::Ser)));
  ValidatorSummary {
    pro_tx_hash: utils::view_hex(&row.pro_tx_hash, HashEndian::Ser),
    collateral_hash: utils::view_hex(&row.collateral_hash, HashEndian::Ser),
    collateral_index: row.collateral_index,
    addresses: row_addresses(row),
    live: overlay_to_live(overlay, live_times),
    operator_pub_key: row.operator_pubkey.as_ref().map(hex::encode),
    operator_reward: row.operator_reward,
    validator_type: public_validator_type(&row.validator_type).to_owned(),
    status: row.status.clone(),
    services: row_services(row),
    registered_height: row.registered_height,
    last_updated_height: row.last_updated_height,
  }
}

async fn overlay_map(
  state: &AppState,
) -> Result<Option<std::collections::HashMap<String, super::overlay::ValidatorOverlay>>, ApiError> {
  if !state.overlay_enabled(OverlayFeature::Protx) {
    return Ok(None);
  };
  super::overlay::fetch_overlay_map(&state.pool)
    .await
    .map(Some)
    .map_err(|e| ApiError::from_daemon(e, "read validator protx overlay"))
}

async fn overlay_time_map(
  state: &AppState,
  overlay: Option<&std::collections::HashMap<String, super::overlay::ValidatorOverlay>>,
) -> Result<std::collections::HashMap<i32, String>, ApiError> {
  let Some(overlay) = overlay else {
    return Ok(std::collections::HashMap::new());
  };
  let heights: Vec<i32> = overlay
    .values()
    .filter_map(|value| value.last_paid_height)
    .collect::<std::collections::BTreeSet<_>>()
    .into_iter()
    .collect();
  let block_times = super::sql::block_times_by_height(&state.pool, &heights)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query validator payout block times"))?;
  merge_raw_block_times(&state.db, heights, block_times)?
    .into_iter()
    .map(|(height, epoch)| rfc3339_time(epoch).map(|time| (height, time)))
    .collect()
}

async fn overlay_time_map_for_one(
  state: &AppState,
  overlay: Option<&super::overlay::ValidatorOverlay>,
) -> Result<std::collections::HashMap<i32, String>, ApiError> {
  let Some(height) = overlay.and_then(|value| value.last_paid_height) else {
    return Ok(std::collections::HashMap::new());
  };
  let block_times = super::sql::block_times_by_height(&state.pool, &[height])
    .await
    .map_err(|e| ApiError::from_daemon(e, "query validator payout block time"))?;
  merge_raw_block_times(&state.db, vec![height], block_times)?
    .into_iter()
    .map(|(height, epoch)| rfc3339_time(epoch).map(|time| (height, time)))
    .collect()
}

fn merge_raw_block_times(
  db: &std::sync::Arc<rocksdb::DB>,
  heights: Vec<i32>,
  mut block_times: std::collections::HashMap<i32, i64>,
) -> Result<std::collections::HashMap<i32, i64>, ApiError> {
  let missing_heights: Vec<i32> = heights
    .into_iter()
    .filter(|height| !block_times.contains_key(height))
    .collect();
  for height in missing_heights {
    let Some(raw_time) = raw_block_time_by_height(db, height)? else {
      continue;
    };
    block_times.insert(height, raw_time);
  }
  Ok(block_times)
}

fn raw_block_time_by_height(
  db: &std::sync::Arc<rocksdb::DB>,
  height: i32,
) -> Result<Option<i64>, ApiError> {
  let Ok(height_u32) = u32::try_from(height) else {
    return Ok(None);
  };
  let Some(hash) = crate::db::get_hash_for_height(db, height_u32) else {
    return Ok(None);
  };
  let blocks_cf = db
    .cf_handle(crate::db::CF_BLOCKS)
    .ok_or_else(|| ApiError::internal("blocks CF missing"))?;
  let Some(raw) = db
    .get_cf(blocks_cf, hash)
    .map_err(|e| ApiError::internal(format!("rocksdb: {e}")))?
  else {
    return Ok(None);
  };
  let block = crate::index::BlockCtx::from_raw(height_u32, raw.to_vec())
    .map_err(|e| ApiError::from_daemon(e, "decode validator payout raw block"))?;
  Ok(Some(i64::from(block.block().header.time)))
}

fn overlay_to_live(
  overlay: Option<&super::overlay::ValidatorOverlay>,
  live_times: &std::collections::HashMap<i32, String>,
) -> ValidatorLive {
  ValidatorLive {
    valid: overlay.and_then(|value| {
      value
        .current_status
        .as_deref()
        .map(|status| public_current_status(status) == "valid")
    }),
    status: overlay.and_then(|value| {
      value
        .current_status
        .as_deref()
        .map(public_current_status)
        .map(str::to_owned)
    }),
    last_paid_height: overlay.and_then(|value| value.last_paid_height),
    last_paid_at: overlay
      .and_then(|value| value.last_paid_height)
      .and_then(|height| live_times.get(&height).cloned()),
    consecutive_payments: overlay.and_then(|value| value.consecutive_payments),
    pose: overlay.and_then(|value| {
      let pose = ValidatorPose {
        ban_score: value.pose_penalty,
        revived_height: value.pose_revived_height,
        ban_height: value.pose_ban_height,
      };
      (pose.ban_score.is_some() || pose.revived_height.is_some() || pose.ban_height.is_some())
        .then_some(pose)
    }),
    revocation_reason: overlay.and_then(|value| value.revocation_reason),
  }
}

fn public_current_status(status: &str) -> &'static str {
  match status {
    "valid" => "valid",
    "registered" => "registered",
    "revoked" => "revoked",
    "pose_banned" => "banned",
    _ => "unknown",
  }
}

fn public_validator_type(value: &str) -> &'static str {
  let compact = value
    .chars()
    .filter(|ch| ch.is_ascii_alphanumeric())
    .flat_map(char::to_lowercase)
    .collect::<String>();
  match compact.as_str() {
    "regular" => "regular",
    "highperformance" | "evo" | "evonode" => "evonode",
    _ => "unknown",
  }
}

fn row_addresses(row: &super::sql::ValidatorRow) -> ValidatorAddresses {
  ValidatorAddresses {
    collateral: row.collateral_address.clone(),
    owner: row.owner_address.clone(),
    voting: row.voting_address.clone(),
    payout: row.payout_address.clone(),
    payee: row.operator_payout_address.clone(),
  }
}

fn row_services(row: &super::sql::ValidatorRow) -> ValidatorServices {
  ValidatorServices {
    core_p2p: row.addr_core_p2p.clone(),
    platform_p2p: row.addr_platform_p2p.clone(),
    platform_https: row.addr_platform_https.clone(),
  }
}

fn decode_validator_cursor(
  cursor: Option<&str>,
  as_of_height: i32,
  params: &ListParams,
) -> Result<Option<ValidatorCursor>, ApiError> {
  let Some(cursor) = decode_cursor::<ValidatorCursor>(cursor)? else {
    return Ok(None);
  };
  if cursor.as_of_height > as_of_height
    || cursor.status != params.status
    || cursor.validator_type != params.validator_type
  {
    return Err(ApiError::bad_request(
      "cursor does not match current validator filters",
    ));
  }
  Ok(Some(cursor))
}

fn encode_validator_cursor(
  as_of_height: i32,
  params: &ListParams,
  row: Option<&super::sql::ValidatorRow>,
  has_more: bool,
) -> Result<Option<String>, ApiError> {
  if !has_more {
    return Ok(None);
  }
  let Some(row) = row else {
    return Ok(None);
  };
  encode_cursor(&ValidatorCursor {
    as_of_height,
    registered_height: row.registered_height,
    pro_tx_hash: utils::view_hex(&row.pro_tx_hash, HashEndian::Ser),
    status: params.status.clone(),
    validator_type: params.validator_type.clone(),
  })
  .map(Some)
}

fn decode_validator_cursor_key(
  cursor: Option<&ValidatorCursor>,
) -> Result<Option<(i32, Vec<u8>)>, ApiError> {
  cursor
    .map(|cursor| {
      utils::decode_hex_to(&cursor.pro_tx_hash, HashEndian::Ser)
        .map(|pro_tx_hash| (cursor.registered_height, pro_tx_hash))
        .map_err(|_| ApiError::bad_request("invalid cursor proTxHash"))
    })
    .transpose()
}

pub async fn search_by_pro_tx_hash(
  state: &AppState,
  _query: &str,
  hash_bytes: &[u8],
) -> Result<Option<SearchResponse>, ApiError> {
  if !state.indices.validator {
    return Ok(None);
  }
  let row = super::sql::validator(&state.pool, hash_bytes)
    .await
    .map_err(|e| ApiError::from_daemon(e, "search validator"))?;
  Ok(row.map(|row| {
    SearchResponse::Validator(SearchValidatorResult {
      pro_tx_hash: utils::view_hex(&row.pro_tx_hash, HashEndian::Ser),
      status: row.status,
    })
  }))
}

pub(crate) async fn search_by_pro_tx_hash_prefix(
  state: &AppState,
  prefix: &str,
  limit: i64,
) -> Result<Vec<SearchResponse>, ApiError> {
  if !state.indices.validator {
    return Ok(Vec::new());
  }
  let rows = super::sql::search_validators_by_prefix(&state.pool, prefix, limit)
    .await
    .map_err(|e| ApiError::from_daemon(e, "search validator prefix"))?;
  Ok(
    rows
      .into_iter()
      .map(|row| {
        SearchResponse::Validator(SearchValidatorResult {
          pro_tx_hash: utils::view_hex(&row.pro_tx_hash, HashEndian::Ser),
          status: row.status,
        })
      })
      .collect(),
  )
}

#[cfg(test)]
mod tests {
  use super::{
    decode_validator_cursor, encode_validator_cursor, get_validator, overlay_time_map,
    public_validator_type, row_to_summary, ListParams,
  };
  use crate::utils;
  use crate::utils::HashEndian;
  use axum::extract::Path;
  use axum::extract::State;

  fn seed_raw_block(
    db: &rocksdb::DB,
    height: u32,
    raw: &[u8],
  ) -> Result<(), Box<dyn std::error::Error>> {
    let blocks_cf = db
      .cf_handle(crate::db::CF_BLOCKS)
      .ok_or("blocks CF missing")?;
    let heights_cf = db
      .cf_handle(crate::db::CF_HEIGHTS)
      .ok_or("heights CF missing")?;
    let meta_cf = db.cf_handle(crate::db::CF_META).ok_or("meta CF missing")?;
    let hash = crate::consensus::block_hash(raw)?.to_bytes();
    let mut batch = rocksdb::WriteBatch::default();
    batch.put_cf(blocks_cf, hash, raw);
    batch.put_cf(heights_cf, height.to_be_bytes(), hash);
    crate::db::put_tip_hash(&mut batch, meta_cf, &hash);
    crate::db::put_tip_height(&mut batch, meta_cf, height);
    db.write(batch)?;
    Ok(())
  }

  /// Build one representative validator insert because validator REST tests need a real indexed row.
  fn sample_validator_insert() -> crate::tx::ValidatorInsert {
    crate::tx::ValidatorInsert {
      pro_tx_hash: vec![0x11; 32],
      collateral_hash: vec![0x22; 32],
      collateral_index: 1,
      collateral_address: Some("Xcollateral111111111111111111111111".to_owned()),
      operator_pubkey: vec![0x33; 48],
      owner_address: Some("Xowner11111111111111111111111111111".to_owned()),
      voting_address: Some("Xvote111111111111111111111111111111".to_owned()),
      operator_reward: 5_000,
      validator_type: "regular".to_owned(),
      payout_addr: Some("Xpayout111111111111111111111111111".to_owned()),
      operator_payout_addr: Some("Xpayee1111111111111111111111111111".to_owned()),
      core_addrs: vec!["1.2.3.4:9999".to_owned()],
      plat_p2p_addrs: vec!["5.6.7.8:26656".to_owned()],
      plat_https_addrs: vec!["5.6.7.8:443".to_owned()],
    }
  }

  async fn seed_collateral_output(
    pool: &sqlx::PgPool,
    txid: &[u8],
    vout_index: i32,
    address: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size,
         value_out, input_count, output_count
       ) VALUES ($1, 0, 0, 1, 'normal', 0, 100, 1000, 1, 1)",
    )
    .bind(txid)
    .execute(pool)
    .await?;
    sqlx::query(
      "INSERT INTO tx_outputs (
         txid, vout_index, block_height, value, script_pubkey, address, address_type
       ) VALUES ($1, $2, 0, 1000, $3, $4, 'p2pkh')",
    )
    .bind(txid)
    .bind(vout_index)
    .bind(vec![0x76_u8, 0xa9, 0x14])
    .bind(address)
    .execute(pool)
    .await?;
    Ok(())
  }

  async fn seed_transaction(
    pool: &sqlx::PgPool,
    txid: &[u8],
    block_height: i32,
    block_pos: i32,
    tx_type: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size,
         value_out, input_count, output_count
       ) VALUES ($1, $2, $3, 3, $4::tx_type, 0, 100, 1000, 1, 1)",
    )
    .bind(txid)
    .bind(block_height)
    .bind(block_pos)
    .bind(tx_type)
    .execute(pool)
    .await?;
    Ok(())
  }

  async fn seed_validator_payload(
    pool: &sqlx::PgPool,
    txid: &[u8],
    pro_tx_hash: &[u8],
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO special_tx_payloads (txid, tx_type, pro_tx_hash, payload)
       VALUES ($1, 'provider_register', $2, $3)",
    )
    .bind(txid)
    .bind(pro_tx_hash)
    .bind(serde_json::json!({ "version": 1 }))
    .execute(pool)
    .await?;
    Ok(())
  }

  #[test]
  fn validator_cursor_preserves_filter_values() {
    let params = ListParams {
      limit: 25,
      cursor: None,
      status: Some("registered & ready".to_owned()),
      validator_type: Some("evonode".to_owned()),
    };
    let row = crate::index::validator::sql::ValidatorRow {
      pro_tx_hash: vec![0x11; 32],
      collateral_hash: vec![0x22; 32],
      collateral_index: 7,
      collateral_address: None,
      owner_address: None,
      voting_address: None,
      operator_pubkey: None,
      operator_reward: 0,
      validator_type: "HighPerformance".to_owned(),
      status: "registered & ready".to_owned(),
      addr_core_p2p: Vec::new(),
      addr_platform_p2p: Vec::new(),
      addr_platform_https: Vec::new(),
      payout_address: None,
      operator_payout_address: Some("Xpayee1111111111111111111111111111".to_owned()),
      registered_height: 123,
      last_updated_height: 123,
    };

    let next_cursor = encode_validator_cursor(42, &params, Some(&row), true);
    assert!(next_cursor.is_ok());
    let decoded = decode_validator_cursor(
      next_cursor.as_ref().ok().and_then(|value| value.as_deref()),
      42,
      &params,
    );
    let decoded = decoded.ok().flatten().expect("cursor should decode");
    assert_eq!(decoded.as_of_height, 42);
    assert_eq!(decoded.registered_height, 123);
    assert_eq!(decoded.pro_tx_hash, "11".repeat(32));
    let summary = row_to_summary(&row, None, &std::collections::HashMap::new());
    assert_eq!(summary.live.last_paid_height, None);
    assert_eq!(summary.validator_type, "evonode");
  }

  #[test]
  fn row_to_summary_normalizes_live_status_and_type() {
    let row = crate::index::validator::sql::ValidatorRow {
      pro_tx_hash: vec![0x11; 32],
      collateral_hash: vec![0x22; 32],
      collateral_index: 7,
      collateral_address: None,
      owner_address: None,
      voting_address: None,
      operator_pubkey: None,
      operator_reward: 0,
      validator_type: "HighPerformance".to_owned(),
      status: "registered".to_owned(),
      addr_core_p2p: Vec::new(),
      addr_platform_p2p: Vec::new(),
      addr_platform_https: Vec::new(),
      payout_address: None,
      operator_payout_address: Some("Xpayee1111111111111111111111111111".to_owned()),
      registered_height: 123,
      last_updated_height: 123,
    };
    let overlay = std::collections::HashMap::from([(
      "11".repeat(32),
      crate::index::validator::overlay::ValidatorOverlay {
        current_type: Some("Evo".to_owned()),
        current_status: Some("valid".to_owned()),
        last_paid_height: Some(99),
        consecutive_payments: None,
        pose_penalty: None,
        pose_revived_height: None,
        pose_ban_height: None,
        revocation_reason: None,
      },
    )]);

    let live_times = std::collections::HashMap::from([(99, "1970-01-01T00:01:39Z".to_owned())]);
    let summary = row_to_summary(&row, Some(&overlay), &live_times);
    assert_eq!(summary.status, "registered");
    assert_eq!(summary.validator_type, "evonode");
    assert_eq!(
      summary.addresses.payee.as_deref(),
      Some("Xpayee1111111111111111111111111111")
    );
    assert_eq!(summary.live.valid, Some(true));
    assert_eq!(summary.live.status.as_deref(), Some("valid"));
    assert_eq!(summary.live.last_paid_height, Some(99));
    assert_eq!(
      summary.live.last_paid_at.as_deref(),
      Some("1970-01-01T00:01:39Z")
    );
  }

  #[test]
  fn public_validator_type_normalizes_known_variants() {
    assert_eq!(public_validator_type("regular"), "regular");
    assert_eq!(public_validator_type("Regular"), "regular");
    assert_eq!(public_validator_type("HighPerformance"), "evonode");
    assert_eq!(public_validator_type("Evo"), "evonode");
    assert_eq!(public_validator_type("evonode"), "evonode");
  }

  #[tokio::test]
  async fn overlay_time_map_falls_back_to_raw_store_when_pg_lags(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let state = crate::tests::app_state_with_pool(
      pool,
      crate::rest::LiveCapabilities {
        mempool: false,
        islock: false,
        network: false,
        governance: false,
        stream: false,
      },
      crate::rest::OverlayCapabilities {
        protx: true,
        chainlock: false,
      },
      0,
    );
    let raw = crate::tests::minimal_block_bytes(7);
    let block = crate::index::BlockCtx::from_raw(0, raw.clone())?;
    seed_raw_block(&state.db, 0, &raw)?;

    let overlay = std::collections::HashMap::from([(
      "11".repeat(32),
      crate::index::validator::overlay::ValidatorOverlay {
        current_type: Some("Evo".to_owned()),
        current_status: Some("valid".to_owned()),
        last_paid_height: Some(0),
        consecutive_payments: None,
        pose_penalty: None,
        pose_revived_height: None,
        pose_ban_height: None,
        revocation_reason: None,
      },
    )]);

    let times = overlay_time_map(&state, Some(&overlay))
      .await
      .expect("overlay time map should read raw store fallback");
    let expected_time = crate::rest::rfc3339_time(i64::from(block.block().header.time))
      .expect("raw block time should format");
    assert_eq!(
      times.get(&0).map(String::as_str),
      Some(expected_time.as_str()),
    );
    Ok(())
  }

  #[tokio::test]
  async fn get_validator_returns_reaped_detail_with_collateral_spend_history(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let insert = sample_validator_insert();
    let spend_txid = vec![0x44; 32];
    seed_collateral_output(
      &pool,
      &insert.collateral_hash,
      insert.collateral_index,
      "Xcollateral111111111111111111111111",
    )
    .await?;
    seed_transaction(&pool, &insert.pro_tx_hash, 10, 0, "provider_register").await?;
    seed_validator_payload(&pool, &insert.pro_tx_hash, &insert.pro_tx_hash).await?;
    seed_transaction(&pool, &spend_txid, 11, 0, "normal").await?;

    let mut conn = pool.acquire().await?;
    crate::index::validator::sql::insert_validator(&mut conn, insert.clone(), 10, 0).await?;
    sqlx::query(
      "UPDATE tx_outputs
          SET spent_txid = $1, spent_vin = 0, spent_height = 11
        WHERE txid = $2 AND vout_index = $3",
    )
    .bind(&spend_txid)
    .bind(&insert.collateral_hash)
    .bind(insert.collateral_index)
    .execute(&pool)
    .await?;
    crate::index::validator::sql::reap_spent_collateral(
      &mut conn,
      &insert.collateral_hash,
      insert.collateral_index,
      11,
      0,
    )
    .await?;

    let mut state = crate::tests::app_state_with_pool(
      pool,
      crate::rest::LiveCapabilities {
        mempool: false,
        islock: false,
        network: false,
        governance: false,
        stream: false,
      },
      crate::rest::OverlayCapabilities {
        protx: false,
        chainlock: false,
      },
      0,
    );
    state.indices.validator = true;

    let response = get_validator(
      State(state),
      Path(utils::view_hex(&insert.pro_tx_hash, HashEndian::Ser)),
    )
    .await
    .expect("reaped validator detail should resolve");
    let detail = response.0;

    assert_eq!(detail.status, "spent");
    let terminal = detail
      .history
      .last()
      .expect("synthetic collateral spend history should be appended");
    assert_eq!(terminal.tx_type, "collateral_spent");
    assert_eq!(terminal.txid, utils::view_hex(&spend_txid, HashEndian::Ser),);
    assert_eq!(terminal.block_height, 11);
    assert_eq!(
      terminal.payload,
      serde_json::json!({
        "collateralHash": utils::view_hex(&insert.collateral_hash, HashEndian::Ser),
        "collateralIndex": insert.collateral_index,
      }),
    );
    Ok(())
  }
}
