//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Serve-mode runtime assembly for daemon supervisors.

use std::path::Path;
use std::sync::Arc;

use tokio::sync::broadcast;

use hyphen_db::{HyDbConfig, HyphenDb};
use tracing::{info, warn};

use crate::api_metrics::ApiServerMetrics;
use crate::config;
use crate::error::DaemonError;
use crate::index;
use crate::init::Options;
use crate::lifecycle::Lifecycle;
use crate::rest;
use crate::sources::{self, RestSource};

use super::Visor;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SourceAvailability {
  rpc: bool,
  zmq: bool,
}

fn runtime_options_with_available_sources(
  options: &Options,
  availability: SourceAvailability,
) -> Options {
  let mut runtime = options.clone();

  if !availability.rpc {
    runtime.live.mempool = false;
    runtime.live.chainlock = false;
    runtime.live.governance = false;
    runtime.live.network = false;
    runtime.overlays.chainlock = false;
    runtime.overlays.protx = false;
  }

  if !availability.zmq {
    runtime.live.mempool = false;
    runtime.live.islock = false;
    runtime.live.stream = false;
  }

  runtime
}

/// Owned serve-mode daemon supervisors and their shared lifecycle.
pub(crate) struct ServeRuntime {
  lifecycle: Arc<Lifecycle>,
  ingest: crate::ingest::IngestVisor,
  live: super::live::LiveVisor,
  rest: Option<super::rest::RestVisor>,
  ipc: super::ipc::IpcVisor,
}

impl std::fmt::Debug for ServeRuntime {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ServeRuntime").finish_non_exhaustive()
  }
}

