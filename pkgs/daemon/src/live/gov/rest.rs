//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! REST endpoints for live governance proposal state.

use axum::extract::{Path, Query, State};
use axum::Json;
use serde::{Deserialize, Serialize};
use serde_json::Value;

use crate::rest::error::ApiError;
use crate::rest::openapi::{
  ErrorResponse, ProposalDetail, ProposalListResponse, ProposalVote, ProposalVoteListResponse,
  SearchProposalResult, SearchResponse,
};
use crate::rest::{
  decode_cursor, encode_cursor, rfc3339_time, AppState, CursorPage, CursorParams, LiveFeature,
};

const BASE_UNITS_PER_COIN: i64 = 100_000_000;

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProposalCursor {
  updated_at: i64,
  object_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
struct ProposalVoteCursor {
  proposal_hash: String,
  updated_at: i64,
  vote_hash: String,
}

#[derive(Debug, Clone, Copy)]
struct GovernanceNetworkStats {
  total_nodes: i64,
  enabled_nodes: i64,
}

/// Get current governance proposals.
///
/// # Errors
///
/// Returns `ApiError` when the live governance capability is disabled, a
/// backing query fails, or a stored proposal payload cannot be decoded.
#[utoipa::path(
  get,
  path = "/api/v1/proposals",
  description = "Requires the live governance capability. Returns the current live governance object set and is not backed by a historical governance index.",
  params(CursorParams),
  responses(
    (status = 200, description = "Governance proposals", body = ProposalListResponse),
    (status = 400, description = "Invalid pagination query", body = ErrorResponse),
    (status = 501, description = "Governance feature disabled", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Governance"
)]
pub async fn list_proposals(
  State(state): State<AppState>,
  Query(params): Query<CursorParams>,
) -> Result<Json<CursorPage<ProposalDetail>>, ApiError> {
  state.check_live_feature(LiveFeature::Governance)?;
  let params = params.validate(100)?;
  let limit = params.limit;
  let cursor = decode_proposal_cursor(params.cursor.as_deref())?;
  let after = cursor
    .as_ref()
    .map(|cursor| {
      hex::decode(&cursor.object_hash)
        .map(|object_hash| (cursor.updated_at, object_hash))
        .map_err(|_| ApiError::bad_request("invalid cursor object hash"))
    })
    .transpose()?;
  let after_ref = after
    .as_ref()
    .map(|(updated_at, object_hash)| (*updated_at, object_hash.as_slice()));
  let network = governance_network_stats(&state).await?;
  let mut rows = crate::sql::live::gov::objects(&state.pool, limit + 1, after_ref)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query proposals"))?;
  let has_more = rows.len() as i64 > limit;
  rows.truncate(limit as usize);
  let value = rows
    .iter()
    .map(|row| proposal_row(row, network))
    .collect::<Result<Vec<_>, _>>()?;
  let total_count = crate::sql::live::gov::governance_summary(&state.pool)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query proposal summary"))?
    .proposal_total;
  Ok(Json(CursorPage {
    total_count,
    next_cursor: if has_more {
      rows
        .last()
        .map(|row| {
          encode_cursor(&ProposalCursor {
            updated_at: row.updated_at,
            object_hash: hex::encode(&row.object_hash),
          })
        })
        .transpose()?
    } else {
      None
    },
    value,
  }))
}

/// Get one governance proposal by hash.
///
/// # Errors
///
/// Returns `ApiError` when the live governance capability is disabled, the
/// hash is invalid, the proposal does not exist, or its stored payload cannot
/// be decoded.
#[utoipa::path(
  get,
  path = "/api/v1/proposal/{hash}",
  description = "Requires the live governance capability. Returns the current live proposal record, not a historical indexed governance timeline.",
  params(("hash" = String, Path, description = "Governance proposal hash (64 hex chars)")),
  responses(
    (status = 200, description = "Governance proposal detail", body = ProposalDetail),
    (status = 400, description = "Invalid proposal hash", body = ErrorResponse),
    (status = 404, description = "Proposal not found", body = ErrorResponse),
    (status = 501, description = "Governance feature disabled", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Governance"
)]
pub async fn get_proposal(
  State(state): State<AppState>,
  Path(hash): Path<String>,
) -> Result<Json<ProposalDetail>, ApiError> {
  state.check_live_feature(LiveFeature::Governance)?;
  let hash_bytes = decode_hash(&hash)?;
  let network = governance_network_stats(&state).await?;
  let row = crate::sql::live::gov::object(&state.pool, &hash_bytes)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query proposal"))?
    .ok_or_else(|| ApiError::not_found("proposal not found"))?;
  Ok(Json(proposal_row(&row, network)?))
}

/// Get current votes for one governance proposal.
///
/// # Errors
///
/// Returns `ApiError` when the live governance capability is disabled, the
/// hash is invalid, a backing query fails, or a vote timestamp cannot be
/// normalized.
#[utoipa::path(
  get,
  path = "/api/v1/proposal/{hash}/votes",
  description = "Requires the live governance capability. Returns the current live vote set for one proposal and is not backed by a historical governance index.",
  params(
    ("hash" = String, Path, description = "Governance proposal hash (64 hex chars)"),
    CursorParams
  ),
  responses(
    (status = 200, description = "Governance proposal votes", body = ProposalVoteListResponse),
    (status = 400, description = "Invalid proposal hash", body = ErrorResponse),
    (status = 501, description = "Governance feature disabled", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Governance"
)]
pub async fn get_votes(
  State(state): State<AppState>,
  Path(hash): Path<String>,
  Query(params): Query<CursorParams>,
) -> Result<Json<CursorPage<ProposalVote>>, ApiError> {
  state.check_live_feature(LiveFeature::Governance)?;
  let hash_bytes = decode_hash(&hash)?;
  let params = params.validate(100)?;
  let limit = params.limit;
  let cursor = decode_proposal_vote_cursor(params.cursor.as_deref(), &hash)?;
  let after = cursor
    .as_ref()
    .map(|cursor| {
      hex::decode(&cursor.vote_hash)
        .map(|vote_hash| (cursor.updated_at, vote_hash))
        .map_err(|_| ApiError::bad_request("invalid cursor vote hash"))
    })
    .transpose()?;
  let after_ref = after
    .as_ref()
    .map(|(updated_at, vote_hash)| (*updated_at, vote_hash.as_slice()));
  let mut rows = crate::sql::live::gov::votes(&state.pool, &hash_bytes, limit + 1, after_ref)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query proposal votes"))?;
  let has_more = rows.len() as i64 > limit;
  rows.truncate(limit as usize);
  let value = rows.iter().map(vote_row).collect::<Result<Vec<_>, _>>()?;
  let total_count = match crate::sql::live::gov::object(&state.pool, &hash_bytes)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query proposal"))?
  {
    Some(row) => {
      let proposal = proposal_row(&row, governance_network_stats(&state).await?)?;
      proposal.yes_count + proposal.no_count + proposal.abstain_count
    }
    None => 0,
  };
  Ok(Json(CursorPage {
    total_count,
    next_cursor: if has_more {
      rows
        .last()
        .map(|row| {
          encode_cursor(&ProposalVoteCursor {
            proposal_hash: hash.clone(),
            updated_at: row.updated_at,
            vote_hash: hex::encode(&row.vote_hash),
          })
        })
        .transpose()?
    } else {
      None
    },
    value,
  }))
}

fn decode_proposal_cursor(cursor: Option<&str>) -> Result<Option<ProposalCursor>, ApiError> {
  decode_cursor::<ProposalCursor>(cursor)
}

fn decode_proposal_vote_cursor(
  cursor: Option<&str>,
  proposal_hash: &str,
) -> Result<Option<ProposalVoteCursor>, ApiError> {
  let Some(cursor) = decode_cursor::<ProposalVoteCursor>(cursor)? else {
    return Ok(None);
  };
  if cursor.proposal_hash != proposal_hash {
    return Err(ApiError::bad_request(
      "cursor does not match current proposal request",
    ));
  }
  Ok(Some(cursor))
}

fn decode_hash(hash: &str) -> Result<Vec<u8>, ApiError> {
  if hash.len() != 64 {
    return Err(ApiError::bad_request(
      "proposal hash must be 64 hex characters",
    ));
  }
  hex::decode(hash).map_err(|_| ApiError::bad_request("invalid proposal hash hex"))
}

fn proposal_row(
  row: &crate::sql::live::gov::GovernanceObjectRow,
  network: GovernanceNetworkStats,
) -> Result<ProposalDetail, ApiError> {
  let raw: Value = if let Some(value) = row.data_json.clone() {
    value
  } else {
    serde_json::from_slice(&row.raw_object)
      .map_err(|e| ApiError::internal(format!("decode stored proposal: {e}")))?
  };
  let proposal = parse_proposal_payload(&raw)?;
  let yes_count = raw.get("YesCount").and_then(Value::as_i64).unwrap_or(0);
  let no_count = raw.get("NoCount").and_then(Value::as_i64).unwrap_or(0);
  let abstain_count = raw.get("AbstainCount").and_then(Value::as_i64).unwrap_or(0);
  let vote_count = yes_count + no_count + abstain_count;
  Ok(ProposalDetail {
    hash: hex::encode(&row.object_hash),
    collateral_hash: raw
      .get("CollateralHash")
      .and_then(Value::as_str)
      .map(ToOwned::to_owned),
    name: proposal_string(&proposal, &["name"]).unwrap_or_default(),
    url: proposal_string(&proposal, &["url"]),
    payment_address: proposal_string(&proposal, &["payment_address", "paymentAddress"]),
    payment_amount: proposal_value(&proposal, &["payment_amount", "paymentAmount"])
      .map(parse_amount_value)
      .transpose()?
      .unwrap_or(0),
    start_time: proposal_value(&proposal, &["start_epoch", "startEpoch"])
      .and_then(Value::as_i64)
      .map(rfc3339_time)
      .transpose()?,
    end_time: proposal_value(&proposal, &["end_epoch", "endEpoch"])
      .and_then(Value::as_i64)
      .map(rfc3339_time)
      .transpose()?,
    yes_count,
    no_count,
    abstain_count,
    vote_count,
    total_nodes: network.total_nodes,
    enabled_nodes: network.enabled_nodes,
    participating_nodes: vote_count,
    funding_threshold: funding_threshold(network.enabled_nodes),
    funded: raw
      .get("fCachedFunding")
      .and_then(Value::as_bool)
      .unwrap_or(false),
    valid: raw
      .get("fCachedValid")
      .and_then(Value::as_bool)
      .unwrap_or(false),
    creation_time: raw
      .get("CreationTime")
      .and_then(Value::as_i64)
      .map(rfc3339_time)
      .transpose()?,
  })
}

async fn governance_network_stats(state: &AppState) -> Result<GovernanceNetworkStats, ApiError> {
  let summary = crate::sql::live::protx::summary(&state.pool)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query validator protx summary"))?;
  Ok(GovernanceNetworkStats {
    total_nodes: summary.total_nodes,
    enabled_nodes: summary.enabled_nodes,
  })
}

fn funding_threshold(enabled_nodes: i64) -> i64 {
  if enabled_nodes <= 0 {
    return 0;
  }
  (enabled_nodes + 9) / 10
}

fn parse_proposal_payload(raw: &Value) -> Result<Value, ApiError> {
  if let Some(value) = raw.get("DataString").and_then(Value::as_str) {
    return serde_json::from_str(value)
      .map_err(|e| ApiError::internal(format!("decode proposal payload: {e}")));
  }
  if let Some(value) = raw.get("DataHex").and_then(Value::as_str) {
    let decoded = hex::decode(value)
      .map_err(|e| ApiError::internal(format!("decode proposal payload hex: {e}")))?;
    return serde_json::from_slice(&decoded)
      .map_err(|e| ApiError::internal(format!("decode proposal payload from hex: {e}")));
  }
  Err(ApiError::internal("stored proposal missing DataString"))
}

fn proposal_value<'a>(proposal: &'a Value, keys: &[&str]) -> Option<&'a Value> {
  keys.iter().find_map(|key| proposal.get(*key))
}

