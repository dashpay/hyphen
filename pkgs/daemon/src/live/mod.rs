//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live current-state subsystems backed by JSON-RPC and ZMQ.
//!
//! These modules own live acquisition, refresh, persistence, and dedicated
//! live routes. They do not own indexed-response overlay composition.

use std::sync::Arc;

use async_trait::async_trait;
use tokio::sync::broadcast;

use crate::error::DaemonError;
use crate::index::spent::cache::SpentQueryCache;
use crate::lifecycle::Lifecycle;
use crate::policy::watchdog::{
  WATCHDOG_ACTIVITY_STALE_SECS, WATCHDOG_STARTING_STALE_SECS, WATCHDOG_TICK_SECS,
};
use crate::sources::{CoreRpcClient, CoreZmqSource, ZmqEvent};
use crate::visor::{
  Fenceable, ProgressState, RecoveryDisposition, RunState, ShutdownPhase, Supervisable,
};
use crate::worker::{
  runtime_watchdog_disposition, spawn_activity_ticker, SupervisedTaskSpec, WorkerRuntimeState,
  WorkerTask,
};

pub mod chainlock;
pub mod gov;
pub mod islock;
pub mod mempool;
pub mod network;
pub mod protx;
pub mod stream;
pub mod warn;
pub mod zmq;

pub use stream::StreamHub;

const MIN_RECENT_MEMPOOL_TXS_FOR_API: usize = 100;

/// Create the shared live ZMQ broadcast channel.
pub(crate) fn event_channel() -> (broadcast::Sender<ZmqEvent>, broadcast::Receiver<ZmqEvent>) {
  zmq::event_channel()
}

/// Inputs needed to construct a mempool-summary worker task.
#[derive(Clone, Debug)]
pub(crate) struct MempoolSummaryWorkerTaskConfig {
  pub pool: crate::sql::SqlPool,
  pub rpc: Arc<CoreRpcClient>,
  pub lifecycle: Arc<Lifecycle>,
  pub recent_mempool_txs: usize,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
}

/// Inputs needed to construct a network-refresh worker task.
#[derive(Clone, Debug)]
pub(crate) struct NetworkWorkerTaskConfig {
  pub pool: crate::sql::SqlPool,
  pub rpc: Arc<CoreRpcClient>,
  pub lifecycle: Arc<Lifecycle>,
  pub refresh_secs: u64,
  pub stream_hub: Option<Arc<stream::StreamHub>>,
}

/// Inputs needed to construct a chainlock-refresh worker task.
#[derive(Clone, Debug)]
pub(crate) struct ChainlockWorkerTaskConfig {
  pub pool: crate::sql::SqlPool,
  pub rpc: Arc<CoreRpcClient>,
  pub lifecycle: Arc<Lifecycle>,
  pub refresh_secs: u64,
  pub start_block: u32,
  pub stream_hub: Option<Arc<stream::StreamHub>>,
}

/// Inputs needed to construct a protx-refresh worker task.
#[derive(Clone, Debug)]
pub(crate) struct ProtxWorkerTaskConfig {
  pub pool: crate::sql::SqlPool,
  pub rpc: Arc<CoreRpcClient>,
  pub lifecycle: Arc<Lifecycle>,
  pub refresh_secs: u64,
}

/// Inputs needed to construct a governance-refresh worker task.
#[derive(Clone, Debug)]
pub(crate) struct GovernanceWorkerTaskConfig {
  pub pool: crate::sql::SqlPool,
  pub rpc: Arc<CoreRpcClient>,
  pub lifecycle: Arc<Lifecycle>,
  pub refresh_secs: u64,
  pub stream_hub: Option<Arc<stream::StreamHub>>,
}

/// Inputs needed to construct a live-zmq fanout worker task.
#[derive(Clone, Debug)]
pub(crate) struct LiveZmqWorkerTaskConfig {
  pub zmq: Arc<CoreZmqSource>,
  pub event_tx: broadcast::Sender<ZmqEvent>,
  pub lifecycle: Arc<Lifecycle>,
}

/// Inputs needed to construct a stream-hub worker task.
#[derive(Debug)]
pub(crate) struct StreamWorkerTaskConfig {
  pub rx: broadcast::Receiver<ZmqEvent>,
  pub stream_hub: Arc<stream::StreamHub>,
  pub lifecycle: Arc<Lifecycle>,
}

