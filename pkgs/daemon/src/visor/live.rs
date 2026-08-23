//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! RPC/ZMQ-backed live supervisor.

use std::sync::Arc;

use async_trait::async_trait;

use crate::error::DaemonError;
use crate::index::visor::OverlayPlan;
use crate::init::Options;
use crate::lifecycle::Lifecycle;
use crate::live::state::LiveState;
use crate::live::StreamHub;
use crate::sources::{CoreRpcClient, CoreZmqSource};
use crate::visor::{
  await_supervisable_task, spawn_supervised_task, InterruptGate, SupervisedHandle, Visor,
};

/// Inputs needed to construct the live supervisor.
pub(crate) struct LiveVisorConfig {
  pub hydb: hyphen_db::HyphenDb,
  pub rpc: Option<Arc<CoreRpcClient>>,
  pub zmq: Option<Arc<CoreZmqSource>>,
  pub lifecycle: Arc<Lifecycle>,
  /// Shared with the mempool worker, which needs to know whether the
  /// projection has reached the node before its queries can be answered.
  pub ingest_status: Arc<crate::ingest::IngestStatus>,
  pub mempool_enabled: bool,
  pub islock_enabled: bool,
  pub network_enabled: bool,
  pub governance_enabled: bool,
  pub stream_enabled: bool,
  pub chainlock_enabled: bool,
  pub protx_enabled: bool,
  pub recent_mempool_txs: usize,
  pub network_refresh_secs: u64,
  pub governance_refresh_secs: u64,
  pub chainlock_refresh_secs: u64,
  pub protx_refresh_secs: u64,
  pub chainlock_start_block: Option<u32>,
  pub stream_client_buffer: usize,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
}

impl std::fmt::Debug for LiveVisorConfig {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("LiveVisorConfig").finish_non_exhaustive()
  }
}

/// Inputs needed to resolve one live supervisor config from startup options.
pub(crate) struct LiveVisorOptionsInput {
  pub hydb: hyphen_db::HyphenDb,
  pub rpc: Option<Arc<CoreRpcClient>>,
  pub zmq: Option<Arc<CoreZmqSource>>,
  pub lifecycle: Arc<Lifecycle>,
  pub ingest_status: Arc<crate::ingest::IngestStatus>,
  pub overlay_plan: OverlayPlan,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
}

impl std::fmt::Debug for LiveVisorOptionsInput {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("LiveVisorOptionsInput")
      .finish_non_exhaustive()
  }
}

/// Shared runtime handles exported by the live supervisor.
#[derive(Clone, Debug)]
pub(crate) struct LiveHandles {
  pub stream_hub: Option<Arc<StreamHub>>,
  /// In-RAM live state — written by the refresh workers, read by REST handlers.
  pub live_state: Arc<LiveState>,
}

#[derive(Debug)]
struct LiveTasks {
  mempool: Option<tokio::task::JoinHandle<()>>,
  mempool_recent: Option<tokio::task::JoinHandle<()>>,
  islock: Option<tokio::task::JoinHandle<()>>,
  network: Option<tokio::task::JoinHandle<()>>,
  chainlock: Option<tokio::task::JoinHandle<()>>,
  protx: Option<tokio::task::JoinHandle<()>>,
  governance: Option<tokio::task::JoinHandle<()>>,
  governance_zmq: Option<tokio::task::JoinHandle<()>>,
  live_zmq: Option<tokio::task::JoinHandle<()>>,
  stream: Option<tokio::task::JoinHandle<()>>,
}

impl LiveTasks {
  fn empty() -> Self {
    Self {
      mempool: None,
      mempool_recent: None,
      islock: None,
      network: None,
      chainlock: None,
      protx: None,
      governance: None,
      governance_zmq: None,
      live_zmq: None,
      stream: None,
    }
  }
}

#[derive(Debug)]
struct LiveWorkers {
  mempool: Option<SupervisedHandle<crate::live::LiveWorker>>,
  mempool_recent: Option<SupervisedHandle<crate::live::LiveWorker>>,
  islock: Option<SupervisedHandle<crate::live::LiveWorker>>,
  network: Option<SupervisedHandle<crate::live::LiveWorker>>,
  chainlock: Option<SupervisedHandle<crate::live::LiveWorker>>,
  protx: Option<SupervisedHandle<crate::live::LiveWorker>>,
  governance: Option<SupervisedHandle<crate::live::LiveWorker>>,
  governance_zmq: Option<SupervisedHandle<crate::live::LiveWorker>>,
  live_zmq: Option<SupervisedHandle<crate::live::LiveWorker>>,
  stream: Option<SupervisedHandle<crate::live::LiveWorker>>,
}

