//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Generic task-side worker implementations that expose the visor supervision
//! contract without embedding supervision policy.

use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;
use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::DaemonError;
use crate::lifecycle::Lifecycle;
use crate::visor::RecoveryDisposition;

pub(crate) type BoxDaemonFuture = Pin<Box<dyn Future<Output = Result<(), DaemonError>> + Send>>;
pub(crate) type WorkerTaskBuilder<W> =
  Arc<dyn Fn() -> Result<WorkerTask<W>, DaemonError> + Send + Sync + 'static>;

#[derive(Debug, Default)]
pub(crate) struct WorkerRuntimeState {
  started: AtomicBool,
  finished: AtomicBool,
  recovering: AtomicBool,
  run_id: AtomicU64,
  recovery_attempts: AtomicU32,
  last_activity_epoch: AtomicU64,
  last_progress_epoch: AtomicU64,
}

impl WorkerRuntimeState {
  #[must_use]
  pub fn shared() -> Arc<Self> {
    Arc::new(Self::default())
  }

  pub fn mark_started(&self) {
    self.started.store(true, Ordering::Relaxed);
    self.finished.store(false, Ordering::Relaxed);
    self.recovering.store(false, Ordering::Relaxed);
    self.recovery_attempts.store(0, Ordering::Relaxed);
    let _ = self.run_id.fetch_add(1, Ordering::Relaxed);
    self.mark_activity();
  }

  pub fn mark_activity(&self) {
    self
      .last_activity_epoch
      .store(current_epoch_secs(), Ordering::Relaxed);
  }

  pub fn mark_progress(&self) {
    let now = current_epoch_secs();
    self.last_activity_epoch.store(now, Ordering::Relaxed);
    self.last_progress_epoch.store(now, Ordering::Relaxed);
  }

  pub fn mark_finished(&self) {
    self.finished.store(true, Ordering::Relaxed);
    self.recovering.store(false, Ordering::Relaxed);
    self.mark_activity();
  }

  #[must_use]
  pub fn started(&self) -> bool {
    self.started.load(Ordering::Relaxed)
  }

  #[must_use]
  pub fn finished(&self) -> bool {
    self.finished.load(Ordering::Relaxed)
  }

  #[must_use]
  pub fn recovering(&self) -> bool {
    self.recovering.load(Ordering::Relaxed)
  }

  #[must_use]
  pub fn run_id(&self) -> Option<u64> {
    if !self.started() || self.finished() {
      return None;
    }
    let run_id = self.run_id.load(Ordering::Relaxed);
    (run_id != 0).then_some(run_id)
  }

  #[must_use]
  pub fn recovery_attempts(&self) -> u32 {
    self.recovery_attempts.load(Ordering::Relaxed)
  }

  #[must_use]
  pub fn last_activity_epoch(&self) -> Option<u64> {
    let epoch = self.last_activity_epoch.load(Ordering::Relaxed);
    (epoch != 0).then_some(epoch)
  }

  #[must_use]
  pub fn last_progress_epoch(&self) -> Option<u64> {
    let epoch = self.last_progress_epoch.load(Ordering::Relaxed);
    (epoch != 0).then_some(epoch)
  }

  #[must_use]
  pub fn recovery_disposition(&self, run_id: Option<u64>) -> RecoveryDisposition {
    if !self.started() || self.finished() {
      return RecoveryDisposition::NotNeeded;
    }
    if run_id.is_some() && run_id != self.run_id() {
      return RecoveryDisposition::Blocked("run mismatch");
    }
    if self.recovering() {
      return RecoveryDisposition::Blocked("recovery in progress");
    }
    RecoveryDisposition::Eligible
  }

  #[must_use]
  pub fn progress_started(&self) -> bool {
    self.last_progress_epoch().is_some()
  }

  pub fn begin_recovery(&self, run_id: Option<u64>) -> RecoveryDisposition {
    match self.recovery_disposition(run_id) {
      RecoveryDisposition::Eligible => {
        self.recovering.store(true, Ordering::Relaxed);
        self.recovery_attempts.fetch_add(1, Ordering::Relaxed);
        self.mark_activity();
        RecoveryDisposition::Eligible
      }
      other => other,
    }
  }
}

pub(crate) struct WorkerTask<W> {
  pub worker: W,
  pub task: BoxDaemonFuture,
}