/// Inputs needed to construct a mempool-recent worker task.
#[derive(Debug)]
pub(crate) struct MempoolRecentWorkerTaskConfig {
  pub pool: crate::sql::SqlPool,
  pub rx: broadcast::Receiver<ZmqEvent>,
  pub lifecycle: Arc<Lifecycle>,
  pub keep: usize,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub spent_cache: Arc<SpentQueryCache>,
}

/// Inputs needed to construct an islock worker task.
#[derive(Debug)]
pub(crate) struct IslockWorkerTaskConfig {
  pub pool: crate::sql::SqlPool,
  pub rx: broadcast::Receiver<ZmqEvent>,
  pub lifecycle: Arc<Lifecycle>,
}

/// Inputs needed to construct a governance-zmq worker task.
#[derive(Debug)]
pub(crate) struct GovernanceZmqWorkerTaskConfig {
  pub pool: crate::sql::SqlPool,
  pub rpc: Arc<CoreRpcClient>,
  pub rx: broadcast::Receiver<ZmqEvent>,
  pub lifecycle: Arc<Lifecycle>,
  pub stream_hub: Option<Arc<stream::StreamHub>>,
}

#[derive(Clone, Debug)]
pub(crate) struct LiveWorker {
  lifecycle: Arc<Lifecycle>,
  phase: &'static str,
  recoverable: bool,
  runtime: Arc<WorkerRuntimeState>,
}

impl LiveWorker {
  fn new(lifecycle: Arc<Lifecycle>, phase: &'static str, recoverable: bool) -> Self {
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
impl Supervisable for LiveWorker {
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
        "live worker recovery blocked: {reason}"
      ))),
      RecoveryDisposition::Exhausted => Err(DaemonError::Config(
        "live worker recovery exhausted retry budget".to_owned(),
      )),
      RecoveryDisposition::Fatal(reason) => Err(DaemonError::Config(format!(
        "live worker recovery unavailable: {reason}"
      ))),
    }
  }
}

impl Fenceable for LiveWorker {
  fn fence(&self, _run_id: Option<Self::RunId>) {}
}

fn lifecycle_worker_task<F>(
  phase: &'static str,
  lifecycle: Arc<Lifecycle>,
  task: F,
) -> WorkerTask<LiveWorker>
where
  F: std::future::Future<Output = Result<(), DaemonError>> + Send + 'static,
{
  let worker = LiveWorker::new(Arc::clone(&lifecycle), phase, true);
  let runtime = Arc::clone(&worker.runtime);
  WorkerTask::new(worker, async move {
    runtime.mark_started();
    let ticker = spawn_activity_ticker(
      Arc::clone(&lifecycle),
      Arc::clone(&runtime),
      std::time::Duration::from_secs(WATCHDOG_TICK_SECS),
    );
    let result = task.await;
    ticker.abort();
    runtime.mark_finished();
    result
  })
}

fn mempool_summary_worker_task(cfg: MempoolSummaryWorkerTaskConfig) -> WorkerTask<LiveWorker> {
  lifecycle_worker_task("mempool_summary", Arc::clone(&cfg.lifecycle), async move {
    mempool::refresh_summary_loop(
      cfg.pool,
      cfg.rpc,
      cfg.lifecycle,
      cfg.recent_mempool_txs,
      cfg.p2pkh_version,
      cfg.p2sh_version,
    )
    .await;
    Ok(())
  })
}

#[must_use]
pub(crate) fn mempool_summary_supervised_task(
  mut cfg: MempoolSummaryWorkerTaskConfig,
) -> SupervisedTaskSpec<LiveWorker> {
  cfg.recent_mempool_txs = cfg.recent_mempool_txs.max(MIN_RECENT_MEMPOOL_TXS_FOR_API);
  SupervisedTaskSpec::new(
    "mempool summary",
    mempool_summary_worker_task(cfg.clone()),
    move || Ok(mempool_summary_worker_task(cfg.clone())),
  )
}

fn network_worker_task(cfg: NetworkWorkerTaskConfig) -> WorkerTask<LiveWorker> {
  lifecycle_worker_task("network_refresh", Arc::clone(&cfg.lifecycle), async move {
    network::refresh_loop(
      cfg.pool,
      cfg.rpc,
      cfg.lifecycle,
      cfg.refresh_secs,
      cfg.stream_hub,
    )
    .await;
    Ok(())
  })
}

