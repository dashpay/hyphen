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
use crate::rest::hydb::{hydb_err, hydb_hash, hydb_height, tx_type_str};
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

/// Convert a normalized public validator-type filter into the storage enum.
fn hydb_vl_type_filter(value: &str) -> Option<hyphen_db::HyDbVlType> {
  use hyphen_db::HyDbVlType::{Evonode, Regular, Unknown};
  match value {
    "regular" => Some(Regular),
    "evonode" => Some(Evonode),
    "unknown" => Some(Unknown),
    _ => None,
  }
}

/// Public string label for a stored validator type.
fn vl_type_str(validator_type: hyphen_db::HyDbVlType) -> &'static str {
  use hyphen_db::HyDbVlType::{Evonode, Regular, Unknown};
  match validator_type {
    Regular => "regular",
    Evonode => "evonode",
    Unknown => "unknown",
  }
}

/// Sum the precomputed validator counts matching the active status/type filters.
async fn validator_total(
  validators: &dyn hyphen_db::query::HyDbVlIdx,
  status: Option<&str>,
  validator_type: Option<hyphen_db::HyDbVlType>,
) -> Result<i64, ApiError> {
  let counts = validators
    .counts()
    .await
    .map_err(|e| hydb_err(e, "validator counts"))?;
  Ok(
    counts
      .iter()
      .filter(|row| status.is_none_or(|s| row.status == s))
      .filter(|row| validator_type.is_none_or(|t| row.validator_type == t))
      .map(|row| row.total)
      .sum(),
  )
}