impl<W> WorkerTask<W> {
  pub fn new<F>(worker: W, task: F) -> Self
  where
    F: Future<Output = Result<(), DaemonError>> + Send + 'static,
  {
    Self {
      worker,
      task: Box::pin(task),
    }
  }
}

impl<W> std::fmt::Debug for WorkerTask<W>
where
  W: std::fmt::Debug,
{
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("WorkerTask")
      .field("worker", &self.worker)
      .field("task", &"<BoxDaemonFuture>")
      .finish()
  }
}

pub(crate) struct SupervisedTaskSpec<W> {
  pub name: &'static str,
  pub initial: WorkerTask<W>,
  pub rebuild: WorkerTaskBuilder<W>,
}

impl<W> SupervisedTaskSpec<W> {
  #[must_use]
  pub fn new<B>(name: &'static str, initial: WorkerTask<W>, rebuild: B) -> Self
  where
    B: Fn() -> Result<WorkerTask<W>, DaemonError> + Send + Sync + 'static,
  {
    Self {
      name,
      initial,
      rebuild: Arc::new(rebuild),
    }
  }
}

impl<W> std::fmt::Debug for SupervisedTaskSpec<W>
where
  W: std::fmt::Debug,
{
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("SupervisedTaskSpec")
      .field("name", &self.name)
      .field("initial", &self.initial)
      .field("rebuild", &"<WorkerTaskBuilder>")
      .finish()
  }
}

#[must_use]
pub(crate) fn current_epoch_secs() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

#[must_use]
pub(crate) fn spawn_activity_ticker(
  lifecycle: Arc<Lifecycle>,
  runtime: Arc<WorkerRuntimeState>,
  interval: Duration,
) -> tokio::task::JoinHandle<()> {
  tokio::spawn(async move {
    while lifecycle.running() && !runtime.finished() {
      lifecycle.sleep(interval).await;
      if lifecycle.running() && runtime.started() && !runtime.finished() {
        runtime.mark_activity();
      }
    }
  })
}