impl LiveWorkers {
  fn empty() -> Self {
    Self {
      mempool: None,
      mempool_recent: None,
      islock: None,
      network: None,
      chainlock: None,
      protx: None,
      governance: None,
      governance_zmq: None,
      live_zmq: None,
      stream: None,
    }
  }
}

/// Owned live runtime supervisor.
pub(crate) struct LiveVisor {
  cfg: LiveVisorConfig,
  handles: LiveHandles,
  tasks: LiveTasks,
  workers: LiveWorkers,
}

impl std::fmt::Debug for LiveVisor {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("LiveVisor").finish_non_exhaustive()
  }
}

impl LiveVisor {
  /// Build one resolved live supervisor config from validated startup options.
  pub(crate) fn config_from_options(
    input: LiveVisorOptionsInput,
    options: &Options,
  ) -> LiveVisorConfig {
    LiveVisorConfig {
      hydb: input.hydb,
      rpc: input.rpc,
      zmq: input.zmq,
      lifecycle: input.lifecycle,
      ingest_status: input.ingest_status,
      mempool_enabled: options.live.mempool,
      islock_enabled: options.live.islock,
      network_enabled: options.live.network,
      governance_enabled: options.live.governance,
      stream_enabled: options.live.stream,
      chainlock_enabled: input.overlay_plan.live_requirements.chainlock,
      protx_enabled: input.overlay_plan.live_requirements.protx,
      recent_mempool_txs: options.recent_mempool_txs,
      network_refresh_secs: options.network_refresh_secs,
      governance_refresh_secs: options.governance_refresh_secs,
      chainlock_refresh_secs: options.chainlock_refresh_secs,
      protx_refresh_secs: options.protx_refresh_secs,
      chainlock_start_block: options.chainlock_start_block,
      stream_client_buffer: options.stream_client_buffer,
      p2pkh_version: input.p2pkh_version,
      p2sh_version: input.p2sh_version,
    }
  }

  /// Build one live supervisor around the resolved runtime config.
  pub(crate) fn new(cfg: LiveVisorConfig) -> Self {
    let stream_hub = cfg
      .stream_enabled
      .then(|| Arc::new(StreamHub::new(cfg.stream_client_buffer)));
    let live_state = Arc::new(LiveState::new());
    Self {
      cfg,
      handles: LiveHandles {
        stream_hub,
        live_state,
      },
      tasks: LiveTasks::empty(),
      workers: LiveWorkers::empty(),
    }
  }

  /// Return shared live handles for the rest of the daemon.
  pub(crate) fn handles(&self) -> LiveHandles {
    self.handles.clone()
  }
}

fn spawn_live_supervised_task(
  lifecycle: Arc<Lifecycle>,
  spec: crate::worker::SupervisedTaskSpec<crate::live::LiveWorker>,
) -> (
  tokio::task::JoinHandle<()>,
  SupervisedHandle<crate::live::LiveWorker>,
) {
  let handle = SupervisedHandle::new(spec.initial.worker.clone());
  let task = spawn_supervised_task(
    "live",
    spec.name,
    lifecycle,
    handle.clone(),
    spec.initial,
    move || (spec.rebuild)(),
  );
  (task, handle)
}

impl InterruptGate for LiveVisor {
  fn interrupt(&self) {
    self.cfg.lifecycle.interrupt();
  }

  fn interrupt_point(&self) -> bool {
    self.cfg.lifecycle.interruption_point()
  }
}