/// Resolve a payout block height to its epoch seconds via the storage engine.
async fn payout_block_epoch(state: &AppState, height: i32) -> Result<Option<i64>, ApiError> {
  if height < 0 {
    return Ok(None);
  }
  Ok(
    state
      .hydb()?
      .blocks()
      .by_height(hydb_height(height))
      .await
      .map_err(|e| hydb_err(e, "validator payout block time"))?
      .map(|block| i64::from(block.time)),
  )
}

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
    if let Some(raw) = self.validator_type.as_deref() {
      let normalized = public_validator_type(raw);
      // "unknown" is a real stored bucket a caller may filter on, but an
      // unrecognized spelling must be a 400 — silently normalizing it to
      // "unknown" would answer a question the caller did not ask.
      let compact = raw
        .chars()
        .filter(|ch| ch.is_ascii_alphanumeric())
        .flat_map(char::to_lowercase)
        .collect::<String>();
      if normalized == "unknown" && compact != "unknown" {
        return Err(ApiError::bad_request(format!(
          "unrecognized validator type filter: {raw}"
        )));
      }
      self.validator_type = Some(normalized.to_owned());
    }
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

  let before_cursor = before.as_ref().map(|(registered_height, pro_tx_hash)| {
    hyphen_db::HyDbCursor(format!("{registered_height}:{}", hex::encode(pro_tx_hash)))
  });
  let vl_type = params
    .validator_type
    .as_deref()
    .and_then(hydb_vl_type_filter);
  let validators = state.hydb()?.validators();
  let page = validators
    .list(params.status.clone(), vl_type, limit as u32, before_cursor)
    .await
    .map_err(|e| hydb_err(e, "list validators"))?;
  let overlay = overlay_map(&state).await?;
  let live_times = overlay_time_map(&state, overlay.as_ref()).await?;

  let value: Vec<ValidatorSummary> = page
    .items
    .iter()
    .map(|row| row_to_summary(row, overlay.as_ref(), &live_times))
    .collect();
  let total_count = validator_total(validators, params.status.as_deref(), vl_type).await?;
  let next_cursor = if page.next_cursor.is_some() {
    page
      .items
      .last()
      .map(|row| {
        encode_cursor(&ValidatorCursor {
          as_of_height,
          registered_height: row.registered_height.get() as i32,
          pro_tx_hash: utils::view_hex(row.protx_hash.as_bytes(), HashEndian::Ser),
          status: params.status.clone(),
          validator_type: params.validator_type.clone(),
        })
      })
      .transpose()?
  } else {
    None
  };
  Ok(Json(IndexedCursorPage {
    as_of_height,
    total_count,
    next_cursor,
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

  let validators = state.hydb()?.validators();
  let mut row = validators
    .get(hydb_hash(&primary_hash)?)
    .await
    .map_err(|e| hydb_err(e, "validator"))?;
  if row.is_none() {
    if let Some(secondary_hash) = secondary_hash.as_ref() {
      row = validators
        .get(hydb_hash(secondary_hash)?)
        .await
        .map_err(|e| hydb_err(e, "validator"))?;
    }
  }
  // Reaped (collateral-spent) validators are dropped from the registry; their
  // historical/collateral-spend detail returns with the validator reap model.
  let row = row.ok_or_else(|| ApiError::not_found("validator not found"))?;

  let history_rows = validators
    .history(row.protx_hash)
    .await
    .map_err(|e| hydb_err(e, "validator history"))?;
  let overlay = overlay_map(&state).await?;
  let overlay = overlay
    .as_ref()
    .and_then(|map| map.get(&utils::view_hex(row.protx_hash.as_bytes(), HashEndian::Ser)));
  let live_times = overlay_time_map_for_one(&state, overlay).await?;

  let history: Vec<ValidatorHistoryEntry> = history_rows
    .iter()
    .map(|entry| ValidatorHistoryEntry {
      txid: utils::view_hex(entry.txid.as_bytes(), HashEndian::Ser),
      tx_type: tx_type_str(entry.tx_type).to_owned(),
      payload: entry.payload.clone(),
      block_height: entry.block_height.get() as i32,
    })
    .collect();
  let addresses = row_addresses(&row);
  let services = row_services(&row);

  Ok(Json(ValidatorDetail {
    as_of_height: stage.height,
    pro_tx_hash: utils::view_hex(row.protx_hash.as_bytes(), HashEndian::Ser),
    collateral_hash: utils::view_hex(row.collateral_hash.as_bytes(), HashEndian::Ser),
    collateral_index: row.collateral_index,
    addresses,
    live: overlay_to_live(overlay, &live_times),
    operator_pub_key: row.operator_pubkey.as_ref().map(hex::encode),
    operator_reward: row.operator_reward,
    validator_type: vl_type_str(row.validator_type).to_owned(),
    status: row.status.clone(),
    services,
    registered_height: row.registered_height.get() as i32,
    last_updated_height: row.last_updated_height.get() as i32,
    history,
  }))
}

fn row_to_summary(
  row: &hyphen_db::query::HyDbQrValidator,
  overlay: Option<&std::collections::HashMap<String, super::overlay::ValidatorOverlay>>,
  live_times: &std::collections::HashMap<i32, String>,
) -> ValidatorSummary {
  let overlay =
    overlay.and_then(|map| map.get(&utils::view_hex(row.protx_hash.as_bytes(), HashEndian::Ser)));
  ValidatorSummary {
    pro_tx_hash: utils::view_hex(row.protx_hash.as_bytes(), HashEndian::Ser),
    collateral_hash: utils::view_hex(row.collateral_hash.as_bytes(), HashEndian::Ser),
    collateral_index: row.collateral_index,
    addresses: row_addresses(row),
    live: overlay_to_live(overlay, live_times),
    operator_pub_key: row.operator_pubkey.as_ref().map(hex::encode),
    operator_reward: row.operator_reward,
    validator_type: vl_type_str(row.validator_type).to_owned(),
    status: row.status.clone(),
    services: row_services(row),
    registered_height: row.registered_height.get() as i32,
    last_updated_height: row.last_updated_height.get() as i32,
  }
}

async fn overlay_map(
  state: &AppState,
) -> Result<Option<std::collections::HashMap<String, super::overlay::ValidatorOverlay>>, ApiError> {
  if !state.overlay_enabled(OverlayFeature::Protx) {
    return Ok(None);
  };
  Ok(Some(super::overlay::fetch_overlay_map(&state.live_state)))
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
  let mut times = std::collections::HashMap::new();
  for height in heights {
    if let Some(epoch) = payout_block_epoch(state, height).await? {
      times.insert(height, rfc3339_time(epoch)?);
    }
  }
  Ok(times)
}

async fn overlay_time_map_for_one(
  state: &AppState,
  overlay: Option<&super::overlay::ValidatorOverlay>,
) -> Result<std::collections::HashMap<i32, String>, ApiError> {
  let Some(height) = overlay.and_then(|value| value.last_paid_height) else {
    return Ok(std::collections::HashMap::new());
  };
  let mut times = std::collections::HashMap::new();
  if let Some(epoch) = payout_block_epoch(state, height).await? {
    times.insert(height, rfc3339_time(epoch)?);
  }
  Ok(times)
}

fn overlay_to_live(
  overlay: Option<&super::overlay::ValidatorOverlay>,
  live_times: &std::collections::HashMap<i32, String>,
) -> ValidatorLive {
  ValidatorLive {
    validator_type: overlay.and_then(|value| value.current_type.clone()),
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

fn row_addresses(row: &hyphen_db::query::HyDbQrValidator) -> ValidatorAddresses {
  ValidatorAddresses {
    collateral: row
      .collateral_address
      .as_ref()
      .map(|address| address.0.clone()),
    owner: row.owner_address.as_ref().map(|address| address.0.clone()),
    voting: row.voting_address.as_ref().map(|address| address.0.clone()),
    payout: row.payout_address.as_ref().map(|address| address.0.clone()),
    payee: row
      .operator_payout_address
      .as_ref()
      .map(|address| address.0.clone()),
  }
}

fn row_services(row: &hyphen_db::query::HyDbQrValidator) -> ValidatorServices {
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
  let row = state
    .hydb()?
    .validators()
    .get(hydb_hash(hash_bytes)?)
    .await
    .map_err(|e| hydb_err(e, "search validator"))?;
  Ok(row.map(|row| {
    SearchResponse::Validator(SearchValidatorResult {
      pro_tx_hash: utils::view_hex(row.protx_hash.as_bytes(), HashEndian::Ser),
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
  let validators = state.hydb()?.validators();
  let hashes = validators
    .search_prefix(prefix.to_owned(), limit.max(0) as u32)
    .await
    .map_err(|e| hydb_err(e, "search validator prefix"))?;
  // `search_prefix` returns bare hashes; resolve each to recover its status.
  // The prefix limit is small, and this consolidates into `HyDbSearch` later.
  let mut results = Vec::with_capacity(hashes.len());
  for hash in hashes {
    if let Some(row) = validators
      .get(hash)
      .await
      .map_err(|e| hydb_err(e, "search validator prefix detail"))?
    {
      results.push(SearchResponse::Validator(SearchValidatorResult {
        pro_tx_hash: utils::view_hex(row.protx_hash.as_bytes(), HashEndian::Ser),
        status: row.status,
      }));
    }
  }
  Ok(results)
}

#[cfg(test)]
mod tests {
  use super::{decode_validator_cursor, public_validator_type, row_to_summary, ListParams};

  fn sample_validator(
    validator_type: hyphen_db::HyDbVlType,
    status: &str,
  ) -> hyphen_db::query::HyDbQrValidator {
    hyphen_db::query::HyDbQrValidator {
      protx_hash: hyphen_db::HyDbHash([0x11; 32]),
      collateral_hash: hyphen_db::HyDbHash([0x22; 32]),
      collateral_index: 7,
      collateral_address: None,
      owner_address: None,
      voting_address: None,
      operator_pubkey: None,
      addr_core_p2p: Vec::new(),
      addr_platform_p2p: Vec::new(),
      addr_platform_https: Vec::new(),
      payout_address: None,
      operator_payout_address: Some(hyphen_db::HyDbAddress(
        "Xpayee1111111111111111111111111111".to_owned(),
      )),
      operator_reward: 0,
      validator_type,
      status: status.to_owned(),
      registered_height: hyphen_db::HyDbHeight::new(123),
      last_updated_height: hyphen_db::HyDbHeight::new(123),
    }
  }

  #[test]
  fn validator_cursor_preserves_filter_values() {
    let params = ListParams {
      limit: 25,
      cursor: None,
      status: Some("registered & ready".to_owned()),
      validator_type: Some("evonode".to_owned()),
    };
    let cursor = super::ValidatorCursor {
      as_of_height: 42,
      registered_height: 123,
      pro_tx_hash: "11".repeat(32),
      status: params.status.clone(),
      validator_type: params.validator_type.clone(),
    };
    let encoded = crate::rest::encode_cursor(&cursor).expect("cursor should encode");
    let decoded = decode_validator_cursor(Some(encoded.as_str()), 42, &params)
      .ok()
      .flatten()
      .expect("cursor should decode");
    assert_eq!(decoded.as_of_height, 42);
    assert_eq!(decoded.registered_height, 123);
    assert_eq!(decoded.pro_tx_hash, "11".repeat(32));

    let summary = row_to_summary(
      &sample_validator(hyphen_db::HyDbVlType::Evonode, "registered & ready"),
      None,
      &std::collections::HashMap::new(),
    );
    assert_eq!(summary.live.last_paid_height, None);
    assert_eq!(summary.validator_type, "evonode");
  }

  #[test]
  fn row_to_summary_normalizes_live_status_and_type() {
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

    let summary = row_to_summary(
      &sample_validator(hyphen_db::HyDbVlType::Evonode, "registered"),
      Some(&overlay),
      &live_times,
    );
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
}