fn proposal_string(proposal: &Value, keys: &[&str]) -> Option<String> {
  proposal_value(proposal, keys)
    .and_then(Value::as_str)
    .map(ToOwned::to_owned)
}

fn parse_amount_value(value: &Value) -> Result<i64, ApiError> {
  match value {
    Value::Number(number) => parse_amount_str(&number.to_string()),
    Value::String(value) => parse_amount_str(value),
    _ => Err(ApiError::internal(
      "proposal payment_amount must be numeric",
    )),
  }
}

fn parse_amount_str(value: &str) -> Result<i64, ApiError> {
  let trimmed = value.trim();
  if trimmed.is_empty() {
    return Err(ApiError::internal(
      "proposal payment_amount must not be empty",
    ));
  }

  let negative = trimmed.starts_with('-');
  let digits = if negative || trimmed.starts_with('+') {
    &trimmed[1..]
  } else {
    trimmed
  };

  let (whole, frac) = digits.split_once('.').unwrap_or((digits, ""));
  if whole.is_empty() && frac.is_empty() {
    return Err(ApiError::internal(
      "proposal payment_amount must contain digits",
    ));
  }
  if !whole.chars().all(|ch| ch.is_ascii_digit()) || !frac.chars().all(|ch| ch.is_ascii_digit()) {
    return Err(ApiError::internal(
      "proposal payment_amount must be decimal digits",
    ));
  }
  if frac.len() > 8 {
    return Err(ApiError::internal(
      "proposal payment_amount exceeds 8 decimal places",
    ));
  }

  let whole_units = if whole.is_empty() {
    0
  } else {
    whole
      .parse::<i64>()
      .map_err(|e| ApiError::internal(format!("parse proposal payment_amount whole units: {e}")))?
  };
  let frac_padded = format!("{frac:0<8}");
  let frac_units = if frac_padded.is_empty() {
    0
  } else {
    frac_padded.parse::<i64>().map_err(|e| {
      ApiError::internal(format!(
        "parse proposal payment_amount fractional units: {e}"
      ))
    })?
  };

  let scaled = whole_units
    .checked_mul(BASE_UNITS_PER_COIN)
    .and_then(|whole_scaled| whole_scaled.checked_add(frac_units))
    .ok_or_else(|| ApiError::internal("proposal payment_amount exceeds i64 range"))?;

  if negative {
    scaled
      .checked_neg()
      .ok_or_else(|| ApiError::internal("proposal payment_amount exceeds i64 range"))
  } else {
    Ok(scaled)
  }
}

