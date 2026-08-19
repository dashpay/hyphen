//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Status endpoint with per-stage sync progress.

use axum::extract::State;
use axum::Json;
use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use super::error::ApiError;
use super::openapi::{ErrorResponse, StageStatus, StatusResponse};
use super::{rfc3339_time, AppState, IndexCapabilities};
use crate::index::Stage;

/// Get daemon sync and indexing status.
///
/// # Errors
///
/// Returns `ApiError` when the storage-engine tip cannot be read or the current
/// timestamp cannot be normalized to RFC 3339.
#[utoipa::path(
  get,
  path = "/api/v1/status",
  description = "Server-specific readiness and stage status view. Does not require any optional index to exist, but reports which indices and live capabilities are enabled and whether they are ready.",
  responses(
    (status = 200, description = "Daemon status", body = StatusResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Status"
)]
pub async fn status(State(state): State<AppState>) -> Result<Json<StatusResponse>, ApiError> {
  let tip_height = state
    .hydb()?
    .status()
    .tip()
    .await
    .map_err(|e| {
      ApiError::from_daemon(
        crate::error::DaemonError::storage("query status tip", e),
        "query status tip",
      )
    })?
    .map_or(-1, |tip| tip.height.get() as i32);

  // Derivation is atomic per block in the storage engine, so every enabled
  // stage is lock-step at the canonical tip height. `updated_at` reflects the
  // last time ingest actually advanced, not the moment of this request; it
  // falls back to "now" only before the first progress is recorded.
  let progress_epoch = state
    .ingest_progress_epoch
    .load(std::sync::atomic::Ordering::Relaxed);
  let updated_at = rfc3339_time(if progress_epoch > 0 {
    progress_epoch.min(i64::MAX as u64) as i64
  } else {
    unix_now()
  })?;
  let mut indices = HashMap::new();
  for stage in enabled_stages(&state.indices) {
    indices.insert(
      stage.name().to_owned(),
      StageStatus {
        indexed_height: tip_height,
        updated_at: updated_at.clone(),
      },
    );
  }

  let node_tip = state.node_tip.load(std::sync::atomic::Ordering::Relaxed);
  let sync_mode =
    crate::ingest::sync_mode_label(state.sync_mode.load(std::sync::atomic::Ordering::Relaxed));
  Ok(Json(StatusResponse {
    sync_status: sync_status(tip_height, node_tip).to_string(),
    sync_mode: sync_mode.to_owned(),
    block_height: tip_height,
    indices,
    version: env!("CARGO_PKG_VERSION").to_string(),
  }))
}

fn enabled_stages(indices: &IndexCapabilities) -> Vec<Stage> {
  let flags = crate::index::IndexStageFlags {
    address: indices.address,
    metrics: indices.metrics,
    validator: indices.validator,
  };
  [
    Stage::Block,
    Stage::Transaction,
    Stage::Script,
    Stage::Spent,
    Stage::Metrics,
    Stage::Validator,
  ]
  .into_iter()
  .filter(|stage| stage.enabled_with_flags(flags))
  .collect()
}

/// "synced" only when a tip exists AND it has reached the node's reported
/// height (an unknown node height gives the local tip the benefit of the
/// doubt — the alternative is never reporting synced without RPC).
fn sync_status(tip_height: i32, node_tip: i64) -> &'static str {
  if tip_height < 0 || i64::from(tip_height) < node_tip {
    "syncing"
  } else {
    "synced"
  }
}

fn unix_now() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0)
}

#[cfg(test)]
mod tests {
  use super::sync_status;

  #[test]
  fn sync_status_is_syncing_before_first_block() {
    assert_eq!(sync_status(-1, -1), "syncing");
  }

  #[test]
  fn sync_status_is_synced_once_a_tip_exists() {
    assert_eq!(sync_status(0, -1), "synced");
    assert_eq!(sync_status(1_000_000, -1), "synced");
    assert_eq!(sync_status(1_000_000, 1_000_000), "synced");
  }

  #[test]
  fn sync_status_is_syncing_while_behind_the_node_tip() {
    assert_eq!(sync_status(10, 1_000_000), "syncing");
  }
}