impl ServeRuntime {
  /// Build the full serve-mode supervisor graph from validated inputs.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` when any required runtime dependency cannot be
  /// created or validated before starting the daemon.
  pub async fn build(
    cfg: &config::Config,
    options: &Options,
    source: Arc<dyn RestSource>,
    datadir: &Path,
  ) -> Result<Self, DaemonError> {
    info!("building serve runtime");
    let rpc_client = cfg
      .core
      .rpc
      .as_ref()
      .filter(|rpc| rpc.enabled)
      .map(|rpc| {
        info!(url = %rpc.url, "creating Core JSON-RPC client");
        sources::CoreRpcClient::new(rpc)
      })
      .transpose()?
      .map(Arc::new);
    let rpc_client = if let Some(rpc) = rpc_client {
      match rpc.get_network_info().await {
        Ok(_) => Some(rpc),
        Err(error) => {
          warn!(
            error = %error,
            "Core JSON-RPC unavailable at startup, disabling RPC-backed live and overlay capabilities"
          );
          None
        }
      }
    } else {
      None
    };
    let zmq_source = if let Some(zmq) = cfg.core.zmq.as_ref().filter(|zmq| zmq.enabled) {
      info!(endpoints = ?zmq.connect, topics = ?zmq.subscribe, "connecting to Core ZMQ");
      match sources::CoreZmqSource::connect(zmq).await {
        Ok(source) => Some(Arc::new(source)),
        Err(error) => {
          warn!(
            error = %error,
            endpoints = ?zmq.connect,
            "Core ZMQ unavailable at startup, disabling ZMQ-backed live capabilities"
          );
          None
        }
      }
    } else {
      None
    };
    let runtime_options = runtime_options_with_available_sources(
      options,
      SourceAvailability {
        rpc: rpc_client.is_some(),
        zmq: zmq_source.is_some(),
      },
    );
    let lifecycle = Lifecycle::start();

    // Pass the operator's pool sizing through verbatim — what the settings
    // page displays as effective must actually be in effect. `HyDbConfig`'s
    // Debug impl redacts the URL's password.
    let db_cfg = HyDbConfig {
      url: cfg.postgres.url.clone(),
      write_pool_size: cfg.postgres.write_pool_size,
      query_pool_size: cfg.postgres.query_pool_size,
      heavy_query_pool_size: cfg.postgres.heavy_query_pool_size,
      admin_pool_size: cfg.postgres.admin_pool_size,
      slow_query_ms: cfg.postgres.slow_query_ms,
    };
    info!(config = ?db_cfg, "connecting to storage engine");
    let db = HyphenDb::connect(db_cfg)
      .await
      .map_err(|e| DaemonError::Config(format!("connect storage engine: {e}")))?;
    info!("storage engine ready, schema migrated");

    let data_src = Arc::new(crate::ingest::datasrc::HyDbBlockSource::new(
      cfg.chain.p2pkh_version,
      cfg.chain.p2sh_version,
    ));
    let ingest_handle = db
      .ingest(data_src)
      .await
      .map_err(|e| DaemonError::Config(format!("start ingest: {e}")))?;
    let ingest = crate::ingest::IngestVisor::new(
      db.clone(),
      ingest_handle,
      Arc::clone(&source),
      Arc::clone(&lifecycle),
      crate::ingest::DEFAULT_POLL_SECS,
    );

    let api_metrics = Arc::new(ApiServerMetrics::default());
    let overlay_plan = index::visor::OverlayPlan::from_options(&runtime_options);
    let overlay_capabilities = overlay_plan.capabilities;

    let live = super::live::LiveVisor::new(super::live::LiveVisor::config_from_options(
      super::live::LiveVisorOptionsInput {
        hydb: db.clone(),
        rpc: rpc_client.clone(),
        zmq: zmq_source.clone(),
        lifecycle: Arc::clone(&lifecycle),
        overlay_plan,
        p2pkh_version: cfg.chain.p2pkh_version,
        p2sh_version: cfg.chain.p2sh_version,
      },
      &runtime_options,
    ));
    let live_handles = live.handles();

    let rest = if options.api.enabled {
      let bind = options
        .api
        .bind
        .clone()
        .ok_or_else(|| DaemonError::Config("api enabled without api.bind".to_owned()))?;
      Some(super::rest::RestVisor::new(super::rest::RestVisorConfig {
        hydb: db.clone(),
        p2pkh_version: cfg.chain.p2pkh_version,
        p2sh_version: cfg.chain.p2sh_version,
        block_time_secs: cfg.chain.block_time_secs,
        indices: rest::IndexCapabilities {
          address: runtime_options.indices.address,
          validator: runtime_options.indices.validator,
          metrics: runtime_options.indices.metrics,
        },
        live: rest::LiveCapabilities {
          mempool: runtime_options.live.mempool,
          islock: runtime_options.live.islock,
          network: runtime_options.live.network,
          governance: runtime_options.live.governance,
          stream: runtime_options.live.stream,
        },
        overlays: overlay_capabilities,
        recent_mempool_txs: runtime_options.recent_mempool_txs,
        stream_hub: live_handles.stream_hub.clone(),
        live_state: live_handles.live_state.clone(),
        stream_heartbeat_secs: runtime_options.stream_heartbeat_secs,
        node_tip: ingest.status().network_tip_handle(),
        ingest_progress_epoch: ingest.status().progress_epoch_handle(),
        bind,
        cors_origin: options.api.cors_origin.clone(),
        server_metrics: Arc::clone(&api_metrics),
        lifecycle: Arc::clone(&lifecycle),
      }))
    } else {
      None
    };

    let event_tx = broadcast::channel::<hyphend::wire::msg::DaemonMessage>(1024).0;
    let config_snapshot = Arc::new(
      runtime_options
        .config_snapshot(cfg)
        .map_err(|e| DaemonError::Config(format!("build config snapshot: {e}")))?,
    );
    let pg_diag_base = Arc::new(hyphend::wire::msg::PostgresDiag {
      write_pool_size: Some(cfg.postgres.write_pool_size),
      query_pool_size: Some(cfg.postgres.query_pool_size),
      heavy_query_pool_size: Some(cfg.postgres.heavy_query_pool_size),
      admin_pool_size: Some(cfg.postgres.admin_pool_size),
      slow_query_ms: Some(cfg.postgres.slow_query_ms),
      ..Default::default()
    });
    let ipc = super::ipc::IpcVisor::new(super::ipc::IpcVisorConfig {
      socket_path: datadir.join("hyphen.sock"),
      options: runtime_options.clone(),
      hydb: db.clone(),
      ingest: ingest.ingest_handle(),
      ingest_status: ingest.status(),
      progress_rx: ingest.progress(),
      source: Arc::clone(&source),
      rpc: rpc_client.clone(),
      zmq: zmq_source.clone(),
      event_tx,
      lifecycle: Arc::clone(&lifecycle),
      config_snapshot,
      pg_diag_base,
      api_metrics: Arc::clone(&api_metrics),
    });

    Ok(Self {
      lifecycle,
      ingest,
      live,
      rest,
      ipc,
    })
  }

