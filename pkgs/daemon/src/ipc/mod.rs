//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Daemon-side IPC orchestration.

pub mod snapshot;
pub mod transport;
pub mod verb;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use hyphen_db::HyphenDb;
use tokio::sync::{broadcast, mpsc};

use crate::api_metrics::ApiServerMetrics;
use crate::error::DaemonError;
use crate::ingest::IngestStatus;
use crate::init::Options;
use crate::ipc::verb::CommandCtx;
use crate::lifecycle::Lifecycle;
use crate::policy::watchdog::{
  WATCHDOG_ACTIVITY_STALE_SECS, WATCHDOG_STARTING_STALE_SECS, WATCHDOG_TICK_SECS,
};
use crate::sources::{CoreRpcClient, CoreZmqSource, RestSource, RpcSource, ZmqSource};
use crate::visor::{
  Fenceable, ProgressState, RecoveryDisposition, RunState, ShutdownPhase, Supervisable,
};
use crate::worker::{
  runtime_watchdog_disposition, spawn_activity_ticker, SupervisedTaskSpec, WorkerRuntimeState,
  WorkerTask,
};
use hyphend::wire;

const HEARTBEAT_INTERVAL_SECS: u64 = WATCHDOG_TICK_SECS;
const HEARTBEAT_STALE_SECS: u64 = HEARTBEAT_INTERVAL_SECS * 5;

/// Inputs needed to construct one supervised IPC transport task.
#[derive(Clone)]
pub(crate) struct IpcServerWorkerTaskConfig {
  pub socket_path: PathBuf,
  pub event_tx: broadcast::Sender<wire::msg::DaemonMessage>,
  pub cmd_tx: mpsc::Sender<wire::msg::DaemonCommand>,
  pub shutdown_rx: tokio::sync::watch::Receiver<bool>,
  pub lifecycle: Arc<Lifecycle>,
}

impl std::fmt::Debug for IpcServerWorkerTaskConfig {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("IpcServerWorkerTaskConfig")
      .field("socket_path", &self.socket_path)
      .finish_non_exhaustive()
  }
}

/// Inputs needed to construct one supervised IPC heartbeat task.
#[derive(Clone)]
pub(crate) struct HeartbeatWorkerTaskConfig {
  pub options: Options,
  pub hydb: HyphenDb,
  pub source: Arc<dyn RestSource>,
  pub rpc: Option<Arc<CoreRpcClient>>,
  pub zmq: Option<Arc<CoreZmqSource>>,
  pub event_tx: broadcast::Sender<wire::msg::DaemonMessage>,
  pub lifecycle: Arc<Lifecycle>,
  pub ingest_status: Arc<IngestStatus>,
  pub progress_rx: tokio::sync::watch::Receiver<hyphen_db::HyDbProgress>,
  pub config_snapshot: Arc<wire::msg::ConfigSnapshot>,
  pub pg_diag_base: Arc<wire::msg::PostgresDiag>,
  pub api_metrics: Arc<ApiServerMetrics>,
}

impl std::fmt::Debug for HeartbeatWorkerTaskConfig {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("HeartbeatWorkerTaskConfig")
      .finish_non_exhaustive()
  }
}

/// Inputs needed to construct one supervised IPC command task.
pub(crate) struct CommandWorkerTaskConfig {
  pub cmd_rx: mpsc::Receiver<wire::msg::DaemonCommand>,
  pub ctx: CommandCtx,
  pub lifecycle: Arc<Lifecycle>,
}

impl std::fmt::Debug for CommandWorkerTaskConfig {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("CommandWorkerTaskConfig")
      .finish_non_exhaustive()
  }
}

#[derive(Clone, Debug)]
pub(crate) struct IpcWorker {
  lifecycle: Arc<Lifecycle>,
  phase: &'static str,
  recoverable: bool,
  runtime: Arc<WorkerRuntimeState>,
}

