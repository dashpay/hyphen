//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! REST endpoints for block metrics.

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::rest::error::ApiError;
use crate::rest::openapi::{BlockMetrics, ErrorResponse, MetricsChartResponse};
use crate::rest::AppState;

const DEFAULT_CHART_BLOCKS: i64 = 144;
const DEFAULT_CHART_POINTS: i64 = 144;
const MAX_CHART_POINTS: i64 = 500;

#[derive(Debug, Deserialize, IntoParams)]
#[serde(default)]
pub struct MetricsChartQuery {
  /// Lookback window in blocks from the current metrics tip.
  #[param(minimum = 1, default = 144)]
  pub blocks: i64,

  /// Target number of output points (1-500).
  #[param(minimum = 1, maximum = 500, default = 144)]
  pub points: i64,
}

impl Default for MetricsChartQuery {
  fn default() -> Self {
    Self {
      blocks: DEFAULT_CHART_BLOCKS,
      points: DEFAULT_CHART_POINTS,
    }
  }
}

impl MetricsChartQuery {
  fn validate(self) -> Result<Self, ApiError> {
    if self.blocks < 1 {
      return Err(ApiError::bad_request("blocks must be at least 1"));
    }
    if !(1..=MAX_CHART_POINTS).contains(&self.points) {
      return Err(ApiError::bad_request(format!(
        "points must be between 1 and {MAX_CHART_POINTS}",
      )));
    }
    Ok(self)
  }
}

/// Get the latest block metrics snapshot.
///
/// # Errors
///
/// Returns `ApiError` when the metrics index is disabled, no metrics have
/// been indexed yet, or the backing query fails.
#[utoipa::path(
  get,
  path = "/api/v1/metrics/latest",
  description = "Requires the optional metrics index. Returns derived chain statistics rather than live Core RPC passthrough values.",
  responses(
    (status = 200, description = "Latest metrics", body = BlockMetrics),
    (status = 404, description = "No metrics available yet", body = ErrorResponse),
    (status = 501, description = "Metrics index not enabled", body = ErrorResponse),
    (status = 503, description = "Metrics index not ready yet", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Metrics"
)]
pub async fn latest(State(state): State<AppState>) -> Result<Json<BlockMetrics>, ApiError> {
  state
    .check_stage_typed(crate::index::Stage::Metrics)
    .await?;

  let row = super::sql::latest_metrics(&state.pool)
    .await
    .map_err(|e| ApiError::internal(format!("query: {e}")))?
    .ok_or_else(|| ApiError::not_found("no metrics available"))?;

  Ok(Json(row_to_metric(&row)))
}

/// Get block-ranged metrics chart buckets.
///
/// # Errors
///
/// Returns `ApiError` when the metrics stage is unavailable, no metrics have
/// been indexed yet, or the backing range query fails.
#[utoipa::path(
  get,
  path = "/api/v1/metrics/chart",
  description = "Requires the optional metrics index. Returns derived historical chart points from the metrics index.",
  params(MetricsChartQuery),
  responses(
    (status = 200, description = "Metrics chart", body = MetricsChartResponse),
    (status = 400, description = "Invalid chart query", body = ErrorResponse),
    (status = 404, description = "No metrics available yet", body = ErrorResponse),
    (status = 501, description = "Metrics index not enabled", body = ErrorResponse),
    (status = 503, description = "Metrics index not ready yet", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Metrics"
)]
pub async fn chart(
  State(state): State<AppState>,
  Query(params): Query<MetricsChartQuery>,
) -> Result<Json<MetricsChartResponse>, ApiError> {
  let stage = state
    .stage_status_typed(crate::index::Stage::Metrics)
    .await?;
  let query = normalize_chart_query(params.validate()?);
  let start_height = (stage.height as i64 - query.blocks + 1).max(0) as i32;
  let rows = super::sql::metrics_bucketed_range(
    &state.pool,
    start_height,
    stage.height,
    query.bucket_blocks as i32,
  )
  .await
  .map_err(|e| ApiError::internal(format!("query: {e}")))?;
  if rows.is_empty() {
    return Err(ApiError::not_found("no metrics available"));
  }
  let value = rows.iter().map(row_to_metric).collect();

  Ok(Json(MetricsChartResponse {
    as_of_height: stage.height,
    start_height,
    end_height: stage.height,
    bucket_blocks: query.bucket_blocks as i32,
    value,
  }))
}

struct NormalizedChartQuery {
  blocks: i64,
  bucket_blocks: i64,
}

fn normalize_chart_query(query: MetricsChartQuery) -> NormalizedChartQuery {
  let blocks = query.blocks;
  let points = query.points;
  let bucket_blocks = ((blocks + points - 1) / points).max(1);
  NormalizedChartQuery {
    blocks,
    bucket_blocks,
  }
}

fn row_to_metric(row: &super::sql::MetricsRow) -> BlockMetrics {
  BlockMetrics {
    height: row.height,
    tx_count: row.tx_count,
    utxos_spendable: row.utxos_spendable,
    utxos_unspendable: row.utxos_unspendable,
    blockchain_size: row.blockchain_size,
    credit_pool_balance: row.credit_pool_balance,
    circulating_supply: row.circulating_supply,
    difficulty: row.difficulty,
    hashrate: row.hashrate,
  }
}

#[cfg(test)]
mod tests {
  use super::{normalize_chart_query, MetricsChartQuery};

  #[test]
  fn chart_query_rejects_out_of_range_values() {
    let err = MetricsChartQuery {
      blocks: 0,
      points: 10_000,
    }
    .validate()
    .expect_err("out-of-range chart query should fail");
    assert_eq!(err.status, axum::http::StatusCode::BAD_REQUEST);
  }

  #[test]
  fn chart_query_computes_bucket_blocks_from_points() {
    let normalized = normalize_chart_query(MetricsChartQuery {
      blocks: 1000,
      points: 300,
    });
    assert_eq!(normalized.bucket_blocks, 4);
  }
}
