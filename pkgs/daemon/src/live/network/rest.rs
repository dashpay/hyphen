//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! REST endpoint for live network state.

use axum::extract::State;
use axum::Json;
use serde_json::Value;

use crate::rest::error::ApiError;
use crate::rest::openapi::{
  ErrorResponse, NetworkInfoResponse, NetworkResponse, NetworkValidatorClassSummary,
  NetworkValidatorSummary, PeerInfoResponse,
};
use crate::rest::{rfc3339_time, AppState, LiveFeature};

/// Get the current network snapshot.
///
/// # Errors
///
/// Returns `ApiError` when the live network capability is disabled, no
/// snapshot has been persisted yet, a backing query fails, or the timestamp
/// cannot be normalized.
#[utoipa::path(
  get,
  path = "/api/v1/network",
  description = "Requires the live network capability. Returns the current live network snapshot and is not backed by an indexed historical dataset.",
  responses(
    (status = 200, description = "Current network snapshot", body = NetworkResponse),
    (status = 501, description = "Network feature disabled", body = ErrorResponse),
    (status = 503, description = "Network snapshot not ready yet", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Network"
)]
pub async fn snapshot(State(state): State<AppState>) -> Result<Json<NetworkResponse>, ApiError> {
  state.check_live_feature(LiveFeature::Network)?;

  let row = state
    .live_state
    .network()
    .ok_or_else(|| ApiError::service_unavailable("network snapshot is not ready"))?;
  let validator_aggregates = state
    .live_state
    .protx()
    .map(|snapshot| snapshot.aggregate())
    .unwrap_or_default();
  let network_info = serde_json::from_value::<NetworkInfoResponse>(
    row
      .payload
      .get("networkInfo")
      .cloned()
      .unwrap_or_else(|| serde_json::json!({})),
  )
  .map_err(|e| ApiError::internal(format!("decode networkInfo: {e}")))?;
  let peer_info = serde_json::from_value::<Vec<PeerInfoResponse>>(
    row
      .payload
      .get("peerInfo")
      .cloned()
      .unwrap_or_else(|| serde_json::json!([])),
  )
  .map_err(|e| ApiError::internal(format!("decode peerInfo: {e}")))?;

  Ok(Json(NetworkResponse {
    network_info,
    peer_info,
    peer_count: row
      .payload
      .get("peerCount")
      .and_then(Value::as_i64)
      .unwrap_or(0),
    validators: NetworkValidatorSummary {
      regular: NetworkValidatorClassSummary {
        total: validator_aggregates.regular_total,
        status: validator_aggregates.regular_status_buckets,
      },
      evonode: NetworkValidatorClassSummary {
        total: validator_aggregates.evonode_total,
        status: validator_aggregates.evonode_status_buckets,
      },
    },
    updated_at: rfc3339_time(row.updated_at)?,
  }))
}