impl IpcWorker {
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
impl Supervisable for IpcWorker {
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
      ShutdownPhase::Running if self.phase == "heartbeat" => {
        let Some(last_activity_epoch) = self.runtime.last_activity_epoch() else {
          return RecoveryDisposition::Blocked("worker has not emitted watchdog activity");
        };
        let progress_age = self
          .runtime
          .last_progress_epoch()
          .map(|epoch| now_epoch.saturating_sub(epoch));
        let activity_age = now_epoch.saturating_sub(last_activity_epoch);
        if progress_age.is_some_and(|age| age >= HEARTBEAT_STALE_SECS)
          && activity_age >= HEARTBEAT_STALE_SECS
        {
          self.runtime.recovery_disposition(self.runtime.run_id())
        } else {
          RecoveryDisposition::NotNeeded
        }
      }
      ShutdownPhase::Running if !self.recoverable => {
        RecoveryDisposition::Blocked("worker is not restartable")
      }
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
        "ipc worker recovery blocked: {reason}"
      ))),
      RecoveryDisposition::Exhausted => Err(DaemonError::Config(
        "ipc worker recovery exhausted retry budget".to_owned(),
      )),
      RecoveryDisposition::Fatal(reason) => Err(DaemonError::Config(format!(
        "ipc worker recovery unavailable: {reason}"
      ))),
    }
  }
}

impl Fenceable for IpcWorker {
  fn fence(&self, _run_id: Option<Self::RunId>) {}
}

/// Build one IPC server worker task without exposing transport internals to the visor.
fn server_worker_task(cfg: IpcServerWorkerTaskConfig) -> WorkerTask<IpcWorker> {
  let worker = IpcWorker::new(Arc::clone(&cfg.lifecycle), "ipc_server", true);
  let runtime = Arc::clone(&worker.runtime);
  let task = async move {
    runtime.mark_started();
    let ticker = spawn_activity_ticker(
      Arc::clone(&cfg.lifecycle),
      Arc::clone(&runtime),
      Duration::from_secs(WATCHDOG_TICK_SECS),
    );
    let result = transport::serve(&cfg.socket_path, cfg.event_tx, cfg.cmd_tx, cfg.shutdown_rx)
      .await
      .map_err(DaemonError::Config);
    ticker.abort();
    runtime.mark_finished();
    result
  };
  WorkerTask::new(worker, task)
}

#[must_use]
pub(crate) fn server_supervised_task(
  cfg: IpcServerWorkerTaskConfig,
) -> SupervisedTaskSpec<IpcWorker> {
  SupervisedTaskSpec::new("ipc server", server_worker_task(cfg.clone()), move || {
    Ok(server_worker_task(cfg.clone()))
  })
}

/// Build one IPC heartbeat worker task without exposing snapshot internals to the visor.
fn heartbeat_worker_task(cfg: HeartbeatWorkerTaskConfig) -> WorkerTask<IpcWorker> {
  let worker = IpcWorker::new(Arc::clone(&cfg.lifecycle), "heartbeat", true);
  let runtime = Arc::clone(&worker.runtime);
  let task = async move {
    runtime.mark_started();
    let ticker = spawn_activity_ticker(
      Arc::clone(&cfg.lifecycle),
      Arc::clone(&runtime),
      Duration::from_secs(WATCHDOG_TICK_SECS),
    );
    let mut state = snapshot::HeartbeatState::new();
    let start_time = Instant::now();

    while cfg.lifecycle.running() {
      let ctx = snapshot::HeartbeatCtx {
        options: &cfg.options,
        hydb: &cfg.hydb,
        source: cfg.source.as_ref(),
        rpc: cfg.rpc.as_deref().map(|client| client as &dyn RpcSource),
        zmq: cfg.zmq.as_deref().map(|client| client as &dyn ZmqSource),
        ingest_status: &cfg.ingest_status,
        progress: *cfg.progress_rx.borrow(),
        config_snap: &cfg.config_snapshot,
        pg_diag_base: &cfg.pg_diag_base,
        api_metrics: &cfg.api_metrics,
        start_time,
      };
      let snapshot = snapshot::build_snapshot(&ctx, &mut state).await;
      let _ = cfg
        .event_tx
        .send(wire::msg::DaemonMessage::Snapshot(Box::new(snapshot)));
      runtime.mark_progress();

      cfg
        .lifecycle
        .sleep(Duration::from_secs(HEARTBEAT_INTERVAL_SECS))
        .await;
    }
    ticker.abort();
    runtime.mark_finished();
    Ok(())
  };
  WorkerTask::new(worker, task)
}

