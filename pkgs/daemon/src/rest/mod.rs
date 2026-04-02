//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! REST API server built on Axum.

pub mod error;
pub mod openapi;
pub mod search;
pub mod status;

use std::sync::atomic::Ordering;
use std::sync::Arc;

use crate::sql::SqlPool;
use async_trait::async_trait;
use axum::middleware::{self, Next};
use axum::Router;
use rocksdb::DB;
use tower_http::compression::CompressionLayer;
use tower_http::cors::CorsLayer;

use serde::de::DeserializeOwned;
use serde::Deserialize;
use serde::Serialize;
use time::format_description::well_known::Rfc3339;
use time::OffsetDateTime;
use utoipa::IntoParams;

use crate::api_metrics::ApiServerMetrics;
use crate::error::DaemonError;
use crate::index::block::rest as block_rest;
use crate::index::metrics::rest as metrics_rest;
use crate::index::spent::cache::SpentQueryCache;
use crate::index::spent::rest as spent_rest;
use crate::index::tx::rest as tx_rest;
use crate::index::validator::rest as validator_rest;
use crate::index::Stage;
use crate::live::gov::rest as gov_rest;
use crate::live::mempool::rest as mempool_rest;
use crate::live::network::rest as network_rest;
use crate::live::stream::rest as stream_rest;
use crate::live::stream::StreamHub;
use crate::policy::watchdog::{
  WATCHDOG_ACTIVITY_STALE_SECS, WATCHDOG_STARTING_STALE_SECS, WATCHDOG_TICK_SECS,
};
use crate::readiness::ReadinessCache;
use crate::visor::{
  Fenceable, ProgressState, RecoveryDisposition, RunState, ShutdownPhase, Supervisable,
};
use crate::worker::{
  runtime_watchdog_disposition, spawn_activity_ticker, SupervisedTaskSpec, WorkerRuntimeState,
  WorkerTask,
};

pub const DEFAULT_API_BIND: &str = "0.0.0.0:3080";
pub const DEFAULT_PAGINATION_LIMIT: i64 = 25;

/// Shared cursor parameters for paginated API endpoints.
#[derive(Debug, Deserialize, IntoParams)]
#[serde(default)]
pub struct CursorParams {
  /// Max results per page (1-100).
  #[param(minimum = 1, maximum = 100, default = 25)]
  pub limit: i64,
  /// Opaque pagination cursor returned by a prior page.
  pub cursor: Option<String>,
}

/// Shared live page envelope for collection responses.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct CursorPage<T> {
  pub total_count: i64,
  pub value: Vec<T>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

/// Shared indexed page envelope for collection responses.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexedCursorPage<T> {
  pub as_of_height: i32,
  pub total_count: i64,
  pub value: Vec<T>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub next_cursor: Option<String>,
}

impl Default for CursorParams {
  fn default() -> Self {
    Self {
      limit: DEFAULT_PAGINATION_LIMIT,
      cursor: None,
    }
  }
}

impl CursorParams {
  /// Validate one shared cursor query.
  ///
  /// # Errors
  ///
  /// Returns `ApiError::BadRequest` when `limit` is out of range.
  pub fn validate(self, max_limit: i64) -> Result<Self, error::ApiError> {
    if self.limit < 1 || self.limit > max_limit {
      return Err(error::ApiError::bad_request(format!(
        "limit must be between 1 and {max_limit}",
      )));
    }
    Ok(self)
  }
}

/// Encode one opaque pagination cursor.
///
/// # Errors
///
/// Returns `ApiError` when cursor serialization fails.
pub fn encode_cursor<T: Serialize>(cursor: &T) -> Result<String, error::ApiError> {
  let bytes = serde_json::to_vec(cursor)
    .map_err(|e| error::ApiError::internal(format!("encode cursor: {e}")))?;
  Ok(hex::encode(bytes))
}

/// Decode one opaque pagination cursor.
///
/// # Errors
///
/// Returns `ApiError::BadRequest` when the cursor is malformed.
pub fn decode_cursor<T: DeserializeOwned>(
  cursor: Option<&str>,
) -> Result<Option<T>, error::ApiError> {
  let Some(cursor) = cursor else {
    return Ok(None);
  };
  let raw =
    hex::decode(cursor).map_err(|_| error::ApiError::bad_request("invalid cursor encoding"))?;
  serde_json::from_slice(&raw)
    .map(Some)
    .map_err(|_| error::ApiError::bad_request("invalid cursor payload"))
}