#[async_trait]
impl Visor for LiveVisor {
  async fn start(&mut self) -> Result<(), DaemonError> {
    if !self.interrupt_point() {
      return Ok(());
    }

    let live_zmq_tx = if self.cfg.zmq.is_some() {
      let (tx, _) = crate::live::event_channel();
      Some(tx)
    } else {
      None
    };

    if self.cfg.mempool_enabled {
      let rpc = self.cfg.rpc.clone().ok_or_else(|| {
        DaemonError::Config("live.enable_mempool requires core.rpc.enabled".to_owned())
      })?;
      let spec =
        crate::live::mempool_summary_supervised_task(crate::live::MempoolSummaryWorkerTaskConfig {
          db: self.cfg.hydb.clone(),
          live_state: Arc::clone(&self.handles.live_state),
          rpc,
          lifecycle: Arc::clone(&self.cfg.lifecycle),
          ingest_status: Arc::clone(&self.cfg.ingest_status),
          recent_mempool_txs: self.cfg.recent_mempool_txs,
          p2pkh_version: self.cfg.p2pkh_version,
          p2sh_version: self.cfg.p2sh_version,
        });
      let (task, worker) = spawn_live_supervised_task(Arc::clone(&self.cfg.lifecycle), spec);
      self.tasks.mempool = Some(task);
      self.workers.mempool = Some(worker);
    }

    if self.cfg.network_enabled {
      let rpc = self.cfg.rpc.clone().ok_or_else(|| {
        DaemonError::Config("live.enable_network requires core.rpc.enabled".to_owned())
      })?;
      let spec = crate::live::network_supervised_task(crate::live::NetworkWorkerTaskConfig {
        live_state: Arc::clone(&self.handles.live_state),
        rpc,
        lifecycle: Arc::clone(&self.cfg.lifecycle),
        refresh_secs: self.cfg.network_refresh_secs,
        stream_hub: self.handles.stream_hub.clone(),
      });
      let (task, worker) = spawn_live_supervised_task(Arc::clone(&self.cfg.lifecycle), spec);
      self.tasks.network = Some(task);
      self.workers.network = Some(worker);
    }

    if self.cfg.chainlock_enabled {
      let rpc = self.cfg.rpc.clone().ok_or_else(|| {
        DaemonError::Config("chainlock live/overlay requires core.rpc.enabled".to_owned())
      })?;
      let start_block = self.cfg.chainlock_start_block.ok_or_else(|| {
        DaemonError::Config("chainlock live/overlay requires live.chainlock_start_block".to_owned())
      })?;
      let spec = crate::live::chainlock_supervised_task(crate::live::ChainlockWorkerTaskConfig {
        live_state: Arc::clone(&self.handles.live_state),
        rpc,
        lifecycle: Arc::clone(&self.cfg.lifecycle),
        refresh_secs: self.cfg.chainlock_refresh_secs,
        start_block,
        stream_hub: self.handles.stream_hub.clone(),
      });
      let (task, worker) = spawn_live_supervised_task(Arc::clone(&self.cfg.lifecycle), spec);
      self.tasks.chainlock = Some(task);
      self.workers.chainlock = Some(worker);
    }

    if self.cfg.protx_enabled {
      let rpc = self.cfg.rpc.clone().ok_or_else(|| {
        DaemonError::Config("overlays.enable_protx requires core.rpc.enabled".to_owned())
      })?;
      let spec = crate::live::protx_supervised_task(crate::live::ProtxWorkerTaskConfig {
        live_state: Arc::clone(&self.handles.live_state),
        rpc,
        lifecycle: Arc::clone(&self.cfg.lifecycle),
        refresh_secs: self.cfg.protx_refresh_secs,
      });
      let (task, worker) = spawn_live_supervised_task(Arc::clone(&self.cfg.lifecycle), spec);
      self.tasks.protx = Some(task);
      self.workers.protx = Some(worker);
    }

    if self.cfg.governance_enabled {
      let rpc = self.cfg.rpc.clone().ok_or_else(|| {
        DaemonError::Config("live.enable_governance requires core.rpc.enabled".to_owned())
      })?;
      let spec = crate::live::governance_supervised_task(crate::live::GovernanceWorkerTaskConfig {
        live_state: Arc::clone(&self.handles.live_state),
        rpc,
        lifecycle: Arc::clone(&self.cfg.lifecycle),
        refresh_secs: self.cfg.governance_refresh_secs,
        stream_hub: self.handles.stream_hub.clone(),
      });
      let (task, worker) = spawn_live_supervised_task(Arc::clone(&self.cfg.lifecycle), spec);
      self.tasks.governance = Some(task);
      self.workers.governance = Some(worker);
    }

    if let Some(zmq) = self.cfg.zmq.clone() {
      let tx = live_zmq_tx.clone().ok_or_else(|| {
        DaemonError::Config("live zmq fanout requires core.zmq.enabled".to_owned())
      })?;
      let spec = crate::live::live_zmq_supervised_task(crate::live::LiveZmqWorkerTaskConfig {
        zmq,
        event_tx: tx,
        lifecycle: Arc::clone(&self.cfg.lifecycle),
      });
      let (task, worker) = spawn_live_supervised_task(Arc::clone(&self.cfg.lifecycle), spec);
      self.tasks.live_zmq = Some(task);
      self.workers.live_zmq = Some(worker);
    }

    if self.cfg.stream_enabled {
      let tx = live_zmq_tx.as_ref().ok_or_else(|| {
        DaemonError::Config("live.enable_stream requires core.zmq.enabled".to_owned())
      })?;
      let stream_hub = self.handles.stream_hub.clone().ok_or_else(|| {
        DaemonError::Config("live.enable_stream requires stream hub initialization".to_owned())
      })?;
      let spec = crate::live::stream_supervised_task(
        tx.clone(),
        stream_hub,
        Arc::clone(&self.cfg.lifecycle),
      );
      let (task, worker) = spawn_live_supervised_task(Arc::clone(&self.cfg.lifecycle), spec);
      self.tasks.stream = Some(task);
      self.workers.stream = Some(worker);
    }

    if self.cfg.mempool_enabled {
      let tx = live_zmq_tx.as_ref().ok_or_else(|| {
        DaemonError::Config("live.enable_mempool requires core.zmq.enabled".to_owned())
      })?;
      let spec = crate::live::mempool_recent_supervised_task(
        tx.clone(),
        self.cfg.hydb.clone(),
        Arc::clone(&self.handles.live_state),
        Arc::clone(&self.cfg.lifecycle),
        self.cfg.recent_mempool_txs,
        self.cfg.p2pkh_version,
        self.cfg.p2sh_version,
      );
      let (task, worker) = spawn_live_supervised_task(Arc::clone(&self.cfg.lifecycle), spec);
      self.tasks.mempool_recent = Some(task);
      self.workers.mempool_recent = Some(worker);
    }

    if self.cfg.islock_enabled {
      let tx = live_zmq_tx.as_ref().ok_or_else(|| {
        DaemonError::Config("live.enable_islock requires core.zmq.enabled".to_owned())
      })?;
      let spec = crate::live::islock_supervised_task(
        tx.clone(),
        Arc::clone(&self.handles.live_state),
        Arc::clone(&self.cfg.lifecycle),
      );
      let (task, worker) = spawn_live_supervised_task(Arc::clone(&self.cfg.lifecycle), spec);
      self.tasks.islock = Some(task);
      self.workers.islock = Some(worker);
    }

    if self.cfg.governance_enabled && self.cfg.zmq.is_some() {
      let rpc = self.cfg.rpc.clone().ok_or_else(|| {
        DaemonError::Config("live.enable_governance requires core.rpc.enabled".to_owned())
      })?;
      let tx = live_zmq_tx.as_ref().ok_or_else(|| {
        DaemonError::Config("governance zmq refresh requires core.zmq.enabled".to_owned())
      })?;
      let spec = crate::live::governance_zmq_supervised_task(
        tx.clone(),
        Arc::clone(&self.handles.live_state),
        rpc,
        Arc::clone(&self.cfg.lifecycle),
        self.handles.stream_hub.clone(),
      );
      let (task, worker) = spawn_live_supervised_task(Arc::clone(&self.cfg.lifecycle), spec);
      self.tasks.governance_zmq = Some(task);
      self.workers.governance_zmq = Some(worker);
    }

    Ok(())
  }