  /// Start all owned subsystem supervisors.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` when any subsystem fails startup.
  pub async fn start_all(&mut self) -> Result<(), DaemonError> {
    self.live.start().await?;
    if let Some(rest) = self.rest.as_mut() {
      rest.start().await?;
    }
    self.ipc.start().await?;
    self.ingest.start();
    Ok(())
  }

  /// Stop all owned subsystem supervisors in reverse dependency order.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` when shutdown does not complete cleanly.
  pub async fn stop_all(
    &mut self,
    sigterm: &mut tokio::signal::unix::Signal,
    graceful_deadline: Option<tokio::time::Instant>,
  ) -> Result<(), DaemonError> {
    self.ingest.stop().await;
    self.ipc.stop(sigterm, graceful_deadline).await?;
    if let Some(rest) = self.rest.as_mut() {
      rest.stop(sigterm, graceful_deadline).await?;
    }
    self.live.stop(sigterm, graceful_deadline).await?;
    Ok(())
  }

  /// Shared lifecycle handle for process-level orchestration.
  pub fn lifecycle(&self) -> Arc<Lifecycle> {
    Arc::clone(&self.lifecycle)
  }
}

#[cfg(test)]
mod tests {
  use super::{runtime_options_with_available_sources, SourceAvailability};
  use crate::init::options::{
    ApiOptions, CacheOptions, IndexOptions, LiveOptions, Options, OverlayOptions, StartupMode,
  };

  fn test_options() -> Options {
    Options {
      startup_mode: StartupMode::Serve,
      indices: IndexOptions {
        address: true,
        validator: true,
        metrics: true,
      },
      live: LiveOptions {
        mempool: true,
        islock: true,
        chainlock: true,
        governance: true,
        network: true,
        stream: true,
      },
      overlays: OverlayOptions {
        protx: true,
        chainlock: true,
      },
      cache: CacheOptions {
        enabled: false,
        address_enabled: false,
        address_budget: "1MiB".to_owned(),
        address_budget_bytes: 1,
        address_hot_cache_threshold: 0,
        address_summary_catchup_lag: 0,
        readiness_enabled: false,
        readiness_budget: "1MiB".to_owned(),
        readiness_budget_bytes: 1,
      },
      api: ApiOptions {
        enabled: true,
        bind: Some("127.0.0.1:3080".to_owned()),
        cors_origin: None,
      },
      recent_mempool_txs: 25,
      stream_client_buffer: 128,
      stream_heartbeat_secs: 5,
      chainlock_start_block: Some(1),
      network_refresh_secs: 5,
      governance_refresh_secs: 30,
      chainlock_refresh_secs: 5,
      protx_refresh_secs: 30,
    }
  }

  #[test]
  fn runtime_options_disable_rpc_backed_capabilities_when_rpc_is_down() {
    let options = runtime_options_with_available_sources(
      &test_options(),
      SourceAvailability {
        rpc: false,
        zmq: true,
      },
    );

    assert!(!options.live.mempool);
    assert!(options.live.islock);
    assert!(!options.live.chainlock);
    assert!(!options.live.governance);
    assert!(!options.live.network);
    assert!(options.live.stream);
    assert!(!options.overlays.protx);
    assert!(!options.overlays.chainlock);
  }

  #[test]
  fn runtime_options_disable_zmq_backed_capabilities_when_zmq_is_down() {
    let options = runtime_options_with_available_sources(
      &test_options(),
      SourceAvailability {
        rpc: true,
        zmq: false,
      },
    );

    assert!(!options.live.mempool);
    assert!(!options.live.islock);
    assert!(options.live.chainlock);
    assert!(options.live.governance);
    assert!(options.live.network);
    assert!(!options.live.stream);
    assert!(options.overlays.protx);
    assert!(options.overlays.chainlock);
  }
}