#[must_use]
pub(crate) fn network_supervised_task(
  cfg: NetworkWorkerTaskConfig,
) -> SupervisedTaskSpec<LiveWorker> {
  SupervisedTaskSpec::new(
    "network refresh",
    network_worker_task(cfg.clone()),
    move || Ok(network_worker_task(cfg.clone())),
  )
}

fn chainlock_worker_task(cfg: ChainlockWorkerTaskConfig) -> WorkerTask<LiveWorker> {
  lifecycle_worker_task(
    "chainlock_refresh",
    Arc::clone(&cfg.lifecycle),
    async move {
      chainlock::refresh_loop(
        cfg.pool,
        cfg.rpc,
        cfg.lifecycle,
        cfg.refresh_secs,
        cfg.start_block,
        cfg.stream_hub,
      )
      .await;
      Ok(())
    },
  )
}

#[must_use]
pub(crate) fn chainlock_supervised_task(
  cfg: ChainlockWorkerTaskConfig,
) -> SupervisedTaskSpec<LiveWorker> {
  SupervisedTaskSpec::new(
    "chainlock refresh",
    chainlock_worker_task(cfg.clone()),
    move || Ok(chainlock_worker_task(cfg.clone())),
  )
}

fn protx_worker_task(cfg: ProtxWorkerTaskConfig) -> WorkerTask<LiveWorker> {
  lifecycle_worker_task("protx_refresh", Arc::clone(&cfg.lifecycle), async move {
    protx::refresh_loop(cfg.pool, cfg.rpc, cfg.lifecycle, cfg.refresh_secs).await;
    Ok(())
  })
}

#[must_use]
pub(crate) fn protx_supervised_task(cfg: ProtxWorkerTaskConfig) -> SupervisedTaskSpec<LiveWorker> {
  SupervisedTaskSpec::new("protx refresh", protx_worker_task(cfg.clone()), move || {
    Ok(protx_worker_task(cfg.clone()))
  })
}

fn governance_worker_task(cfg: GovernanceWorkerTaskConfig) -> WorkerTask<LiveWorker> {
  lifecycle_worker_task(
    "governance_refresh",
    Arc::clone(&cfg.lifecycle),
    async move {
      gov::refresh_loop(
        cfg.pool,
        cfg.rpc,
        cfg.lifecycle,
        cfg.refresh_secs,
        cfg.stream_hub,
      )
      .await;
      Ok(())
    },
  )
}

#[must_use]
pub(crate) fn governance_supervised_task(
  cfg: GovernanceWorkerTaskConfig,
) -> SupervisedTaskSpec<LiveWorker> {
  SupervisedTaskSpec::new(
    "governance refresh",
    governance_worker_task(cfg.clone()),
    move || Ok(governance_worker_task(cfg.clone())),
  )
}

fn live_zmq_worker_task(cfg: LiveZmqWorkerTaskConfig) -> WorkerTask<LiveWorker> {
  lifecycle_worker_task("live_zmq_fanout", Arc::clone(&cfg.lifecycle), async move {
    zmq::fanout_loop(cfg.zmq, cfg.event_tx, cfg.lifecycle).await;
    Ok(())
  })
}

#[must_use]
pub(crate) fn live_zmq_supervised_task(
  cfg: LiveZmqWorkerTaskConfig,
) -> SupervisedTaskSpec<LiveWorker> {
  SupervisedTaskSpec::new(
    "live zmq fanout",
    live_zmq_worker_task(cfg.clone()),
    move || Ok(live_zmq_worker_task(cfg.clone())),
  )
}

fn stream_worker_task(cfg: StreamWorkerTaskConfig) -> WorkerTask<LiveWorker> {
  lifecycle_worker_task("stream_hub", Arc::clone(&cfg.lifecycle), async move {
    stream::hub::fanout_loop(cfg.rx, cfg.stream_hub, cfg.lifecycle).await;
    Ok(())
  })
}

#[must_use]
pub(crate) fn stream_supervised_task(
  rx: broadcast::Sender<ZmqEvent>,
  stream_hub: Arc<stream::StreamHub>,
  lifecycle: Arc<Lifecycle>,
) -> SupervisedTaskSpec<LiveWorker> {
  let initial = stream_worker_task(StreamWorkerTaskConfig {
    rx: rx.subscribe(),
    stream_hub: Arc::clone(&stream_hub),
    lifecycle: Arc::clone(&lifecycle),
  });
  SupervisedTaskSpec::new("stream hub", initial, move || {
    Ok(stream_worker_task(StreamWorkerTaskConfig {
      rx: rx.subscribe(),
      stream_hub: Arc::clone(&stream_hub),
      lifecycle: Arc::clone(&lifecycle),
    }))
  })
}