fn vote_row(row: &crate::sql::live::gov::GovernanceVoteRow) -> Result<ProposalVote, ApiError> {
  let raw = std::str::from_utf8(&row.raw_vote)
    .map_err(|e| ApiError::internal(format!("decode stored vote: {e}")))?;
  let parts: Vec<&str> = raw.split(':').collect();
  let pro_tx_hash = parts
    .first()
    .and_then(|part| part.split('-').next())
    .unwrap_or_default();
  let vote = parts.get(2).copied().unwrap_or_default();
  Ok(ProposalVote {
    pro_tx_hash: pro_tx_hash.to_owned(),
    vote: vote.to_owned(),
    vote_time: row.vote_time.map(rfc3339_time).transpose()?,
  })
}

pub(crate) async fn search_by_hash(
  state: &AppState,
  _query: &str,
  hash_bytes: &[u8],
) -> Result<Option<SearchResponse>, ApiError> {
  if !state.live.governance {
    return Ok(None);
  }
  Ok(
    crate::sql::live::gov::object(&state.pool, hash_bytes)
      .await
      .map_err(|e| ApiError::from_daemon(e, "search proposal"))?
      .map(|row| {
        SearchResponse::Proposal(SearchProposalResult {
          id: hex::encode(row.object_hash),
        })
      }),
  )
}