#[must_use]
pub(crate) fn runtime_watchdog_disposition(
  runtime: &WorkerRuntimeState,
  run_id: Option<u64>,
  now_epoch: u64,
  starting_stale_secs: u64,
  activity_stale_secs: u64,
  progress_stale_secs: Option<u64>,
) -> RecoveryDisposition {
  let Some(last_activity_epoch) = runtime.last_activity_epoch() else {
    return RecoveryDisposition::Blocked("worker has not emitted watchdog activity");
  };
  if !runtime.progress_started() {
    return if now_epoch.saturating_sub(last_activity_epoch) >= starting_stale_secs {
      runtime.recovery_disposition(run_id)
    } else {
      RecoveryDisposition::NotNeeded
    };
  }

  if progress_stale_secs.is_some_and(|stale_secs| {
    runtime
      .last_progress_epoch()
      .map(|epoch| now_epoch.saturating_sub(epoch) >= stale_secs)
      .unwrap_or(false)
  }) {
    return runtime.recovery_disposition(run_id);
  }

  if now_epoch.saturating_sub(last_activity_epoch) >= activity_stale_secs {
    runtime.recovery_disposition(run_id)
  } else {
    RecoveryDisposition::NotNeeded
  }
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;
  use std::time::Duration;

  use super::{runtime_watchdog_disposition, spawn_activity_ticker, WorkerRuntimeState};
  use crate::lifecycle::Lifecycle;
  use crate::visor::RecoveryDisposition;

  #[test]
  fn worker_runtime_state_tracks_start_progress_and_finish() {
    let state = WorkerRuntimeState::default();

    assert!(!state.started());
    assert!(!state.finished());
    assert!(!state.recovering());
    assert_eq!(state.run_id(), None);
    assert_eq!(state.recovery_attempts(), 0);
    assert_eq!(state.last_activity_epoch(), None);
    assert_eq!(state.last_progress_epoch(), None);

    state.mark_started();
    assert!(state.started());
    assert!(!state.finished());
    assert!(!state.recovering());
    assert_eq!(state.run_id(), Some(1));
    assert_eq!(state.recovery_attempts(), 0);
    assert!(state.last_activity_epoch().is_some());
    assert_eq!(state.last_progress_epoch(), None);

    state.mark_progress();
    assert!(state.last_activity_epoch().is_some());
    assert!(state.last_progress_epoch().is_some());

    state.mark_finished();
    assert!(state.finished());
    assert!(!state.recovering());
    assert_eq!(state.run_id(), None);
    assert!(state.last_activity_epoch().is_some());
  }

  #[test]
  fn worker_runtime_state_tracks_recovery_eligibility() {
    let state = WorkerRuntimeState::default();

    assert_eq!(
      state.recovery_disposition(None),
      RecoveryDisposition::NotNeeded
    );

    state.mark_started();
    assert_eq!(
      state.recovery_disposition(Some(1)),
      RecoveryDisposition::Eligible
    );
    assert_eq!(state.begin_recovery(Some(1)), RecoveryDisposition::Eligible);
    assert!(state.recovering());
    assert_eq!(state.recovery_attempts(), 1);
    assert_eq!(
      state.recovery_disposition(Some(1)),
      RecoveryDisposition::Blocked("recovery in progress")
    );
    assert_eq!(
      state.recovery_disposition(Some(2)),
      RecoveryDisposition::Blocked("run mismatch")
    );

    state.mark_finished();
    assert_eq!(
      state.recovery_disposition(None),
      RecoveryDisposition::NotNeeded
    );
  }

  #[test]
  fn runtime_watchdog_disposition_marks_stale_started_worker_eligible() {
    let state = WorkerRuntimeState::default();
    state.mark_started();
    let run_id = state.run_id();
    let stale_now = state
      .last_activity_epoch()
      .expect("started workers record activity")
      .saturating_add(20);

    assert_eq!(
      runtime_watchdog_disposition(&state, run_id, stale_now, 15, 15, None),
      RecoveryDisposition::Eligible
    );
  }

  #[test]
  fn runtime_watchdog_disposition_requires_more_time_for_recent_worker() {
    let state = WorkerRuntimeState::default();
    state.mark_started();
    let run_id = state.run_id();
    let recent_now = state
      .last_activity_epoch()
      .expect("started workers record activity")
      .saturating_add(5);

    assert_eq!(
      runtime_watchdog_disposition(&state, run_id, recent_now, 15, 15, None),
      RecoveryDisposition::NotNeeded
    );
  }

  #[test]
  fn runtime_watchdog_disposition_marks_stale_progress_worker_eligible() {
    let state = WorkerRuntimeState::default();
    state.mark_started();
    state.mark_progress();
    let run_id = state.run_id();
    let stale_now = state
      .last_progress_epoch()
      .expect("progress records one watchdog timestamp")
      .saturating_add(20);

    assert_eq!(
      runtime_watchdog_disposition(&state, run_id, stale_now, 15, 15, Some(15)),
      RecoveryDisposition::Eligible
    );
  }

  #[test]
  fn runtime_watchdog_disposition_blocks_run_mismatch() {
    let state = WorkerRuntimeState::default();
    state.mark_started();
    let stale_now = state
      .last_activity_epoch()
      .expect("started workers record activity")
      .saturating_add(20);

    assert_eq!(
      runtime_watchdog_disposition(&state, Some(999), stale_now, 15, 15, None),
      RecoveryDisposition::Blocked("run mismatch")
    );
  }

  #[tokio::test]
  async fn spawn_activity_ticker_marks_started_workers_active() {
    let lifecycle = Lifecycle::start();
    let runtime = Arc::new(WorkerRuntimeState::default());
    runtime.mark_started();
    let before = runtime.last_activity_epoch();

    let ticker = spawn_activity_ticker(
      Arc::clone(&lifecycle),
      Arc::clone(&runtime),
      Duration::from_millis(10),
    );
    tokio::time::sleep(Duration::from_millis(25)).await;
    lifecycle.stop();
    ticker.await.expect("activity ticker should exit cleanly");

    assert!(runtime.last_activity_epoch().is_some());
    assert!(runtime.last_activity_epoch() >= before);
  }

  #[tokio::test]
  async fn spawn_activity_ticker_ignores_finished_workers() {
    let lifecycle = Lifecycle::start();
    let runtime = Arc::new(WorkerRuntimeState::default());
    runtime.mark_started();
    runtime.mark_finished();
    let before = runtime.last_activity_epoch();

    let ticker = spawn_activity_ticker(
      Arc::clone(&lifecycle),
      Arc::clone(&runtime),
      Duration::from_millis(10),
    );
    tokio::time::sleep(Duration::from_millis(25)).await;
    lifecycle.stop();
    ticker.await.expect("activity ticker should exit cleanly");

    assert_eq!(runtime.last_activity_epoch(), before);
  }
}