  async fn stop(
    &mut self,
    sigterm: &mut tokio::signal::unix::Signal,
    graceful_deadline: Option<tokio::time::Instant>,
  ) -> Result<(), DaemonError> {
    if let (Some(task), Some(worker)) = (self.tasks.mempool.take(), self.workers.mempool.take()) {
      await_supervisable_task(
        "live",
        "mempool",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (
      self.tasks.mempool_recent.take(),
      self.workers.mempool_recent.take(),
    ) {
      await_supervisable_task(
        "live",
        "mempool recent",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (self.tasks.islock.take(), self.workers.islock.take()) {
      await_supervisable_task(
        "live",
        "islock",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (self.tasks.chainlock.take(), self.workers.chainlock.take())
    {
      await_supervisable_task(
        "live",
        "chainlock",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (self.tasks.protx.take(), self.workers.protx.take()) {
      await_supervisable_task(
        "live",
        "protx",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (self.tasks.network.take(), self.workers.network.take()) {
      await_supervisable_task(
        "live",
        "network",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) =
      (self.tasks.governance.take(), self.workers.governance.take())
    {
      await_supervisable_task(
        "live",
        "governance",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (
      self.tasks.governance_zmq.take(),
      self.workers.governance_zmq.take(),
    ) {
      await_supervisable_task(
        "live",
        "governance zmq",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (self.tasks.live_zmq.take(), self.workers.live_zmq.take()) {
      await_supervisable_task(
        "live",
        "live zmq",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (self.tasks.stream.take(), self.workers.stream.take()) {
      await_supervisable_task(
        "live",
        "stream hub",
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
