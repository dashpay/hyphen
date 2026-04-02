//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! REST API supervisor.

use std::sync::Arc;

use crate::sql::SqlPool;
use async_trait::async_trait;
use rocksdb::DB;

use crate::api_metrics::ApiServerMetrics;
use crate::error::DaemonError;
use crate::lifecycle::Lifecycle;
use crate::readiness::ReadinessCache;
use crate::visor::{
  await_supervisable_task, spawn_supervised_task, InterruptGate, SupervisedHandle, Visor,
};

use crate::index::spent::cache::SpentQueryCache;
use crate::live::stream::StreamHub;
use crate::rest::{AppState, IndexCapabilities, LiveCapabilities, OverlayCapabilities, RestWorker};

/// Inputs needed to construct the REST supervisor.
pub(crate) struct RestVisorConfig {
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
  pub recent_mempool_txs: usize,
  pub stream_hub: Option<Arc<StreamHub>>,
  pub stream_heartbeat_secs: u64,
  pub bind: String,
  pub cors_origin: Option<String>,
  pub server_metrics: Arc<ApiServerMetrics>,
  pub lifecycle: Arc<Lifecycle>,
}

impl std::fmt::Debug for RestVisorConfig {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("RestVisorConfig").finish_non_exhaustive()
  }
}

/// Owned REST runtime supervisor.
pub(crate) struct RestVisor {
  cfg: RestVisorConfig,
  api_task: Option<tokio::task::JoinHandle<()>>,
  api_worker: Option<SupervisedHandle<RestWorker>>,
}

impl std::fmt::Debug for RestVisor {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("RestVisor").finish_non_exhaustive()
  }
}

impl RestVisor {
  /// Build one REST supervisor around the resolved runtime config.
  pub(crate) fn new(cfg: RestVisorConfig) -> Self {
    Self {
      cfg,
      api_task: None,
      api_worker: None,
    }
  }

  fn app_state(&self) -> AppState {
    AppState {
      pool: self.cfg.pool.clone(),
      address_pool: self.cfg.address_pool.clone(),
      db: Arc::clone(&self.cfg.db),
      secondary_stores: self.cfg.secondary_stores.clone(),
      readiness_cache: Arc::clone(&self.cfg.readiness_cache),
      spent_cache: Arc::clone(&self.cfg.spent_cache),
      address_hot_cache_threshold: self.cfg.address_hot_cache_threshold,
      address_summary_catchup_lag: self.cfg.address_summary_catchup_lag,
      p2pkh_version: self.cfg.p2pkh_version,
      p2sh_version: self.cfg.p2sh_version,
      indices: self.cfg.indices,
      live: self.cfg.live,
      overlays: self.cfg.overlays,
      recent_mempool_txs: self.cfg.recent_mempool_txs,
      stream_hub: self.cfg.stream_hub.clone(),
      stream_heartbeat_secs: self.cfg.stream_heartbeat_secs,
    }
  }
}

impl InterruptGate for RestVisor {
  fn interrupt(&self) {
    self.cfg.lifecycle.interrupt();
  }

  fn interrupt_point(&self) -> bool {
    self.cfg.lifecycle.interruption_point()
  }
}

#[async_trait]
impl Visor for RestVisor {
  async fn start(&mut self) -> Result<(), DaemonError> {
    if self.api_task.is_some() || !self.interrupt_point() {
      return Ok(());
    }

    let spec = crate::rest::api_supervised_task(crate::rest::ApiWorkerTaskConfig {
      state: self.app_state(),
      bind: self.cfg.bind.clone(),
      shutdown_rx: self.cfg.lifecycle.subscribe(),
      server_metrics: Arc::clone(&self.cfg.server_metrics),
      cors_origin: self.cfg.cors_origin.clone(),
      lifecycle: Arc::clone(&self.cfg.lifecycle),
    });
    let handle = SupervisedHandle::new(spec.initial.worker.clone());
    self.api_worker = Some(handle.clone());
    self.api_task = Some(spawn_supervised_task(
      "rest",
      spec.name,
      Arc::clone(&self.cfg.lifecycle),
      handle,
      spec.initial,
      move || (spec.rebuild)(),
    ));
    Ok(())
  }

  async fn stop(
    &mut self,
    sigterm: &mut tokio::signal::unix::Signal,
    graceful_deadline: Option<tokio::time::Instant>,
  ) -> Result<(), DaemonError> {
    if let (Some(task), Some(worker)) = (self.api_task.take(), self.api_worker.take()) {
      await_supervisable_task(
        "rest",
        "api server",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    Ok(())
  }
}
