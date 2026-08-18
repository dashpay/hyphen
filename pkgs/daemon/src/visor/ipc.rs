//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Wire-shaped IPC supervisor: the Unix-socket server the console connects
//! to, the heartbeat snapshot broadcaster, and the operator command loop.

use std::path::PathBuf;
use std::sync::Arc;

use async_trait::async_trait;
use hyphen_db::{HyDbIngest, HyphenDb};
use tokio::sync::{broadcast, mpsc};

use crate::api_metrics::ApiServerMetrics;
use crate::error::DaemonError;
use crate::ingest::IngestStatus;
use crate::init::Options;
use crate::ipc::verb::CommandCtx;
use crate::lifecycle::Lifecycle;
use crate::sources::{CoreRpcClient, CoreZmqSource, RestSource};
use crate::visor::{
  await_supervisable_task, spawn_supervised_task, InterruptGate, SupervisedHandle, Visor,
};
use hyphend::wire;

/// Inputs needed to construct the IPC supervisor.
pub(crate) struct IpcVisorConfig {
  pub socket_path: PathBuf,
  pub options: Options,
  pub hydb: HyphenDb,
  pub ingest: Arc<HyDbIngest>,
  pub ingest_status: Arc<IngestStatus>,
  pub progress_rx: tokio::sync::watch::Receiver<hyphen_db::HyDbProgress>,
  pub source: Arc<dyn RestSource>,
  pub rpc: Option<Arc<CoreRpcClient>>,
  pub zmq: Option<Arc<CoreZmqSource>>,
  pub event_tx: broadcast::Sender<wire::msg::DaemonMessage>,
  pub lifecycle: Arc<Lifecycle>,
  pub config_snapshot: Arc<wire::msg::ConfigSnapshot>,
  pub pg_diag_base: Arc<wire::msg::PostgresDiag>,
  pub api_metrics: Arc<ApiServerMetrics>,
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
        hydb: self.cfg.hydb.clone(),
        source: Arc::clone(&self.cfg.source),
        rpc: self.cfg.rpc.clone(),
        zmq: self.cfg.zmq.clone(),
        event_tx: self.cfg.event_tx.clone(),
        lifecycle: Arc::clone(&self.cfg.lifecycle),
        ingest_status: Arc::clone(&self.cfg.ingest_status),
        progress_rx: self.cfg.progress_rx.clone(),
        config_snapshot: Arc::clone(&self.cfg.config_snapshot),
        pg_diag_base: Arc::clone(&self.cfg.pg_diag_base),
        api_metrics: Arc::clone(&self.cfg.api_metrics),
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
        hydb: self.cfg.hydb.clone(),
        ingest: Arc::clone(&self.cfg.ingest),
        event_tx: self.cfg.event_tx.clone(),
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