pub(crate) async fn search_by_query(
  state: &AppState,
  query: &str,
  limit: i64,
) -> Result<Vec<SearchResponse>, ApiError> {
  if !state.live.governance {
    return Ok(Vec::new());
  }
  let rows = crate::sql::live::gov::search_objects_by_query(&state.pool, query, limit)
    .await
    .map_err(|e| ApiError::from_daemon(e, "search proposal suggestions"))?;
  Ok(
    rows
      .into_iter()
      .map(|row| {
        SearchResponse::Proposal(SearchProposalResult {
          id: hex::encode(row.object_hash),
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

  use axum::extract::{Path, Query, State};
  use axum::http::StatusCode;
  use rocksdb::DB;

  use super::{get_proposal, get_votes, list_proposals};
  use crate::rest::{
    test_support::test_state, AppState, CursorParams, IndexCapabilities, LiveCapabilities,
    OverlayCapabilities,
  };

  #[tokio::test]
  async fn list_proposals_rejects_disabled_feature() {
    let err = list_proposals(State(test_state()), Query(CursorParams::default()))
      .await
      .expect_err("disabled governance feature should fail");
    assert_eq!(err.status, StatusCode::NOT_IMPLEMENTED);
  }

  #[tokio::test]
  async fn get_proposal_rejects_disabled_feature() {
    let err = get_proposal(State(test_state()), Path("11".repeat(32)))
      .await
      .expect_err("disabled governance feature should fail");
    assert_eq!(err.status, StatusCode::NOT_IMPLEMENTED);
  }

  #[tokio::test]
  async fn get_votes_rejects_disabled_feature() {
    let err = get_votes(
      State(test_state()),
      Path("11".repeat(32)),
      Query(CursorParams::default()),
    )
    .await
    .expect_err("disabled governance feature should fail");
    assert_eq!(err.status, StatusCode::NOT_IMPLEMENTED);
  }

  #[tokio::test]
  async fn governance_handlers_return_persisted_live_rows_when_pg_is_configured(
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

    let object_hash = vec![0x11; 32];
    let mut tx = pool.begin().await?;
    crate::sql::live::gov::upsert_object(
      &mut tx,
      &crate::sql::live::gov::GovernanceObjectRow {
        object_hash: object_hash.clone(),
        parent_hash: None,
        revision: Some(1),
        object_time: Some(100),
        object_type: 1,
        data_json: Some(serde_json::json!({
          "CollateralHash": "22",
          "YesCount": 5,
          "NoCount": 1,
          "AbstainCount": 2,
          "fCachedFunding": true,
          "fCachedValid": true,
          "CreationTime": 123,
          "DataString": "{\"name\":\"prop\",\"url\":\"https://example.com\",\"payment_address\":\"Xpay\",\"payment_amount\":12.5,\"start_epoch\":10,\"end_epoch\":20}"
        })),
        raw_object: br#"{
          "CollateralHash": "22",
          "YesCount": 5,
          "NoCount": 1,
          "AbstainCount": 2,
          "fCachedFunding": true,
          "fCachedValid": true,
          "CreationTime": 123,
          "DataString": "{\"name\":\"prop\",\"url\":\"https://example.com\",\"payment_address\":\"Xpay\",\"payment_amount\":12.5,\"start_epoch\":10,\"end_epoch\":20}"
        }"#
        .to_vec(),
        updated_at: 77,
      },
    )
    .await?;
    crate::sql::live::gov::replace_votes_for_object(
      &mut tx,
      &object_hash,
      &[crate::sql::live::gov::GovernanceVoteRow {
        vote_hash: vec![0x33; 32],
        object_hash: object_hash.clone(),
        signal: Some(1),
        outcome: Some(1),
        vote_time: Some(456),
        raw_vote: b"abcd1234-0:funding:yes".to_vec(),
        updated_at: 88,
      }],
    )
    .await?;
    crate::sql::live::protx::replace_snapshot(
      &mut tx,
      &[
        crate::sql::live::protx::ValidatorProTxSnapshotRow {
          pro_tx_hash: vec![0x44; 32],
          validator_type: Some("Evo".to_owned()),
          current_status: Some("valid".to_owned()),
          last_paid_height: None,
          consecutive_payments: None,
          pose_penalty: None,
          pose_revived_height: None,
          pose_ban_height: None,
          revocation_reason: None,
          updated_at: 90,
        },
        crate::sql::live::protx::ValidatorProTxSnapshotRow {
          pro_tx_hash: vec![0x55; 32],
          validator_type: Some("Regular".to_owned()),
          current_status: Some("registered".to_owned()),
          last_paid_height: None,
          consecutive_payments: None,
          pose_penalty: None,
          pose_revived_height: None,
          pose_ban_height: None,
          revocation_reason: None,
          updated_at: 90,
        },
      ],
    )
    .await?;
    tx.commit().await?;

    let state = test_state_with_pool(pool, true);
    let list = list_proposals(State(state.clone()), Query(CursorParams::default()))
      .await
      .expect("proposal list should resolve")
      .0;
    assert_eq!(list.total_count, 1);
    assert_eq!(list.value.len(), 1);
    assert!(list.next_cursor.is_none());
    assert_eq!(list.value[0].hash, hex::encode(&object_hash));
    assert_eq!(list.value[0].name, "prop");
    assert_eq!(list.value[0].yes_count, 5);
    assert_eq!(list.value[0].vote_count, 8);
    assert_eq!(list.value[0].total_nodes, 2);
    assert_eq!(list.value[0].enabled_nodes, 1);
    assert_eq!(list.value[0].participating_nodes, 8);
    assert_eq!(list.value[0].funding_threshold, 1);
    assert_eq!(list.value[0].payment_amount, 1_250_000_000);
    assert_eq!(
      list.value[0].start_time.as_deref(),
      Some("1970-01-01T00:00:10Z")
    );
    assert_eq!(
      list.value[0].end_time.as_deref(),
      Some("1970-01-01T00:00:20Z")
    );

    let detail = get_proposal(State(state.clone()), Path(hex::encode(&object_hash)))
      .await
      .expect("proposal detail should resolve")
      .0;
    assert_eq!(detail.payment_address.as_deref(), Some("Xpay"));
    assert!(detail.funded);
    assert_eq!(detail.vote_count, 8);
    assert_eq!(
      detail.creation_time.as_deref(),
      Some("1970-01-01T00:02:03Z")
    );

    let votes = get_votes(
      State(state),
      Path(hex::encode(&object_hash)),
      Query(CursorParams::default()),
    )
    .await
    .expect("proposal votes should resolve")
    .0;
    assert_eq!(votes.total_count, 8);
    assert_eq!(votes.value.len(), 1);
    assert!(votes.next_cursor.is_none());
    assert_eq!(votes.value[0].pro_tx_hash, "abcd1234");
    assert_eq!(votes.value[0].vote, "yes");
    assert_eq!(
      votes.value[0].vote_time.as_deref(),
      Some("1970-01-01T00:07:36Z")
    );
    Ok(())
  }

  #[test]
  fn parse_amount_str_scales_base_units_exactly() {
    assert_eq!(super::parse_amount_str("12.5").ok(), Some(1_250_000_000));
    assert_eq!(super::parse_amount_str("0.00000001").ok(), Some(1));
    assert!(super::parse_amount_str("0.000000001").is_err());
  }

  #[test]
  fn proposal_row_accepts_data_hex_and_camel_case_aliases() {
    let payload = serde_json::json!({
      "name": "prop",
      "paymentAddress": "Xpay",
      "paymentAmount": "12.5",
      "startEpoch": 10,
      "endEpoch": 20
    });
    let row = crate::sql::live::gov::GovernanceObjectRow {
      object_hash: vec![0x11; 32],
      parent_hash: None,
      revision: None,
      object_time: None,
      object_type: 1,
      data_json: Some(serde_json::json!({
        "CollateralHash": "22",
        "YesCount": 5,
        "NoCount": 1,
        "AbstainCount": 2,
        "fCachedFunding": true,
        "fCachedValid": true,
        "CreationTime": 123,
        "DataHex": hex::encode(serde_json::to_vec(&payload).expect("payload should encode"))
      })),
      raw_object: Vec::new(),
      updated_at: 77,
    };

    let detail = super::proposal_row(
      &row,
      super::GovernanceNetworkStats {
        total_nodes: 12,
        enabled_nodes: 10,
      },
    )
    .expect("proposal row should decode");
    assert_eq!(detail.name, "prop");
    assert_eq!(detail.payment_address.as_deref(), Some("Xpay"));
    assert_eq!(detail.payment_amount, 1_250_000_000);
    assert_eq!(detail.start_time.as_deref(), Some("1970-01-01T00:00:10Z"));
    assert_eq!(detail.end_time.as_deref(), Some("1970-01-01T00:00:20Z"));
    assert_eq!(detail.vote_count, 8);
    assert_eq!(detail.total_nodes, 12);
    assert_eq!(detail.enabled_nodes, 10);
    assert_eq!(detail.participating_nodes, 8);
    assert_eq!(detail.funding_threshold, 1);
  }

  fn test_state_with_pool(pool: crate::sql::SqlPool, live_governance: bool) -> AppState {
    let unique = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_nanos())
      .unwrap_or_default();
    let db_path = std::env::temp_dir().join(format!("hyphen-gov-test-{unique}"));
    let db = DB::open_default(&db_path).expect("rocksdb should open");
    let _ = fs::remove_dir_all(&db_path);

    AppState {
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
        mempool: false,
        islock: false,
        network: false,
        governance: live_governance,
        stream: false,
      },
      overlays: OverlayCapabilities {
        protx: false,
        chainlock: false,
      },
      recent_mempool_txs: 0,
      stream_hub: None,
      stream_heartbeat_secs: 1,
    }
  }
}
