//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Wire-shaped IPC supervisor.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use rocksdb::DB;
use tokio::sync::{broadcast, mpsc};

use crate::api_metrics::ApiServerMetrics;
use crate::error::DaemonError;
use crate::index;
use crate::index::batch::BatchSizerRegistry;
use crate::index::BlockIndexerWorker;
use crate::ingest::IngestRuntime;
use crate::init::Options;
use crate::ipc::verb::CommandCtx;
use crate::lifecycle::Lifecycle;
use crate::readiness::ReadinessCache;
use crate::sources::{CoreRpcClient, CoreZmqSource, RestSource};
use crate::store::manager::SecondaryStoreSet;
use crate::visor::{
  await_supervisable_task, spawn_supervised_task, InterruptGate, SupervisedHandle, Visor,
};
use hyphend::wire;

/// Inputs needed to construct the IPC supervisor.
pub(crate) struct IpcVisorConfig {
  pub socket_path: PathBuf,
  pub heartbeat_db_path: PathBuf,
  pub options: Options,
  pub write_pool: crate::sql::SqlPool,
  pub query_pool: crate::sql::SqlPool,
  pub admin_pool: crate::sql::SqlPool,
  pub db: Arc<DB>,
  pub secondary_stores: Arc<SecondaryStoreSet>,
  pub source: Arc<dyn RestSource>,
  pub rpc: Option<Arc<CoreRpcClient>>,
  pub zmq: Option<Arc<CoreZmqSource>>,
  pub event_tx: broadcast::Sender<wire::msg::DaemonMessage>,
  pub lifecycle: Arc<Lifecycle>,
  pub ingest_runtime: Arc<IngestRuntime>,
  pub block_worker: Arc<BlockIndexerWorker>,
  pub batch_registry: BatchSizerRegistry,
  pub config_snapshot: Arc<wire::msg::ConfigSnapshot>,
  pub pg_diag_base: Arc<wire::msg::PostgresDiag>,
  pub api_metrics: Arc<ApiServerMetrics>,
  pub readiness_cache: Arc<ReadinessCache>,
  pub controls:
    Arc<tokio::sync::Mutex<std::collections::HashMap<index::Stage, index::IndexerControl>>>,
  pub revalidate_schema: Arc<std::sync::atomic::AtomicBool>,
  pub control_plane_pause_active: Arc<std::sync::atomic::AtomicBool>,
  pub operator_pause_requested: Arc<std::sync::atomic::AtomicBool>,
  pub pausing_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  pub suspend_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  pub pausing_rx: tokio::sync::watch::Receiver<std::collections::BTreeSet<String>>,
  pub suspend_rx: tokio::sync::watch::Receiver<std::collections::BTreeSet<String>>,
  pub chain_state_lock: Arc<tokio::sync::RwLock<()>>,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub block_time_secs: f64,
}

impl std::fmt::Debug for IpcVisorConfig {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("IpcVisorConfig").finish_non_exhaustive()
  }
}

/// Owned IPC runtime supervisor.
pub(crate) struct IpcVisor {
  cfg: IpcVisorConfig,
  cmd_tx: mpsc::Sender<wire::msg::DaemonCommand>,
  cmd_rx: Option<mpsc::Receiver<wire::msg::DaemonCommand>>,
  ipc_task: Option<tokio::task::JoinHandle<()>>,
  ipc_worker: Option<SupervisedHandle<crate::ipc::IpcWorker>>,
  heartbeat_task: Option<tokio::task::JoinHandle<()>>,
  heartbeat_worker: Option<SupervisedHandle<crate::ipc::IpcWorker>>,
  command_task: Option<tokio::task::JoinHandle<()>>,
  command_worker: Option<SupervisedHandle<crate::ipc::IpcWorker>>,
}

impl std::fmt::Debug for IpcVisor {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("IpcVisor").finish_non_exhaustive()
  }
}

impl IpcVisor {
  /// Default command channel capacity for daemon IPC.
  pub const COMMAND_CHANNEL_CAPACITY: usize = 64;

  /// Build one IPC supervisor around the resolved runtime config.
  pub(crate) fn new(cfg: IpcVisorConfig) -> Self {
    let (cmd_tx, cmd_rx) = mpsc::channel(Self::COMMAND_CHANNEL_CAPACITY);
    Self {
      cfg,
      cmd_tx,
      cmd_rx: Some(cmd_rx),
      ipc_task: None,
      ipc_worker: None,
      heartbeat_task: None,
      heartbeat_worker: None,
      command_task: None,
      command_worker: None,
    }
  }
}

impl InterruptGate for IpcVisor {
  fn interrupt(&self) {
    self.cfg.lifecycle.interrupt();
  }

  fn interrupt_point(&self) -> bool {
    self.cfg.lifecycle.interruption_point()
  }
}

