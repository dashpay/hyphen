//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Status endpoint with per-stage sync progress.

use axum::extract::State;
use axum::Json;
use std::collections::HashMap;

use super::error::ApiError;
use super::openapi::{ErrorResponse, StageStatus, StatusResponse};
use super::{rfc3339_time, AppState, IndexCapabilities};
use crate::index::Stage;
use crate::sql::query::SyncStateRow;

/// Get daemon sync and indexing status.
///
/// # Errors
///
/// Returns `ApiError` when the sync-state query fails or one timestamp cannot
/// be normalized to RFC 3339.
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
  let rows = crate::sql::query::sync_state_rows(&state.pool)
    .await
    .map_err(|e| ApiError::from_daemon(e, "query status sync_state"))?;

  let mut indices = HashMap::new();
  let mut block_height: Option<i32> = None;

  for row in &rows {
    if row.stage == Stage::Block.name() {
      block_height = Some(row.indexed_height);
    }

    indices.insert(
      row.stage.clone(),
      StageStatus {
        indexed_height: row.indexed_height,
        updated_at: rfc3339_time(row.updated_at)?,
      },
    );
  }

  Ok(Json(StatusResponse {
    sync_status: sync_status(&rows, &state.indices).to_string(),
    block_height: block_height.unwrap_or(-1),
    indices,
    version: env!("CARGO_PKG_VERSION").to_string(),
  }))
}

fn sync_status(rows: &[SyncStateRow], indices: &IndexCapabilities) -> &'static str {
  let heights: HashMap<Stage, i32> = rows
    .iter()
    .filter_map(|row| Stage::from_name(&row.stage).map(|stage| (stage, row.indexed_height)))
    .collect();

  for stage in [
    Stage::Block,
    Stage::Transaction,
    Stage::Script,
    Stage::Spent,
    Stage::Metrics,
    Stage::Validator,
  ] {
    if !stage.enabled_with_flags(crate::index::IndexStageFlags {
      address: indices.address,
      metrics: indices.metrics,
      validator: indices.validator,
    }) {
      continue;
    }

    let height = heights.get(&stage).copied().unwrap_or(-1);
    if height < 0 {
      return "syncing";
    }

    if let Some(gate_stage) = stage.gate_stage() {
      let gate_height = heights.get(&gate_stage).copied().unwrap_or(-1);
      if gate_height < 0 || height < gate_height {
        return "syncing";
      }
    }
  }

  "synced"
}

#[cfg(test)]
mod tests {
  use crate::sql::query::SyncStateRow;

  use super::sync_status;

  fn row(stage: &str, indexed_height: i32) -> SyncStateRow {
    SyncStateRow {
      stage: stage.to_owned(),
      indexed_height,
      updated_at: 0,
    }
  }

  #[test]
  fn sync_status_requires_enabled_stages_to_match_their_gates() {
    let rows = vec![
      row("block", 100),
      row("transaction", 99),
      row("script", 99),
      row("spent", 99),
      row("metrics", 100),
      row("validator", 100),
    ];

    assert_eq!(
      sync_status(
        &rows,
        &crate::rest::IndexCapabilities {
          address: true,
          metrics: true,
          validator: true,
        },
      ),
      "syncing",
    );
  }

  #[test]
  fn sync_status_ignores_disabled_optional_stages() {
    let rows = vec![row("block", 100), row("transaction", 100)];

    assert_eq!(
      sync_status(
        &rows,
        &crate::rest::IndexCapabilities {
          address: false,
          metrics: false,
          validator: false,
        },
      ),
      "synced",
    );
  }

  #[test]
  fn sync_status_requires_enabled_optional_stages_to_be_initialized() {
    let rows = vec![
      row("block", 100),
      row("transaction", 100),
      row("metrics", -1),
    ];

    assert_eq!(
      sync_status(
        &rows,
        &crate::rest::IndexCapabilities {
          address: false,
          metrics: true,
          validator: false,
        },
      ),
      "syncing",
    );
  }
}