/// Format one Unix timestamp in seconds as an RFC 3339 UTC string.
///
/// # Errors
///
/// Returns `ApiError` when the timestamp is outside the supported range.
pub fn rfc3339_time(epoch_secs: i64) -> Result<String, error::ApiError> {
  let dt = OffsetDateTime::from_unix_timestamp(epoch_secs)
    .map_err(|e| error::ApiError::internal(format!("invalid unix timestamp {epoch_secs}: {e}")))?;
  dt.format(&Rfc3339)
    .map_err(|e| error::ApiError::internal(format!("format rfc3339 timestamp {epoch_secs}: {e}")))
}

#[cfg(test)]
mod tests {
  use serde::{Deserialize, Serialize};

  use super::{decode_cursor, encode_cursor, rfc3339_time};

  #[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
  struct Cursor {
    as_of_height: i32,
    offset: i64,
  }

  #[test]
  fn rfc3339_time_formats_unix_seconds_in_utc() {
    let formatted = rfc3339_time(42);
    assert!(formatted.is_ok());
    assert_eq!(formatted.ok().as_deref(), Some("1970-01-01T00:00:42Z"));
  }

  #[test]
  fn cursor_round_trip_preserves_payload() {
    let cursor = Cursor {
      as_of_height: 42,
      offset: 25,
    };
    let encoded = encode_cursor(&cursor);
    assert!(encoded.is_ok());
    let decoded: Result<Option<Cursor>, _> =
      decode_cursor(encoded.as_ref().ok().map(String::as_str));
    assert_eq!(decoded.ok().flatten(), Some(cursor));
  }
}

/// Shared application state for all API handlers.
#[derive(Clone)]
pub struct AppState {
  pub pool: SqlPool,
  pub address_pool: SqlPool,
  pub db: Arc<DB>,
  pub secondary_stores: Option<Arc<crate::store::manager::SecondaryStoreSet>>,
  pub readiness_cache: Arc<ReadinessCache>,
  pub spent_cache: Arc<SpentQueryCache>,
  pub address_hot_cache_threshold: i32,
  pub address_summary_catchup_lag: i32,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub indices: IndexCapabilities,
  pub live: LiveCapabilities,
  pub overlays: OverlayCapabilities,
  #[allow(dead_code)]
  pub recent_mempool_txs: usize,
  pub stream_hub: Option<Arc<StreamHub>>,
  pub stream_heartbeat_secs: u64,
}

/// Inputs needed to construct one supervised API worker task.
#[derive(Clone)]
pub(crate) struct ApiWorkerTaskConfig {
  pub state: AppState,
  pub bind: String,
  pub shutdown_rx: tokio::sync::watch::Receiver<bool>,
  pub server_metrics: Arc<ApiServerMetrics>,
  pub cors_origin: Option<String>,
  pub lifecycle: Arc<crate::lifecycle::Lifecycle>,
}

impl std::fmt::Debug for ApiWorkerTaskConfig {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ApiWorkerTaskConfig")
      .field("state", &self.state)
      .field("bind", &self.bind)
      .field("shutdown_rx", &"<watch::Receiver>")
      .field("server_metrics", &"<ApiServerMetrics>")
      .field("cors_origin", &self.cors_origin)
      .field("lifecycle", &"<Lifecycle>")
      .finish()
  }
}

#[derive(Clone, Debug)]
pub(crate) struct RestWorker {
  lifecycle: Arc<crate::lifecycle::Lifecycle>,
  phase: &'static str,
  recoverable: bool,
  runtime: Arc<WorkerRuntimeState>,
}

impl RestWorker {
  fn new(
    lifecycle: Arc<crate::lifecycle::Lifecycle>,
    phase: &'static str,
    recoverable: bool,
  ) -> Self {
    Self {
      lifecycle,
      phase,
      recoverable,
      runtime: WorkerRuntimeState::shared(),
    }
  }

  fn current_shutdown_phase(&self) -> ShutdownPhase {
    if self.lifecycle.terminated() {
      ShutdownPhase::Forcing
    } else if self.lifecycle.interrupted() {
      ShutdownPhase::Interrupting
    } else {
      ShutdownPhase::Running
    }
  }
}

#[async_trait]
impl Supervisable for RestWorker {
  type RunId = u64;

