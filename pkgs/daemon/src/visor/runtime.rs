//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Serve-mode runtime assembly for daemon supervisors.

use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use rocksdb::DB;
use tracing::{info, warn};

use crate::api_metrics::ApiServerMetrics;
use crate::config;
use crate::error::DaemonError;
use crate::index;
use crate::index::batch::{
  DEFAULT_COMMIT_BATCH_BLOCKS, DEFAULT_INGESTION_BATCH_SIZE, DEFAULT_MAX_BATCH_SIZE,
  DEFAULT_MIN_BATCH_SIZE, DEFAULT_PREFETCH_BLOCKS, DEFAULT_TARGET_BATCH_MS,
};
use crate::init::{ConfigSnapshotParams, Options};
use crate::ipc;
use crate::lifecycle::Lifecycle;
use crate::readiness::ReadinessCache;
use crate::recovery;
use crate::rest;
use crate::sources::{self, RestSource};
use crate::sql::SqlPool;
use crate::store::manager::SecondaryStoreSet;

use super::core::{CoreVisor, CoreVisorOptionsInput};
use super::Visor;

const INDEXER_IDLE_TIMEOUT_SECS: u64 = 60;

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
  admin_pool: SqlPool,
  core: CoreVisor,
  live: super::live::LiveVisor,
  ipc: super::ipc::IpcVisor,
  rest: Option<super::rest::RestVisor>,
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
    db: Arc<DB>,
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
    let (event_tx, _) = tokio::sync::broadcast::channel::<hyphend::wire::msg::DaemonMessage>(1024);
    let lifecycle = Lifecycle::start();

    info!(url = %cfg.postgres.url, "connecting to postgres");
    let pg_pools = crate::sql::create_pools(&cfg.postgres).await?;
    crate::sql::bootstrap::bootstrap(&pg_pools.write).await?;
    info!("postgres ready, schema bootstrapped");

    let readiness_cache = Arc::new(ReadinessCache::new(
      options.cache.readiness_enabled,
      options.cache.readiness_budget_bytes,
    ));
    let spent_cache = Arc::new(index::spent::cache::SpentQueryCache::new(
      options.cache.address_enabled,
      options.cache.address_budget_bytes,
    ));
    let secondary_stores = Arc::new(SecondaryStoreSet::open(
      datadir,
      &cfg.chain,
      cfg.rocksdb.as_ref(),
    )?);

    recovery::verify_rocks_covers_pg(&pg_pools.write, &db).await?;
    recovery::verify_pg_undo_coverage_or_empty(&pg_pools.write).await?;
    recovery::clamp_sync_state(
      &pg_pools.write,
      index::IndexStageFlags {
        address: options.indices.address,
        metrics: options.indices.metrics,
        validator: options.indices.validator,
      },
    )
    .await?;
    let rocks_tip = crate::db::get_tip_height(&db)
      .map(|height| height as i32)
      .unwrap_or(-1);
    let chain_progress = crate::sql::progress::chain_progress(&pg_pools.write).await?;
    crate::store::manager::catch_up_recent(
      &db,
      &secondary_stores,
      (chain_progress.canonical_tip_height >= 0)
        .then_some(chain_progress.canonical_tip_height as u32),
      &cfg.chain,
    )?;
    crate::store::manager::verify_recent(
      &db,
      &secondary_stores,
      (chain_progress.canonical_tip_height >= 0)
        .then_some(chain_progress.canonical_tip_height as u32),
      &cfg.chain,
      500,
    )?;
    if chain_progress.canonical_tip_height < 0 && rocks_tip >= 0 {
      info!(
        rocks_tip,
        "postgres is empty; rebuilding PostgreSQL state from local RocksDB"
      );
    }

    let idle_timeout = Duration::from_secs(INDEXER_IDLE_TIMEOUT_SECS);
    let core = CoreVisor::new(CoreVisor::config_from_options(
      CoreVisorOptionsInput {
        db: Arc::clone(&db),
        pool: pg_pools.write.clone(),
        source: Arc::clone(&source),
        secondary_stores: Some(Arc::clone(&secondary_stores)),
        event_tx: event_tx.clone(),
        lifecycle: Arc::clone(&lifecycle),
        spent_cache: Arc::clone(&spent_cache),
        idle_timeout,
        p2pkh_version: cfg.chain.p2pkh_version,
        p2sh_version: cfg.chain.p2sh_version,
        block_time_secs: cfg.chain.block_time_secs,
      },
      &runtime_options,
      cfg.performance.as_ref(),
    ));
    let core_handles = core.handles();

    let perf = cfg.performance.as_ref();
    let config_snapshot = Arc::new(
      runtime_options.config_snapshot(
        cfg,
        ConfigSnapshotParams {
          ingestion_batch_size: perf
            .map_or(DEFAULT_INGESTION_BATCH_SIZE, |p| p.ingestion_batch_size)
            .max(1),
          max_ingestion_lead_blocks: perf
            .map_or(crate::ingest::DEFAULT_MAX_INGEST_LEAD_BLOCKS, |p| {
              p.max_ingestion_lead_blocks
            }),
          prefetch_blocks: perf.map_or(DEFAULT_PREFETCH_BLOCKS, |p| p.prefetch_blocks),
          commit_batch_blocks: perf.map_or(DEFAULT_COMMIT_BATCH_BLOCKS, |p| p.commit_batch_blocks),
          min_batch_size: perf.map_or(DEFAULT_MIN_BATCH_SIZE, |p| p.min_batch_size),
          max_batch_size: perf.map_or(DEFAULT_MAX_BATCH_SIZE, |p| p.max_batch_size),
          target_batch_ms: perf.map_or(DEFAULT_TARGET_BATCH_MS, |p| p.target_batch_ms),
        },
      )?,
    );
    let pg_diag_base = Arc::new(
      ipc::snapshot::pg_startup_diag(
        &pg_pools.admin,
        cfg.postgres.write_pool_size,
        cfg.postgres.query_pool_size,
        cfg.postgres.admin_pool_size,
      )
      .await,
    );
    let api_metrics = Arc::new(ApiServerMetrics::default());
    let overlay_plan = index::visor::OverlayPlan::from_options(&runtime_options);

    let live = super::live::LiveVisor::new(super::live::LiveVisor::config_from_options(
      super::live::LiveVisorOptionsInput {
        pool: pg_pools.write.clone(),
        rpc: rpc_client.clone(),
        zmq: zmq_source.clone(),
        lifecycle: Arc::clone(&lifecycle),
        spent_cache: Arc::clone(&spent_cache),
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
        pool: pg_pools.query.clone(),
        address_pool: pg_pools.heavy_query.clone(),
        db: Arc::clone(&db),
        secondary_stores: Some(Arc::clone(&secondary_stores)),
        readiness_cache: Arc::clone(&readiness_cache),
        spent_cache: Arc::clone(&spent_cache),
        address_hot_cache_threshold: options.cache.address_hot_cache_threshold,
        address_summary_catchup_lag: options.cache.address_summary_catchup_lag,
        p2pkh_version: cfg.chain.p2pkh_version,
        p2sh_version: cfg.chain.p2sh_version,
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
        overlays: overlay_plan.capabilities,
        recent_mempool_txs: runtime_options.recent_mempool_txs,
        stream_hub: live_handles.stream_hub.clone(),
        stream_heartbeat_secs: runtime_options.stream_heartbeat_secs,
        bind,
        cors_origin: options.api.cors_origin.clone(),
        server_metrics: Arc::clone(&api_metrics),
        lifecycle: Arc::clone(&lifecycle),
      }))
    } else {
      None
    };

    let ipc = super::ipc::IpcVisor::new(super::ipc::IpcVisorConfig {
      socket_path: datadir.join("hyphen.sock"),
      heartbeat_db_path: datadir.join("blocks.db"),
      options: runtime_options,
      write_pool: pg_pools.write.clone(),
      query_pool: pg_pools.query.clone(),
      admin_pool: pg_pools.admin.clone(),
      db,
      secondary_stores,
      source,
      rpc: rpc_client,
      zmq: zmq_source,
      event_tx,
      lifecycle: Arc::clone(&lifecycle),
      ingest_runtime: Arc::clone(&core_handles.ingest_runtime),
      block_worker: Arc::clone(&core_handles.block_worker),
      batch_registry: core_handles.batch_registry.clone(),
      config_snapshot,
      pg_diag_base,
      api_metrics,
      readiness_cache,
      controls: Arc::clone(&core_handles.controls),
      revalidate_schema: Arc::clone(&core_handles.revalidate_schema),
      control_plane_pause_active: Arc::clone(&core_handles.control_plane_pause_active),
      operator_pause_requested: Arc::clone(&core_handles.operator_pause_requested),
      pausing_tx: core_handles.pausing_tx.clone(),
      suspend_tx: core_handles.suspend_tx.clone(),
      pausing_rx: core_handles.pausing_rx.clone(),
      suspend_rx: core_handles.suspend_rx.clone(),
      chain_state_lock: Arc::clone(&core_handles.chain_state_lock),
      p2pkh_version: cfg.chain.p2pkh_version,
      p2sh_version: cfg.chain.p2sh_version,
      block_time_secs: cfg.chain.block_time_secs,
    });

    Ok(Self {
      lifecycle,
      admin_pool: pg_pools.admin.clone(),
      core,
      live,
      ipc,
      rest,
    })
  }

  /// Start all owned subsystem supervisors.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` when any subsystem fails startup.
  pub async fn start_all(&mut self) -> Result<(), DaemonError> {
    self.core.start().await?;
    self.live.start().await?;
    if let Some(rest) = self.rest.as_mut() {
      rest.start().await?;
    }
    self.ipc.start().await?;
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
    match crate::sql::cancel_daemon_backends(&self.admin_pool).await {
      Ok(cancelled) if cancelled > 0 => {
        info!(
          cancelled,
          "cancelled active postgres statements for shutdown"
        );
      }
      Ok(_) => {}
      Err(error) => {
        warn!(%error, "failed to cancel active postgres statements during shutdown");
      }
    }
    self.ipc.stop(sigterm, graceful_deadline).await?;
    if let Some(rest) = self.rest.as_mut() {
      rest.stop(sigterm, graceful_deadline).await?;
    }
    self.live.stop(sigterm, graceful_deadline).await?;
    self.core.stop(sigterm, graceful_deadline).await?;
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