#[async_trait]
impl Visor for IpcVisor {
  async fn start(&mut self) -> Result<(), DaemonError> {
    if !self.interrupt_point() {
      return Ok(());
    }

    if self.ipc_task.is_none() {
      let spec = crate::ipc::server_supervised_task(crate::ipc::IpcServerWorkerTaskConfig {
        socket_path: self.cfg.socket_path.clone(),
        event_tx: self.cfg.event_tx.clone(),
        cmd_tx: self.cmd_tx.clone(),
        shutdown_rx: self.cfg.lifecycle.subscribe(),
        lifecycle: Arc::clone(&self.cfg.lifecycle),
      });
      let handle = SupervisedHandle::new(spec.initial.worker.clone());
      self.ipc_worker = Some(handle.clone());
      self.ipc_task = Some(spawn_supervised_task(
        "ipc",
        spec.name,
        Arc::clone(&self.cfg.lifecycle),
        handle,
        spec.initial,
        move || (spec.rebuild)(),
      ));
    }

    if self.heartbeat_task.is_none() {
      let spec = crate::ipc::heartbeat_supervised_task(crate::ipc::HeartbeatWorkerTaskConfig {
        options: self.cfg.options.clone(),
        write_pool: self.cfg.write_pool.clone(),
        query_pool: self.cfg.query_pool.clone(),
        admin_pool: self.cfg.admin_pool.clone(),
        db: Arc::clone(&self.cfg.db),
        secondary_stores: Arc::clone(&self.cfg.secondary_stores),
        db_path: self.cfg.heartbeat_db_path.clone(),
        source: Arc::clone(&self.cfg.source),
        rpc: self.cfg.rpc.clone(),
        zmq: self.cfg.zmq.clone(),
        event_tx: self.cfg.event_tx.clone(),
        lifecycle: Arc::clone(&self.cfg.lifecycle),
        ingestion_runtime: Arc::clone(&self.cfg.ingest_runtime),
        block_worker: Arc::clone(&self.cfg.block_worker),
        batch_registry: self.cfg.batch_registry.clone(),
        config_snapshot: Arc::clone(&self.cfg.config_snapshot),
        pg_diag_base: Arc::clone(&self.cfg.pg_diag_base),
        api_metrics: Arc::clone(&self.cfg.api_metrics),
        readiness_cache: Arc::clone(&self.cfg.readiness_cache),
        revalidate_schema: Arc::clone(&self.cfg.revalidate_schema),
        pausing_rx: self.cfg.pausing_rx.clone(),
        suspend_rx: self.cfg.suspend_rx.clone(),
      });
      let handle = SupervisedHandle::new(spec.initial.worker.clone());
      self.heartbeat_worker = Some(handle.clone());
      self.heartbeat_task = Some(spawn_supervised_task(
        "ipc",
        spec.name,
        Arc::clone(&self.cfg.lifecycle),
        handle,
        spec.initial,
        move || (spec.rebuild)(),
      ));
    }

    if self.command_task.is_none() {
      let cmd_rx = self
        .cmd_rx
        .take()
        .ok_or_else(|| DaemonError::Config("ipc command receiver already consumed".to_owned()))?;
      let ctx = CommandCtx {
        pool: self.cfg.write_pool.clone(),
        db: Arc::clone(&self.cfg.db),
        secondary_stores: Arc::clone(&self.cfg.secondary_stores),
        event_tx: self.cfg.event_tx.clone(),
        controls: Arc::clone(&self.cfg.controls),
        pausing_tx: self.cfg.pausing_tx.clone(),
        suspend_tx: self.cfg.suspend_tx.clone(),
        revalidate_schema: Arc::clone(&self.cfg.revalidate_schema),
        ingestion_monitor: Arc::clone(&self.cfg.ingest_runtime),
        block_worker: Arc::clone(&self.cfg.block_worker),
        control_plane_pause_active: Arc::clone(&self.cfg.control_plane_pause_active),
        operator_pause_requested: Arc::clone(&self.cfg.operator_pause_requested),
        chain_state_lock: Arc::clone(&self.cfg.chain_state_lock),
        p2pkh_version: self.cfg.p2pkh_version,
        p2sh_version: self.cfg.p2sh_version,
        block_time_secs: self.cfg.block_time_secs,
      };
      let spec = crate::ipc::command_supervised_task(crate::ipc::CommandWorkerTaskConfig {
        cmd_rx,
        ctx,
        lifecycle: Arc::clone(&self.cfg.lifecycle),
      });
      let handle = SupervisedHandle::new(spec.initial.worker.clone());
      self.command_worker = Some(handle.clone());
      self.command_task = Some(spawn_supervised_task(
        "ipc",
        spec.name,
        Arc::clone(&self.cfg.lifecycle),
        handle,
        spec.initial,
        move || (spec.rebuild)(),
      ));
    }

    Ok(())
  }

  async fn stop(
    &mut self,
    sigterm: &mut tokio::signal::unix::Signal,
    graceful_deadline: Option<tokio::time::Instant>,
  ) -> Result<(), DaemonError> {
    if let (Some(task), Some(worker)) = (self.command_task.take(), self.command_worker.take()) {
      await_supervisable_task(
        "ipc",
        "command loop",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (self.heartbeat_task.take(), self.heartbeat_worker.take()) {
      await_supervisable_task(
        "ipc",
        "heartbeat",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (self.ipc_task.take(), self.ipc_worker.take()) {
      await_supervisable_task(
        "ipc",
        "ipc server",
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