  fn run_state(&self) -> RunState<Self::RunId> {
    let progress_state = if self.runtime.recovering() {
      ProgressState::Recovering
    } else if self.runtime.finished() || !self.lifecycle.running() {
      ProgressState::Idle
    } else if self.runtime.started() && !self.runtime.progress_started() {
      ProgressState::Starting
    } else if self.runtime.started() {
      ProgressState::Active
    } else {
      ProgressState::Idle
    };
    RunState {
      run_id: self.runtime.run_id(),
      phase: self.phase,
      subphase: None,
      detail: None,
      progress_state,
      last_activity_epoch: self.runtime.last_activity_epoch(),
      last_progress_epoch: self.runtime.last_progress_epoch(),
      recovery_attempts: self.runtime.recovery_attempts(),
      shutdown_phase: self.current_shutdown_phase(),
    }
  }

  fn interrupt(&self) {
    self.lifecycle.interrupt();
  }

  fn recovery_disposition(&self, run_id: Option<Self::RunId>) -> RecoveryDisposition {
    match self.current_shutdown_phase() {
      ShutdownPhase::Interrupting => RecoveryDisposition::Blocked("interrupt requested"),
      ShutdownPhase::Forcing => RecoveryDisposition::Blocked("shutdown in progress"),
      ShutdownPhase::Running if self.recoverable => self.runtime.recovery_disposition(run_id),
      ShutdownPhase::Running => RecoveryDisposition::Blocked("worker is not restartable"),
    }
  }

  fn watchdog_disposition(
    &self,
    state: &RunState<Self::RunId>,
    now_epoch: u64,
  ) -> RecoveryDisposition {
    match self.current_shutdown_phase() {
      ShutdownPhase::Interrupting => RecoveryDisposition::Blocked("interrupt requested"),
      ShutdownPhase::Forcing => RecoveryDisposition::Blocked("shutdown in progress"),
      ShutdownPhase::Running => runtime_watchdog_disposition(
        &self.runtime,
        state.run_id,
        now_epoch,
        WATCHDOG_STARTING_STALE_SECS,
        WATCHDOG_ACTIVITY_STALE_SECS,
        None,
      ),
    }
  }

  async fn recover(&self, run_id: Option<Self::RunId>) -> Result<(), DaemonError> {
    match self.runtime.begin_recovery(run_id) {
      RecoveryDisposition::NotNeeded | RecoveryDisposition::Eligible => Ok(()),
      RecoveryDisposition::Blocked(reason) => Err(DaemonError::Config(format!(
        "api server recovery blocked: {reason}"
      ))),
      RecoveryDisposition::Exhausted => Err(DaemonError::Config(
        "api server recovery exhausted retry budget".to_owned(),
      )),
      RecoveryDisposition::Fatal(reason) => Err(DaemonError::Config(format!(
        "api server recovery unavailable: {reason}"
      ))),
    }
  }
}

impl Fenceable for RestWorker {
  fn fence(&self, _run_id: Option<Self::RunId>) {}
}

/// Build one API worker task without exposing serve internals to the visor.
fn api_worker_task(cfg: ApiWorkerTaskConfig) -> WorkerTask<RestWorker> {
  let worker = RestWorker::new(Arc::clone(&cfg.lifecycle), "api_server", true);
  let runtime = Arc::clone(&worker.runtime);
  let task = async move {
    runtime.mark_started();
    let ticker = spawn_activity_ticker(
      Arc::clone(&cfg.lifecycle),
      Arc::clone(&runtime),
      std::time::Duration::from_secs(WATCHDOG_TICK_SECS),
    );
    let result = serve(
      cfg.state,
      &cfg.bind,
      cfg.shutdown_rx,
      cfg.server_metrics,
      cfg.cors_origin.as_deref(),
    )
    .await;
    ticker.abort();
    runtime.mark_finished();
    result
  };
  WorkerTask::new(worker, task)
}