fn mempool_recent_worker_task(cfg: MempoolRecentWorkerTaskConfig) -> WorkerTask<LiveWorker> {
  lifecycle_worker_task("mempool_recent", Arc::clone(&cfg.lifecycle), async move {
    mempool::sync_recent_loop(
      cfg.pool,
      cfg.rx,
      cfg.lifecycle,
      cfg.keep,
      cfg.p2pkh_version,
      cfg.p2sh_version,
      cfg.spent_cache,
    )
    .await;
    Ok(())
  })
}

#[must_use]
pub(crate) fn mempool_recent_supervised_task(
  rx: broadcast::Sender<ZmqEvent>,
  pool: crate::sql::SqlPool,
  lifecycle: Arc<Lifecycle>,
  keep: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
  spent_cache: Arc<SpentQueryCache>,
) -> SupervisedTaskSpec<LiveWorker> {
  let keep = keep.max(MIN_RECENT_MEMPOOL_TXS_FOR_API);
  let initial = mempool_recent_worker_task(MempoolRecentWorkerTaskConfig {
    pool: pool.clone(),
    rx: rx.subscribe(),
    lifecycle: Arc::clone(&lifecycle),
    keep,
    p2pkh_version,
    p2sh_version,
    spent_cache: Arc::clone(&spent_cache),
  });
  SupervisedTaskSpec::new("mempool recent", initial, move || {
    Ok(mempool_recent_worker_task(MempoolRecentWorkerTaskConfig {
      pool: pool.clone(),
      rx: rx.subscribe(),
      lifecycle: Arc::clone(&lifecycle),
      keep,
      p2pkh_version,
      p2sh_version,
      spent_cache: Arc::clone(&spent_cache),
    }))
  })
}

fn islock_worker_task(cfg: IslockWorkerTaskConfig) -> WorkerTask<LiveWorker> {
  lifecycle_worker_task("islock_sync", Arc::clone(&cfg.lifecycle), async move {
    islock::sync_current_loop(cfg.pool, cfg.rx, cfg.lifecycle).await;
    Ok(())
  })
}

#[must_use]
pub(crate) fn islock_supervised_task(
  rx: broadcast::Sender<ZmqEvent>,
  pool: crate::sql::SqlPool,
  lifecycle: Arc<Lifecycle>,
) -> SupervisedTaskSpec<LiveWorker> {
  let initial = islock_worker_task(IslockWorkerTaskConfig {
    pool: pool.clone(),
    rx: rx.subscribe(),
    lifecycle: Arc::clone(&lifecycle),
  });
  SupervisedTaskSpec::new("islock sync", initial, move || {
    Ok(islock_worker_task(IslockWorkerTaskConfig {
      pool: pool.clone(),
      rx: rx.subscribe(),
      lifecycle: Arc::clone(&lifecycle),
    }))
  })
}

fn governance_zmq_worker_task(cfg: GovernanceZmqWorkerTaskConfig) -> WorkerTask<LiveWorker> {
  lifecycle_worker_task("governance_zmq", Arc::clone(&cfg.lifecycle), async move {
    gov::refresh_from_zmq_loop(cfg.pool, cfg.rpc, cfg.rx, cfg.lifecycle, cfg.stream_hub).await;
    Ok(())
  })
}

#[must_use]
pub(crate) fn governance_zmq_supervised_task(
  rx: broadcast::Sender<ZmqEvent>,
  pool: crate::sql::SqlPool,
  rpc: Arc<CoreRpcClient>,
  lifecycle: Arc<Lifecycle>,
  stream_hub: Option<Arc<stream::StreamHub>>,
) -> SupervisedTaskSpec<LiveWorker> {
  let initial = governance_zmq_worker_task(GovernanceZmqWorkerTaskConfig {
    pool: pool.clone(),
    rpc: rpc.clone(),
    rx: rx.subscribe(),
    lifecycle: Arc::clone(&lifecycle),
    stream_hub: stream_hub.clone(),
  });
  SupervisedTaskSpec::new("governance zmq", initial, move || {
    Ok(governance_zmq_worker_task(GovernanceZmqWorkerTaskConfig {
      pool: pool.clone(),
      rpc: rpc.clone(),
      rx: rx.subscribe(),
      lifecycle: Arc::clone(&lifecycle),
      stream_hub: stream_hub.clone(),
    }))
  })
}