#[must_use]
pub(crate) fn heartbeat_supervised_task(
  cfg: HeartbeatWorkerTaskConfig,
) -> SupervisedTaskSpec<IpcWorker> {
  SupervisedTaskSpec::new("heartbeat", heartbeat_worker_task(cfg.clone()), move || {
    Ok(heartbeat_worker_task(cfg.clone()))
  })
}

/// Build one IPC command worker task. Verbs are plain async calls into the
/// storage engine now, so the loop runs on the shared runtime directly.
fn command_worker_task(cfg: CommandWorkerTaskConfig) -> WorkerTask<IpcWorker> {
  let worker = IpcWorker::new(Arc::clone(&cfg.lifecycle), "ipc_command_loop", false);
  let runtime = Arc::clone(&worker.runtime);
  let task = async move {
    runtime.mark_started();
    let ticker = spawn_activity_ticker(
      Arc::clone(&cfg.lifecycle),
      Arc::clone(&runtime),
      Duration::from_secs(WATCHDOG_TICK_SECS),
    );
    verb::handle_commands(cfg.cmd_rx, cfg.ctx, Arc::clone(&cfg.lifecycle)).await;
    ticker.abort();
    runtime.mark_finished();
    Ok(())
  };
  WorkerTask::new(worker, task)
}

#[must_use]
pub(crate) fn command_supervised_task(
  cfg: CommandWorkerTaskConfig,
) -> SupervisedTaskSpec<IpcWorker> {
  let initial = command_worker_task(cfg);
  SupervisedTaskSpec::new("ipc command loop", initial, move || {
    Err(DaemonError::Config(
      "ipc command loop is not restartable".to_owned(),
    ))
  })
}

#[cfg(test)]
mod tests {
  use super::{IpcWorker, HEARTBEAT_STALE_SECS};
  use crate::lifecycle::Lifecycle;
  use crate::visor::{ProgressState, RecoveryDisposition, RunState, ShutdownPhase, Supervisable};
  use std::sync::Arc;

  #[tokio::test]
  async fn heartbeat_watchdog_tolerates_stale_progress_with_fresh_activity() {
    let lifecycle = Lifecycle::start();
    let worker = IpcWorker::new(Arc::clone(&lifecycle), "heartbeat", true);
    worker.runtime.mark_started();
    worker.runtime.mark_progress();
    tokio::time::sleep(std::time::Duration::from_millis(1100)).await;
    worker.runtime.mark_activity();

    let state = RunState {
      run_id: worker.runtime.run_id(),
      phase: "heartbeat",
      subphase: None,
      detail: None,
      progress_state: ProgressState::Active,
      last_activity_epoch: worker.runtime.last_activity_epoch(),
      last_progress_epoch: worker.runtime.last_progress_epoch(),
      recovery_attempts: worker.runtime.recovery_attempts(),
      shutdown_phase: ShutdownPhase::Running,
    };
    let now = worker
      .runtime
      .last_progress_epoch()
      .unwrap_or_default()
      .saturating_add(HEARTBEAT_STALE_SECS);

    assert_eq!(
      worker.watchdog_disposition(&state, now),
      RecoveryDisposition::NotNeeded
    );
  }

  #[test]
  fn heartbeat_watchdog_recovers_when_activity_and_progress_are_both_stale() {
    let lifecycle = Lifecycle::start();
    let worker = IpcWorker::new(Arc::clone(&lifecycle), "heartbeat", true);
    worker.runtime.mark_started();
    worker.runtime.mark_progress();

    let state = RunState {
      run_id: worker.runtime.run_id(),
      phase: "heartbeat",
      subphase: None,
      detail: None,
      progress_state: ProgressState::Active,
      last_activity_epoch: worker.runtime.last_activity_epoch(),
      last_progress_epoch: worker.runtime.last_progress_epoch(),
      recovery_attempts: worker.runtime.recovery_attempts(),
      shutdown_phase: ShutdownPhase::Running,
    };
    let now = worker
      .runtime
      .last_progress_epoch()
      .unwrap_or_default()
      .saturating_add(HEARTBEAT_STALE_SECS);

    assert_eq!(
      worker.watchdog_disposition(&state, now),
      RecoveryDisposition::Eligible
    );
  }
}