#[must_use]
pub(crate) fn api_supervised_task(cfg: ApiWorkerTaskConfig) -> SupervisedTaskSpec<RestWorker> {
  SupervisedTaskSpec::new("api server", api_worker_task(cfg.clone()), move || {
    Ok(api_worker_task(cfg.clone()))
  })
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexCapabilities {
  pub address: bool,
  pub validator: bool,
  pub metrics: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveCapabilities {
  pub mempool: bool,
  pub islock: bool,
  pub network: bool,
  pub governance: bool,
  pub stream: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayCapabilities {
  pub protx: bool,
  pub chainlock: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LiveFeature {
  Mempool,
  Network,
  Governance,
  Stream,
}

impl LiveFeature {
  fn name(self) -> &'static str {
    match self {
      Self::Mempool => "mempool",
      Self::Network => "network",
      Self::Governance => "governance",
      Self::Stream => "stream",
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverlayFeature {
  Protx,
  Chainlock,
}

/// One stage readiness result resolved from cached `sync_state`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StageStatus {
  pub height: i32,
  pub gate_height: Option<i32>,
}

impl std::fmt::Debug for AppState {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("AppState").finish()
  }
}

impl AppState {
  /// Verify that `stage` is enabled and fully caught up enough to serve.
  ///
  /// # Errors
  ///
  /// Returns `ApiError` when the stage is disabled, uninitialized, still
  /// gated on an upstream stage, or when the backing query fails.
  pub async fn check_stage_typed(&self, stage: Stage) -> Result<(), error::ApiError> {
    let _ = self.stage_status_typed(stage).await?;
    Ok(())
  }

  /// Return the cached ready state for one stage.
  ///
  /// # Errors
  ///
  /// Returns `ApiError` under the same conditions as `check_stage_typed`.
  pub async fn stage_status_typed(&self, stage: Stage) -> Result<StageStatus, error::ApiError> {
    let stage_name = stage.name();

    if !self.stage_enabled_typed(stage) {
      return Err(error::ApiError::not_implemented(format!(
        "{stage_name} index is not enabled",
      )));
    }

    let snapshot = self
      .readiness_cache
      .snapshot(&self.pool)
      .await
      .map_err(|e| error::ApiError::from_daemon(e, &format!("read {stage_name} readiness")))?;
    let height = snapshot.height(stage).unwrap_or(-1);
    if height < 0 {
      return Err(error::ApiError::service_unavailable(format!(
        "{stage_name} index is not ready",
      )));
    }

    if let Some(gate_stage) = stage.gate_stage() {
      let gate_height = snapshot.height(gate_stage).unwrap_or(-1);
      if gate_height < 0 || height < gate_height {
        return Err(error::ApiError::service_unavailable(format!(
          "{stage_name} index is catching up",
        )));
      }
      return Ok(StageStatus {
        height,
        gate_height: Some(gate_height),
      });
    }

    Ok(StageStatus {
      height,
      gate_height: None,
    })
  }

  fn stage_enabled_typed(&self, stage: Stage) -> bool {
    stage.enabled_with_flags(crate::index::IndexStageFlags {
      address: self.indices.address,
      metrics: self.indices.metrics,
      validator: self.indices.validator,
    })
  }

  /// Verify that one live capability is enabled before serving a route.
  ///
  /// # Errors
  ///
  /// Returns `ApiError::NotImplemented` when the selected live capability is
  /// disabled.
  pub fn check_live_feature(&self, feature: LiveFeature) -> Result<(), error::ApiError> {
    let enabled = match feature {
      LiveFeature::Mempool => self.live.mempool,
      LiveFeature::Network => self.live.network,
      LiveFeature::Governance => self.live.governance,
      LiveFeature::Stream => self.live.stream,
    };
    if enabled {
      Ok(())
    } else {
      Err(error::ApiError::not_implemented(format!(
        "{} feature is not enabled",
        feature.name(),
      )))
    }
  }

  pub fn overlay_enabled(&self, feature: OverlayFeature) -> bool {
    match feature {
      OverlayFeature::Protx => self.overlays.protx,
      OverlayFeature::Chainlock => self.overlays.chainlock,
    }
  }

  /// Verify that expensive address summary reads are safe to serve.
  ///
  /// # Errors
  ///
  /// Returns `ApiError::ServiceUnavailable` when the spent stage is still too
  /// far behind its gate height for derived summary reads.
  pub fn check_address_summary_stage(&self, status: StageStatus) -> Result<(), error::ApiError> {
    let Some(gate_height) = status.gate_height else {
      return Ok(());
    };
    if gate_height.saturating_sub(status.height) > self.address_summary_catchup_lag {
      return Err(error::ApiError::service_unavailable(
        "address summary is unavailable while the spent index is catching up",
      ));
    }
    Ok(())
  }
}

async fn metrics_middleware(
  state: axum::extract::State<Arc<ApiServerMetrics>>,
  req: axum::extract::Request,
  next: Next,
) -> axum::response::Response {
  let start = std::time::Instant::now();
  let resp = next.run(req).await;
  let elapsed_us = start.elapsed().as_micros() as u64;
  state.total_requests.fetch_add(1, Ordering::Relaxed);
  state.record_latency(elapsed_us);
  if resp.status().is_server_error() {
    state.total_errors.fetch_add(1, Ordering::Relaxed);
  }
  resp
}

/// Build the full REST router, including the generated Swagger UI.
pub fn router(
  state: AppState,
  server_metrics: Arc<ApiServerMetrics>,
  cors_origin: Option<&str>,
) -> Router {
  use axum::routing::get;

  Router::new()
    .route("/api/v1/blocks", get(block_rest::list_blocks))
    .route("/api/v1/block/{hashOrHeight}", get(block_rest::get_block))
    .route(
      "/api/v1/block/{hashOrHeight}/txs",
      get(block_rest::get_block_txs),
    )
    .route(
      "/api/v1/rawblock/{hashOrHeight}",
      get(block_rest::get_rawblock),
    )
    .route("/api/v1/txs", get(tx_rest::list_txs))
    .route("/api/v1/tx/{txid}", get(tx_rest::get_tx))
    .route("/api/v1/rawtx/{txid}", get(tx_rest::get_rawtx))
    .route("/api/v1/address/{addr}", get(spent_rest::get_address))
    .route(
      "/api/v1/address/{addr}/txs",
      get(spent_rest::get_address_txs),
    )
    .route(
      "/api/v1/address/{addr}/utxos",
      get(spent_rest::get_address_utxos),
    )
    .route("/api/v1/balances", get(spent_rest::get_balances))
    .route("/api/v1/metrics/latest", get(metrics_rest::latest))
    .route("/api/v1/metrics/chart", get(metrics_rest::chart))
    .route("/api/v1/mempool", get(mempool_rest::summary))
    .route("/api/v1/mempool/txs", get(mempool_rest::recent_txs))
    .route("/api/v1/network", get(network_rest::snapshot))
    .route("/api/v1/proposals", get(gov_rest::list_proposals))
    .route("/api/v1/proposal/{hash}", get(gov_rest::get_proposal))
    .route("/api/v1/proposal/{hash}/votes", get(gov_rest::get_votes))
    .route("/api/v1/search", get(search::search))
    .route("/api/v1/stream", get(stream_rest::events))
    .route("/api/v1/validators", get(validator_rest::list_validators))
    .route(
      "/api/v1/validator/{proTxHash}",
      get(validator_rest::get_validator),
    )
    .route("/api/v1/status", get(status::status))
    .layer(CompressionLayer::new())
    .layer(cors_layer(cors_origin))
    .with_state(state)
    .layer(middleware::from_fn_with_state(
      server_metrics,
      metrics_middleware,
    ))
    .merge(swagger_ui())
}

/// Build the Swagger UI routes for the generated OpenAPI document.
fn swagger_ui() -> utoipa_swagger_ui::SwaggerUi {
  use utoipa::OpenApi;
  utoipa_swagger_ui::SwaggerUi::new("/api/docs")
    .url("/api/docs/openapi.json", openapi::ApiDoc::openapi())
}

/// Build CORS layer. If `origin` is set, only that origin is
/// allowed; otherwise all origins are permitted.
fn cors_layer(origin: Option<&str>) -> CorsLayer {
  match origin {
    Some(o) => {
      use axum::http::HeaderValue;
      use tower_http::cors::AllowOrigin;
      match HeaderValue::from_str(o) {
        Ok(hv) => CorsLayer::new()
          .allow_origin(AllowOrigin::exact(hv))
          .allow_methods(tower_http::cors::Any)
          .allow_headers(tower_http::cors::Any),
        Err(_) => {
          tracing::warn!(
            origin = o,
            "invalid CORS origin, falling back to permissive"
          );
          CorsLayer::permissive()
        }
      }
    }
    None => CorsLayer::permissive(),
  }
}

/// Bind and serve the REST API until shutdown is signalled.
///
/// # Errors
///
/// Returns `DaemonError::Io` when the listener cannot be bound, or
/// `DaemonError::Http` when Axum exits with a server error.
pub async fn serve(
  state: AppState,
  bind: &str,
  mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
  server_metrics: Arc<ApiServerMetrics>,
  cors_origin: Option<&str>,
) -> Result<(), DaemonError> {
  let app = router(state, server_metrics, cors_origin);
  let listener = tokio::net::TcpListener::bind(bind)
    .await
    .map_err(|e| DaemonError::Io {
      context: "api bind",
      source: e,
    })?;
  tracing::info!(bind, "API server listening");
  axum::serve(listener, app)
    .with_graceful_shutdown(async move {
      let _ = shutdown_rx.changed().await;
      tracing::info!("API server shutting down");
    })
    .await
    .map_err(|e| DaemonError::Io {
      context: "api serve",
      source: e,
    })?;
  Ok(())
}

#[cfg(test)]
pub(crate) mod test_support {
  /// Build one minimal REST app state for handler tests that only exercise
  /// feature gating and response shaping.
  pub(crate) fn test_state() -> super::AppState {
    crate::tests::app_state()
  }
}
