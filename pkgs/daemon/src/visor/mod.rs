//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared supervision helpers and lifecycle contracts for long-lived daemon
//! tasks.

pub(crate) mod core;
pub(crate) mod ipc;
pub(crate) mod live;
pub(crate) mod rest;
pub(crate) mod runtime;

use std::panic::AssertUnwindSafe;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use futures::FutureExt;
use tracing::warn;

use crate::error::DaemonError;
use crate::lifecycle;
use crate::worker::current_epoch_secs;

const SHUTDOWN_POLL_INTERVAL_MS: u64 = 200;
const SUPERVISED_RESTART_LIMIT: u32 = 3;
const SUPERVISED_RESTART_BACKOFF_SECS: u64 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ShutdownPhase {
  Running,
  Interrupting,
  Forcing,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ProgressState {
  Idle,
  Starting,
  Active,
  Slow,
  Stale,
  Recovering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum RecoveryDisposition {
  NotNeeded,
  Eligible,
  Blocked(&'static str),
  Exhausted,
  Fatal(&'static str),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RunState<RunId>
where
  RunId: Copy + Eq + Send + Sync + 'static,
{
  pub run_id: Option<RunId>,
  pub phase: &'static str,
  pub subphase: Option<&'static str>,
  pub detail: Option<&'static str>,
  pub progress_state: ProgressState,
  pub last_activity_epoch: Option<u64>,
  pub last_progress_epoch: Option<u64>,
  pub recovery_attempts: u32,
  pub shutdown_phase: ShutdownPhase,
}

#[async_trait]
pub(crate) trait Supervisable {
  type RunId: Copy + Eq + Send + Sync + 'static;

  fn run_state(&self) -> RunState<Self::RunId>;

  fn interrupt(&self);

  fn recovery_disposition(&self, run_id: Option<Self::RunId>) -> RecoveryDisposition;

  fn watchdog_disposition(
    &self,
    state: &RunState<Self::RunId>,
    now_epoch: u64,
  ) -> RecoveryDisposition;

  async fn recover(&self, run_id: Option<Self::RunId>) -> Result<(), DaemonError>;
}

pub(crate) trait Fenceable: Supervisable {
  fn fence(&self, run_id: Option<Self::RunId>);
}

#[derive(Debug)]
pub(crate) struct SupervisedHandle<W> {
  current: Arc<std::sync::RwLock<W>>,
}

impl<W> Clone for SupervisedHandle<W> {
  fn clone(&self) -> Self {
    Self {
      current: Arc::clone(&self.current),
    }
  }
}

impl<W> SupervisedHandle<W> {
  pub(crate) fn new(worker: W) -> Self {
    Self {
      current: Arc::new(std::sync::RwLock::new(worker)),
    }
  }

  pub(crate) fn replace(&self, worker: W) {
    if let Ok(mut current) = self.current.write() {
      *current = worker;
    }
  }
}

#[async_trait]
impl<W> Supervisable for SupervisedHandle<W>
where
  W: Fenceable + Clone + Send + Sync + 'static,
  <W as Supervisable>::RunId: std::fmt::Debug,
{
  type RunId = <W as Supervisable>::RunId;

  fn run_state(&self) -> RunState<Self::RunId> {
    self
      .current
      .read()
      .map(|worker| worker.run_state())
      .unwrap_or(RunState {
        run_id: None,
        phase: "supervisor_poisoned",
        subphase: None,
        detail: None,
        progress_state: ProgressState::Stale,
        last_activity_epoch: None,
        last_progress_epoch: None,
        recovery_attempts: 0,
        shutdown_phase: ShutdownPhase::Forcing,
      })
  }

  fn interrupt(&self) {
    if let Ok(worker) = self.current.read() {
      worker.interrupt();
    }
  }

  fn recovery_disposition(&self, run_id: Option<Self::RunId>) -> RecoveryDisposition {
    self
      .current
      .read()
      .map(|worker| worker.recovery_disposition(run_id))
      .unwrap_or(RecoveryDisposition::Fatal("supervisor state poisoned"))
  }

  fn watchdog_disposition(
    &self,
    state: &RunState<Self::RunId>,
    now_epoch: u64,
  ) -> RecoveryDisposition {
    self
      .current
      .read()
      .map(|worker| worker.watchdog_disposition(state, now_epoch))
      .unwrap_or(RecoveryDisposition::Fatal("supervisor state poisoned"))
  }

  async fn recover(&self, run_id: Option<Self::RunId>) -> Result<(), DaemonError> {
    let worker = self
      .current
      .read()
      .map(|worker| worker.clone())
      .map_err(|_| DaemonError::Index("supervisor state poisoned during recovery".to_owned()))?;
    worker.recover(run_id).await
  }
}

impl<W> Fenceable for SupervisedHandle<W>
where
  W: Fenceable + Clone + Send + Sync + 'static,
  <W as Supervisable>::RunId: std::fmt::Debug,
{
  fn fence(&self, run_id: Option<Self::RunId>) {
    if let Ok(worker) = self.current.read() {
      worker.fence(run_id);
    }
  }
}

#[allow(dead_code)]
/// Lifecycle control shared by all long-lived supervisors.
pub(crate) trait InterruptGate {
  /// Request graceful interruption.
  fn interrupt(&self);

  /// Return `true` while work may continue cooperatively.
  fn interrupt_point(&self) -> bool;
}

#[allow(dead_code)]
/// Common lifecycle surface for daemon supervisors.
#[async_trait]
pub(crate) trait Visor: InterruptGate {
  /// Start owned tasks and runtime state.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` when startup cannot complete atomically.
  async fn start(&mut self) -> Result<(), DaemonError>;

  /// Stop owned tasks and tear down state.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` when shutdown cannot complete cleanly.
  async fn stop(
    &mut self,
    sigterm: &mut tokio::signal::unix::Signal,
    graceful_deadline: Option<tokio::time::Instant>,
  ) -> Result<(), DaemonError>;
}

/// Await one daemon-owned task during shutdown using only its supervision
/// contract for observability and fencing decisions.
pub(crate) async fn await_supervisable_task<S>(
  visor: &str,
  subtask: &str,
  handle: tokio::task::JoinHandle<()>,
  supervised: &S,
  lifecycle: &lifecycle::Lifecycle,
  sigterm: &mut tokio::signal::unix::Signal,
  graceful_deadline: Option<tokio::time::Instant>,
) where
  S: Fenceable,
  <S as Supervisable>::RunId: std::fmt::Debug,
{
  supervised.interrupt();
  while !handle.is_finished() {
    let state = supervised.run_state();
    if lifecycle.terminated() {
      warn!(
        visor,
        subtask,
        run_id = ?state.run_id,
        phase = state.phase,
        subphase = ?state.subphase,
        detail = ?state.detail,
        progress = ?state.progress_state,
        "forcing shutdown of supervised subtask after termination"
      );
      supervised.fence(state.run_id);
      handle.abort();
      break;
    }
    if graceful_deadline.is_some_and(|deadline| tokio::time::Instant::now() >= deadline) {
      warn!(
        visor,
        subtask,
        run_id = ?state.run_id,
        phase = state.phase,
        subphase = ?state.subphase,
        detail = ?state.detail,
        progress = ?state.progress_state,
        "graceful shutdown timed out, fencing supervised subtask before force"
      );
      supervised.fence(state.run_id);
      lifecycle.stop();
      handle.abort();
      break;
    }
    tokio::select! {
      _ = tokio::time::sleep(Duration::from_millis(SHUTDOWN_POLL_INTERVAL_MS)) => {}
      _ = tokio::signal::ctrl_c() => {
        if lifecycle.terminated() {
          warn!(visor, subtask, run_id = ?state.run_id, "received additional SIGINT, fencing and aborting supervised subtask");
        } else {
          warn!(visor, subtask, run_id = ?state.run_id, "received second SIGINT, forcing shutdown of supervised subtask");
          lifecycle.stop();
        }
        supervised.fence(state.run_id);
        handle.abort();
        break;
      }
      _ = sigterm.recv() => {
        warn!(visor, subtask, run_id = ?state.run_id, "received SIGTERM, fencing and forcing supervised subtask");
        supervised.fence(state.run_id);
        lifecycle.stop();
        handle.abort();
        break;
      }
    }
  }
  if let Err(error) = handle.await {
    warn!(visor, subtask, "shutdown join failed: {error}");
  }
}

pub(crate) fn spawn_supervised_task<W, B>(
  visor: &'static str,
  name: &'static str,
  lifecycle: Arc<lifecycle::Lifecycle>,
  handle: SupervisedHandle<W>,
  initial: crate::worker::WorkerTask<W>,
  build: B,
) -> tokio::task::JoinHandle<()>
where
  W: Fenceable + Clone + Send + Sync + 'static,
  <W as Supervisable>::RunId: std::fmt::Debug,
  B: Fn() -> Result<crate::worker::WorkerTask<W>, DaemonError> + Send + Sync + 'static,
{
  let build = Arc::new(build);
  tokio::spawn(async move {
    let mut pending = Some(initial);
    let mut restart_attempts = 0u32;
    loop {
      let crate::worker::WorkerTask { worker, task } = match pending.take() {
        Some(task) => task,
        None => match build() {
          Ok(task) => task,
          Err(error) => {
            tracing::error!(
              visor,
              task = name,
              "failed to rebuild supervised task: {error}"
            );
            lifecycle.stop();
            return;
          }
        },
      };
      handle.replace(worker);
      let child = tokio::spawn(AssertUnwindSafe(task).catch_unwind());
      tokio::pin!(child);

      enum SupervisedOutcome<RunId>
      where
        RunId: Copy + Eq + Send + Sync + 'static,
      {
        Completed(Result<Result<(), DaemonError>, Box<dyn std::any::Any + Send>>),
        JoinFailed(String),
        WatchdogRecoverable {
          state: RunState<RunId>,
        },
        WatchdogTerminal {
          state: RunState<RunId>,
          disposition: RecoveryDisposition,
        },
      }

      let outcome: SupervisedOutcome<<W as Supervisable>::RunId> = loop {
        tokio::select! {
          join = &mut child => {
            let joined = match join {
              Ok(result) => result,
              Err(error) => break SupervisedOutcome::JoinFailed(error.to_string()),
            };
            break SupervisedOutcome::Completed(joined);
          }
          _ = tokio::time::sleep(Duration::from_millis(SHUTDOWN_POLL_INTERVAL_MS)) => {
            if !lifecycle.running() {
              continue;
            }
            let state = handle.run_state();
            let disposition = handle.watchdog_disposition(&state, current_epoch_secs());
            match disposition {
              RecoveryDisposition::Eligible => {
                break SupervisedOutcome::WatchdogRecoverable { state };
              }
              RecoveryDisposition::Exhausted | RecoveryDisposition::Fatal(_) => {
                break SupervisedOutcome::WatchdogTerminal { state, disposition };
              }
              RecoveryDisposition::NotNeeded | RecoveryDisposition::Blocked(_) => {}
            }
          }
        }
      };

      match outcome {
        SupervisedOutcome::WatchdogRecoverable { state } => {
          restart_attempts += 1;
          tracing::warn!(
            visor,
            task = name,
            run_id = ?state.run_id,
            phase = state.phase,
            subphase = ?state.subphase,
            detail = ?state.detail,
            progress = ?state.progress_state,
            attempts = restart_attempts,
            "supervised task became stale; fencing and attempting recovery"
          );
          handle.fence(state.run_id);
          child.as_mut().abort();
          let _ = child.await;
          if restart_attempts > SUPERVISED_RESTART_LIMIT {
            tracing::error!(
              visor,
              task = name,
              attempts = restart_attempts,
              "supervised task exhausted restart budget after stale watchdog recovery"
            );
            lifecycle.stop();
            return;
          }
          if let Err(error) = handle.recover(state.run_id).await {
            tracing::error!(
              visor,
              task = name,
              "supervised task recovery failed: {error}"
            );
            lifecycle.stop();
            return;
          }
          lifecycle
            .sleep(Duration::from_secs(SUPERVISED_RESTART_BACKOFF_SECS))
            .await;
          if !lifecycle.running() {
            return;
          }
          continue;
        }
        SupervisedOutcome::WatchdogTerminal { state, disposition } => {
          tracing::error!(
            visor,
            task = name,
            run_id = ?state.run_id,
            phase = state.phase,
            subphase = ?state.subphase,
            detail = ?state.detail,
            progress = ?state.progress_state,
            disposition = ?disposition,
            "supervised task reached terminal watchdog disposition"
          );
          handle.fence(state.run_id);
          child.as_mut().abort();
          let _ = child.await;
          lifecycle.stop();
          return;
        }
        SupervisedOutcome::JoinFailed(error) => {
          tracing::error!(visor, task = name, "supervised task join failed: {error}");
          lifecycle.stop();
          return;
        }
        SupervisedOutcome::Completed(result) => match result {
          Ok(Ok(())) => {
            if !lifecycle.running() {
              return;
            }
            let state = handle.run_state();
            match handle.recovery_disposition(state.run_id) {
              RecoveryDisposition::Eligible if restart_attempts < SUPERVISED_RESTART_LIMIT => {
                restart_attempts += 1;
                tracing::warn!(
                  visor,
                  task = name,
                  run_id = ?state.run_id,
                  phase = state.phase,
                  subphase = ?state.subphase,
                  detail = ?state.detail,
                  attempts = restart_attempts,
                  "supervised task exited unexpectedly; attempting recovery"
                );
                if let Err(error) = handle.recover(state.run_id).await {
                  tracing::error!(
                    visor,
                    task = name,
                    "supervised task recovery failed: {error}"
                  );
                  lifecycle.stop();
                  return;
                }
                lifecycle
                  .sleep(Duration::from_secs(SUPERVISED_RESTART_BACKOFF_SECS))
                  .await;
                if !lifecycle.running() {
                  return;
                }
                continue;
              }
              RecoveryDisposition::Eligible => {
                tracing::error!(
                  visor,
                  task = name,
                  attempts = restart_attempts,
                  "supervised task exhausted restart budget"
                );
                lifecycle.stop();
                return;
              }
              RecoveryDisposition::NotNeeded => {
                tracing::error!(visor, task = name, "supervised task exited unexpectedly");
                lifecycle.stop();
                return;
              }
              RecoveryDisposition::Blocked(reason) => {
                tracing::error!(
                  visor,
                  task = name,
                  reason,
                  "supervised task exited without recoverable disposition"
                );
                lifecycle.stop();
                return;
              }
              RecoveryDisposition::Exhausted => {
                tracing::error!(visor, task = name, "supervised task recovery exhausted");
                lifecycle.stop();
                return;
              }
              RecoveryDisposition::Fatal(reason) => {
                tracing::error!(
                  visor,
                  task = name,
                  reason,
                  "supervised task reported fatal recovery disposition"
                );
                lifecycle.stop();
                return;
              }
            }
          }
          Ok(Err(error)) => {
            if !lifecycle.running() {
              return;
            }
            let state = handle.run_state();
            match handle.recovery_disposition(state.run_id) {
              RecoveryDisposition::Eligible if restart_attempts < SUPERVISED_RESTART_LIMIT => {
                restart_attempts += 1;
                tracing::warn!(
                  visor,
                  task = name,
                  run_id = ?state.run_id,
                  phase = state.phase,
                  subphase = ?state.subphase,
                  detail = ?state.detail,
                  attempts = restart_attempts,
                  error_detail = %error,
                  "supervised task failed; attempting recovery"
                );
                if let Err(recover_error) = handle.recover(state.run_id).await {
                  tracing::error!(
                    visor,
                    task = name,
                    "supervised task recovery failed: {recover_error}"
                  );
                  lifecycle.stop();
                  return;
                }
                lifecycle
                  .sleep(Duration::from_secs(SUPERVISED_RESTART_BACKOFF_SECS))
                  .await;
                if !lifecycle.running() {
                  return;
                }
                continue;
              }
              RecoveryDisposition::Eligible => {
                tracing::error!(
                  visor,
                  task = name,
                  attempts = restart_attempts,
                  detail = %error,
                  "supervised task exhausted restart budget after failure"
                );
                lifecycle.stop();
                return;
              }
              RecoveryDisposition::NotNeeded => {
                tracing::error!(visor, task = name, "supervised task failed: {error}");
                lifecycle.stop();
                return;
              }
              RecoveryDisposition::Blocked(reason) => {
                tracing::error!(
                  visor,
                  task = name,
                  reason,
                  detail = %error,
                  "supervised task failed without recoverable disposition"
                );
                lifecycle.stop();
                return;
              }
              RecoveryDisposition::Exhausted => {
                tracing::error!(
                  visor,
                  task = name,
                  detail = %error,
                  "supervised task recovery exhausted after failure"
                );
                lifecycle.stop();
                return;
              }
              RecoveryDisposition::Fatal(reason) => {
                tracing::error!(
                  visor,
                  task = name,
                  reason,
                  detail = %error,
                  "supervised task reported fatal recovery disposition after failure"
                );
                lifecycle.stop();
                return;
              }
            }
          }
          Err(panic) => {
            tracing::error!(
              visor,
              task = name,
              panic = %decode_panic_payload(&panic),
              "supervised task panicked"
            );
            lifecycle.stop();
            return;
          }
        },
      }
    }
  })
}

pub(crate) fn decode_panic_payload(panic: &Box<dyn std::any::Any + Send>) -> String {
  if let Some(msg) = panic.downcast_ref::<&str>() {
    (*msg).to_owned()
  } else if let Some(msg) = panic.downcast_ref::<String>() {
    msg.clone()
  } else {
    "unknown panic payload".to_owned()
  }
}

#[cfg(test)]
mod tests {
  use std::sync::atomic::{AtomicBool, Ordering};
  use std::sync::Arc;

  use async_trait::async_trait;

  use super::{
    await_supervisable_task, spawn_supervised_task, Fenceable, ProgressState, RecoveryDisposition,
    RunState, ShutdownPhase, Supervisable, SupervisedHandle,
  };
  use crate::error::DaemonError;

  #[derive(Clone, Debug)]
  struct FakeWorker {
    fenced: Arc<AtomicBool>,
  }

  #[async_trait]
  impl Supervisable for FakeWorker {
    type RunId = u64;

    fn run_state(&self) -> RunState<Self::RunId> {
      RunState {
        run_id: Some(7),
        phase: "phase",
        subphase: Some("subphase"),
        detail: Some("detail"),
        progress_state: ProgressState::Active,
        last_activity_epoch: Some(1),
        last_progress_epoch: Some(1),
        recovery_attempts: 0,
        shutdown_phase: ShutdownPhase::Running,
      }
    }

    fn interrupt(&self) {}

    fn recovery_disposition(&self, _run_id: Option<Self::RunId>) -> RecoveryDisposition {
      RecoveryDisposition::NotNeeded
    }

    fn watchdog_disposition(
      &self,
      _state: &RunState<Self::RunId>,
      _now_epoch: u64,
    ) -> RecoveryDisposition {
      RecoveryDisposition::NotNeeded
    }

    async fn recover(&self, _run_id: Option<Self::RunId>) -> Result<(), DaemonError> {
      Ok(())
    }
  }

  impl Fenceable for FakeWorker {
    fn fence(&self, _run_id: Option<Self::RunId>) {
      self.fenced.store(true, Ordering::SeqCst);
    }
  }

  #[tokio::test]
  async fn await_supervisable_task_fences_before_forced_abort() {
    let lifecycle = crate::lifecycle::Lifecycle::start();
    let worker = FakeWorker {
      fenced: Arc::new(AtomicBool::new(false)),
    };
    let task = tokio::spawn(async {
      tokio::time::sleep(std::time::Duration::from_secs(60)).await;
    });
    let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
      .expect("sigterm listener");

    await_supervisable_task(
      "test",
      "fake",
      task,
      &worker,
      &lifecycle,
      &mut sigterm,
      Some(tokio::time::Instant::now()),
    )
    .await;

    assert!(worker.fenced.load(Ordering::SeqCst));
    assert!(lifecycle.terminated());
  }

  #[tokio::test]
  async fn spawn_supervised_task_restarts_eligible_worker() {
    #[derive(Clone, Debug)]
    struct RestartableWorker {
      attempts: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl Supervisable for RestartableWorker {
      type RunId = u64;

      fn run_state(&self) -> RunState<Self::RunId> {
        RunState {
          run_id: Some(1),
          phase: "restartable",
          subphase: None,
          detail: None,
          progress_state: ProgressState::Active,
          last_activity_epoch: Some(1),
          last_progress_epoch: Some(1),
          recovery_attempts: self.attempts.load(Ordering::SeqCst) as u32,
          shutdown_phase: ShutdownPhase::Running,
        }
      }

      fn interrupt(&self) {}

      fn recovery_disposition(&self, _run_id: Option<Self::RunId>) -> RecoveryDisposition {
        RecoveryDisposition::Eligible
      }

      fn watchdog_disposition(
        &self,
        _state: &RunState<Self::RunId>,
        _now_epoch: u64,
      ) -> RecoveryDisposition {
        RecoveryDisposition::NotNeeded
      }

      async fn recover(&self, _run_id: Option<Self::RunId>) -> Result<(), DaemonError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Ok(())
      }
    }

    impl Fenceable for RestartableWorker {
      fn fence(&self, _run_id: Option<Self::RunId>) {}
    }

    let lifecycle = crate::lifecycle::Lifecycle::start();
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let builds = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let restart_lifecycle = Arc::clone(&lifecycle);
    let handle = SupervisedHandle::new(RestartableWorker {
      attempts: Arc::clone(&attempts),
    });
    let task = spawn_supervised_task(
      "test",
      "restartable",
      Arc::clone(&lifecycle),
      handle,
      crate::worker::WorkerTask::new(
        RestartableWorker {
          attempts: Arc::clone(&attempts),
        },
        async { Err(DaemonError::Config("boom".to_owned())) },
      ),
      {
        let attempts = Arc::clone(&attempts);
        let builds = Arc::clone(&builds);
        let lifecycle = Arc::clone(&restart_lifecycle);
        move || {
          builds.fetch_add(1, Ordering::SeqCst);
          Ok(crate::worker::WorkerTask::new(
            RestartableWorker {
              attempts: Arc::clone(&attempts),
            },
            {
              let lifecycle = Arc::clone(&lifecycle);
              async move {
                while lifecycle.running() {
                  tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Ok(())
              }
            },
          ))
        }
      },
    );

    tokio::time::sleep(std::time::Duration::from_millis(1200)).await;
    lifecycle.interrupt();
    let _ = task.await;

    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
  }

  #[tokio::test]
  async fn spawn_supervised_task_watchdog_restarts_stale_worker() {
    #[derive(Clone, Debug)]
    struct WatchdogWorker {
      attempts: Arc<std::sync::atomic::AtomicUsize>,
    }

    #[async_trait]
    impl Supervisable for WatchdogWorker {
      type RunId = u64;

      fn run_state(&self) -> RunState<Self::RunId> {
        RunState {
          run_id: Some(9),
          phase: "watchdog",
          subphase: None,
          detail: None,
          progress_state: ProgressState::Active,
          last_activity_epoch: Some(1),
          last_progress_epoch: Some(1),
          recovery_attempts: self.attempts.load(Ordering::SeqCst) as u32,
          shutdown_phase: ShutdownPhase::Running,
        }
      }

      fn interrupt(&self) {}

      fn recovery_disposition(&self, _run_id: Option<Self::RunId>) -> RecoveryDisposition {
        RecoveryDisposition::NotNeeded
      }

      fn watchdog_disposition(
        &self,
        _state: &RunState<Self::RunId>,
        _now_epoch: u64,
      ) -> RecoveryDisposition {
        RecoveryDisposition::Eligible
      }

      async fn recover(&self, _run_id: Option<Self::RunId>) -> Result<(), DaemonError> {
        self.attempts.fetch_add(1, Ordering::SeqCst);
        Ok(())
      }
    }

    impl Fenceable for WatchdogWorker {
      fn fence(&self, _run_id: Option<Self::RunId>) {}
    }

    let lifecycle = crate::lifecycle::Lifecycle::start();
    let attempts = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let builds = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let restart_lifecycle = Arc::clone(&lifecycle);
    let handle = SupervisedHandle::new(WatchdogWorker {
      attempts: Arc::clone(&attempts),
    });
    let task = spawn_supervised_task(
      "test",
      "watchdog",
      Arc::clone(&lifecycle),
      handle,
      crate::worker::WorkerTask::new(
        WatchdogWorker {
          attempts: Arc::clone(&attempts),
        },
        async {
          tokio::time::sleep(std::time::Duration::from_secs(60)).await;
          Ok(())
        },
      ),
      {
        let attempts = Arc::clone(&attempts);
        let builds = Arc::clone(&builds);
        let lifecycle = Arc::clone(&restart_lifecycle);
        move || {
          builds.fetch_add(1, Ordering::SeqCst);
          Ok(crate::worker::WorkerTask::new(
            WatchdogWorker {
              attempts: Arc::clone(&attempts),
            },
            {
              let lifecycle = Arc::clone(&lifecycle);
              async move {
                while lifecycle.running() {
                  tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                }
                Ok(())
              }
            },
          ))
        }
      },
    );

    tokio::time::sleep(std::time::Duration::from_millis(1400)).await;
    lifecycle.interrupt();
    let _ = task.await;

    assert_eq!(attempts.load(Ordering::SeqCst), 1);
    assert_eq!(builds.load(Ordering::SeqCst), 1);
  }

  #[tokio::test]
  async fn spawn_supervised_task_does_not_restart_blocked_watchdog_worker() {
    #[derive(Clone, Debug)]
    struct BlockedWorker;

    #[async_trait]
    impl Supervisable for BlockedWorker {
      type RunId = u64;

      fn run_state(&self) -> RunState<Self::RunId> {
        RunState {
          run_id: Some(3),
          phase: "blocked",
          subphase: None,
          detail: None,
          progress_state: ProgressState::Active,
          last_activity_epoch: Some(1),
          last_progress_epoch: None,
          recovery_attempts: 0,
          shutdown_phase: ShutdownPhase::Running,
        }
      }

      fn interrupt(&self) {}

      fn recovery_disposition(&self, _run_id: Option<Self::RunId>) -> RecoveryDisposition {
        RecoveryDisposition::Blocked("worker is not restartable")
      }

      fn watchdog_disposition(
        &self,
        _state: &RunState<Self::RunId>,
        _now_epoch: u64,
      ) -> RecoveryDisposition {
        RecoveryDisposition::Blocked("worker does not expose watchdog activity")
      }

      async fn recover(&self, _run_id: Option<Self::RunId>) -> Result<(), DaemonError> {
        Err(DaemonError::Config(
          "blocked worker should never be recovered".to_owned(),
        ))
      }
    }

    impl Fenceable for BlockedWorker {
      fn fence(&self, _run_id: Option<Self::RunId>) {}
    }

    let lifecycle = crate::lifecycle::Lifecycle::start();
    let builds = Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let handle = SupervisedHandle::new(BlockedWorker);
    let task = spawn_supervised_task(
      "test",
      "blocked",
      Arc::clone(&lifecycle),
      handle,
      crate::worker::WorkerTask::new(BlockedWorker, async {
        tokio::time::sleep(std::time::Duration::from_millis(250)).await;
        Ok(())
      }),
      {
        let builds = Arc::clone(&builds);
        move || {
          builds.fetch_add(1, Ordering::SeqCst);
          Ok(crate::worker::WorkerTask::new(BlockedWorker, async {
            Ok(())
          }))
        }
      },
    );

    let _ = task.await;

    assert!(lifecycle.terminated());
    assert_eq!(builds.load(Ordering::SeqCst), 0);
  }
}
