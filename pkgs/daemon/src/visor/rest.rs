//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! REST API supervisor.

use std::sync::Arc;

use async_trait::async_trait;

use crate::api_metrics::ApiServerMetrics;
use crate::error::DaemonError;
use crate::lifecycle::Lifecycle;
use crate::visor::{
  await_supervisable_task, spawn_supervised_task, InterruptGate, SupervisedHandle, Visor,
};

use crate::live::state::LiveState;
use crate::live::stream::StreamHub;
use crate::rest::{AppState, IndexCapabilities, LiveCapabilities, OverlayCapabilities, RestWorker};

/// Inputs needed to construct the REST supervisor.
pub(crate) struct RestVisorConfig {
  pub hydb: hyphen_db::HyphenDb,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub block_time_secs: f64,
  pub indices: IndexCapabilities,
  pub live: LiveCapabilities,
  pub overlays: OverlayCapabilities,
  pub recent_mempool_txs: usize,
  pub stream_hub: Option<Arc<StreamHub>>,
  pub live_state: Arc<LiveState>,
  pub stream_heartbeat_secs: u64,
  pub node_tip: std::sync::Arc<std::sync::atomic::AtomicI64>,
  pub ingest_progress_epoch: std::sync::Arc<std::sync::atomic::AtomicU64>,
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
      hydb: self.cfg.hydb.clone(),
      p2pkh_version: self.cfg.p2pkh_version,
      p2sh_version: self.cfg.p2sh_version,
      block_time_secs: self.cfg.block_time_secs,
      indices: self.cfg.indices,
      live: self.cfg.live,
      overlays: self.cfg.overlays,
      recent_mempool_txs: self.cfg.recent_mempool_txs,
      stream_hub: self.cfg.stream_hub.clone(),
      live_state: Arc::clone(&self.cfg.live_state),
      stream_heartbeat_secs: self.cfg.stream_heartbeat_secs,
      node_tip: std::sync::Arc::clone(&self.cfg.node_tip),
      ingest_progress_epoch: std::sync::Arc::clone(&self.cfg.ingest_progress_epoch),
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
