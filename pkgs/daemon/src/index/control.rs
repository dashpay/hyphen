//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Pause/resume control handles and shared dependencies for indexer loops.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use crate::sql::SqlPool;
use async_trait::async_trait;
use rocksdb::DB;

use crate::error::DaemonError;
use crate::index::batch::BatchSizer;
use crate::index::runner::IndexSchedulerState;
use crate::index::spent::cache::SpentQueryCache;
use crate::ingest::IngestRuntime;
use crate::lifecycle::Lifecycle;
use crate::store::manager::SecondaryStoreSet;
use crate::visor::{
  Fenceable, ProgressState, RecoveryDisposition, RunState, ShutdownPhase, Supervisable,
};
use crate::worker::SupervisedTaskSpec;
use hyphend::wire::msg::IndexSchedulerDiag;

use super::Stage;

/// Per-stage control handle held by the command handler.
#[derive(Clone)]
pub(crate) struct IndexerControl {
  pause_tx: tokio::sync::watch::Sender<bool>,
  idle_rx: tokio::sync::watch::Receiver<bool>,
  idle_timeout: std::time::Duration,
}

impl std::fmt::Debug for IndexerControl {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("IndexerControl").finish()
  }
}

impl IndexerControl {
  /// Ask the indexer loop to pause at its next safe point.
  pub fn pause(&self) {
    self.pause_tx.send_replace(true);
  }

  /// Ask the indexer loop to resume processing.
  pub fn resume(&self) {
    self.pause_tx.send_replace(false);
  }

  /// Wait until the indexer loop signals it has entered its idle wait.
  pub async fn wait_idle(&mut self) -> bool {
    self.wait_idle_for(self.idle_timeout).await
  }

  /// Wait until the indexer loop signals it has entered its idle wait.
  pub async fn wait_idle_for(&mut self, timeout: std::time::Duration) -> bool {
    self.idle_rx.borrow_and_update();
    let sleep = tokio::time::sleep(timeout);
    tokio::pin!(sleep);
    loop {
      tokio::select! {
        r = self.idle_rx.changed() => {
          if r.is_err() { return false; }
          if *self.idle_rx.borrow_and_update() { return true; }
        }
        _ = &mut sleep => {
          tracing::warn!(
            timeout_secs = timeout.as_secs(),
            "wait_idle: timed out"
          );
          return false;
        }
      }
    }
  }
}

/// Per-stage handle consumed by an indexer loop.
///
/// The loop side listens for pause requests and reports when it has reached an
/// idle point that is safe for rewind or reindex commands.
#[derive(Clone)]
pub(crate) struct IndexerHandle {
  pub pause_rx: tokio::sync::watch::Receiver<bool>,
  pub idle_tx: tokio::sync::watch::Sender<bool>,
}

impl std::fmt::Debug for IndexerHandle {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("IndexerHandle").finish()
  }
}

/// Create a paired (control, handle) for a single indexer stage.
pub(crate) fn indexer_channel(
  idle_timeout: std::time::Duration,
) -> (IndexerControl, IndexerHandle) {
  let (pause_tx, pause_rx) = tokio::sync::watch::channel(false);
  let (idle_tx, idle_rx) = tokio::sync::watch::channel(false);
  (
    IndexerControl {
      pause_tx,
      idle_rx,
      idle_timeout,
    },
    IndexerHandle { pause_rx, idle_tx },
  )
}

/// Shared runtime dependencies for the forward block indexer loop.
///
/// This groups long-lived services that would otherwise make the runner
/// signature noisy and harder to move between modules.
#[derive(Clone)]
pub(crate) struct IndexerLoopCtx {
  pub db: Arc<DB>,
  pub secondary_stores: Option<Arc<SecondaryStoreSet>>,
  pub pool: SqlPool,
  pub block_worker: Arc<BlockIndexerWorker>,
  pub event_tx: tokio::sync::broadcast::Sender<hyphend::wire::msg::DaemonMessage>,
  pub lifecycle: Arc<Lifecycle>,
  pub enabled_stages: Arc<Vec<Stage>>,
  pub chain_state_lock: Arc<tokio::sync::RwLock<()>>,
  pub scheduler_state: Arc<std::sync::Mutex<IndexSchedulerState>>,
  pub spent_cache: Arc<SpentQueryCache>,
  pub history_partition_cache: Arc<std::sync::Mutex<Option<i32>>>,
  pub canonical_partition_cache: Arc<std::sync::Mutex<Option<i32>>>,
  pub tx_batch_sizer: Arc<std::sync::Mutex<BatchSizer>>,
  pub spent_batch_sizer: Arc<std::sync::Mutex<BatchSizer>>,
  pub operator_pause_requested: Arc<AtomicBool>,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub block_time_secs: f64,
}

impl std::fmt::Debug for IndexerLoopCtx {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("IndexerLoopCtx").finish()
  }
}

#[derive(Clone)]
pub(crate) struct BlockIndexerWorker {
  db: Arc<DB>,
  ingest_runtime: Arc<IngestRuntime>,
  lifecycle: Arc<Lifecycle>,
  scheduler_state: Arc<std::sync::Mutex<IndexSchedulerState>>,
  control_plane_pause_active: Arc<AtomicBool>,
}

impl std::fmt::Debug for BlockIndexerWorker {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("BlockIndexerWorker").finish()
  }
}

impl BlockIndexerWorker {
  pub fn new(
    db: Arc<DB>,
    ingest_runtime: Arc<IngestRuntime>,
    lifecycle: Arc<Lifecycle>,
    scheduler_state: Arc<std::sync::Mutex<IndexSchedulerState>>,
    control_plane_pause_active: Arc<AtomicBool>,
  ) -> Self {
    Self {
      db,
      ingest_runtime,
      lifecycle,
      scheduler_state,
      control_plane_pause_active,
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

  pub async fn recover_pg_shutdown(
    &self,
    _error: &DaemonError,
    backoff_secs: u64,
  ) -> Result<(), DaemonError> {
    self.ingest_runtime.set_paused(true);
    if let Ok(mut state) = self.scheduler_state.lock() {
      state.set_idle();
    }

    let db = Arc::clone(&self.db);
    tokio::task::spawn_blocking(move || {
      db.flush_wal(true).map_err(|flush_error| {
        DaemonError::Index(format!(
          "flush RocksDB WAL during postgres backoff: {flush_error}"
        ))
      })?;
      db.flush().map_err(|flush_error| {
        DaemonError::Index(format!(
          "flush RocksDB memtables during postgres backoff: {flush_error}"
        ))
      })?;
      Ok::<(), DaemonError>(())
    })
    .await
    .map_err(|join_error| {
      DaemonError::Index(format!("flush rocksdb for postgres backoff: {join_error}"))
    })??;

    self
      .lifecycle
      .sleep(std::time::Duration::from_secs(backoff_secs))
      .await;
    self.ingest_runtime.set_paused(false);
    Ok(())
  }

  pub fn scheduler_diag(&self) -> IndexSchedulerDiag {
    self
      .scheduler_state
      .lock()
      .map(|state| state.snapshot())
      .unwrap_or_default()
  }

  pub async fn force_control_plane_recover(&self, run_id: Option<u64>) -> Result<(), DaemonError> {
    let disposition = self
      .scheduler_state
      .lock()
      .map(|state| state.active_recovery_disposition(run_id))
      .unwrap_or(RecoveryDisposition::Fatal("scheduler state poisoned"));
    match disposition {
      RecoveryDisposition::NotNeeded => Ok(()),
      RecoveryDisposition::Eligible => {
        self.ingest_runtime.set_paused(true);
        let recorded = self
          .scheduler_state
          .lock()
          .map_err(|_| {
            DaemonError::Index("scheduler state poisoned during control-plane recovery".to_owned())
          })?
          .record_active_recovery_for_run(run_id);
        if !recorded {
          return Err(DaemonError::Index(
            "control-plane recovery requested without recoverable scheduler state".to_owned(),
          ));
        }
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || {
          db.flush_wal(true).map_err(|flush_error| {
            DaemonError::Index(format!(
              "flush RocksDB WAL during control-plane recovery: {flush_error}"
            ))
          })?;
          db.flush().map_err(|flush_error| {
            DaemonError::Index(format!(
              "flush RocksDB memtables during control-plane recovery: {flush_error}"
            ))
          })?;
          Ok::<(), DaemonError>(())
        })
        .await
        .map_err(|join_error| {
          DaemonError::Index(format!(
            "flush rocksdb for control-plane recovery: {join_error}"
          ))
        })??;
        self
          .lifecycle
          .sleep(std::time::Duration::from_secs(1))
          .await;
        Ok(())
      }
      RecoveryDisposition::Blocked(reason) => Err(DaemonError::Index(format!(
        "control-plane recovery blocked: {reason}"
      ))),
      RecoveryDisposition::Exhausted => Err(DaemonError::Index(
        "control-plane recovery exhausted retry budget".to_owned(),
      )),
      RecoveryDisposition::Fatal(reason) => Err(DaemonError::Index(format!(
        "control-plane recovery unavailable: {reason}"
      ))),
    }
  }
}

#[async_trait]
impl Supervisable for BlockIndexerWorker {
  type RunId = u64;

  fn run_state(&self) -> RunState<Self::RunId> {
    let shutdown_phase = self.current_shutdown_phase();
    self
      .scheduler_state
      .lock()
      .map(|state| state.active_run_state(shutdown_phase))
      .unwrap_or_else(|_| RunState {
        run_id: None,
        phase: "-",
        subphase: None,
        detail: None,
        progress_state: ProgressState::Idle,
        last_activity_epoch: None,
        last_progress_epoch: None,
        recovery_attempts: 0,
        shutdown_phase,
      })
  }

  fn interrupt(&self) {
    self.lifecycle.interrupt();
  }

  fn recovery_disposition(&self, run_id: Option<Self::RunId>) -> RecoveryDisposition {
    match self.current_shutdown_phase() {
      ShutdownPhase::Interrupting => RecoveryDisposition::Blocked("interrupt requested"),
      ShutdownPhase::Forcing => RecoveryDisposition::Blocked("shutdown in progress"),
      ShutdownPhase::Running => self
        .scheduler_state
        .lock()
        .map(|state| state.active_recovery_disposition(run_id))
        .unwrap_or(RecoveryDisposition::Fatal("scheduler state poisoned")),
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
      ShutdownPhase::Running => {
        if self.control_plane_pause_active.load(Ordering::Relaxed) {
          return RecoveryDisposition::Blocked("ipc pause/rewind active");
        }
        if state.run_id.is_none() {
          return RecoveryDisposition::NotNeeded;
        }
        self
          .scheduler_state
          .lock()
          .map(|scheduler| scheduler.active_watchdog_disposition(state.run_id, now_epoch))
          .unwrap_or(RecoveryDisposition::Fatal("scheduler state poisoned"))
      }
    }
  }

  async fn recover(&self, run_id: Option<Self::RunId>) -> Result<(), DaemonError> {
    match self.recovery_disposition(run_id) {
      RecoveryDisposition::NotNeeded => Ok(()),
      RecoveryDisposition::Eligible => {
        self.ingest_runtime.set_paused(true);
        let recorded = self
          .scheduler_state
          .lock()
          .map_err(|_| DaemonError::Index("scheduler state poisoned during recovery".to_owned()))?
          .record_active_recovery_for_run(run_id);
        if !recorded {
          return Err(DaemonError::Index(
            "active run recovery requested without recoverable scheduler state".to_owned(),
          ));
        }
        let db = Arc::clone(&self.db);
        tokio::task::spawn_blocking(move || {
          db.flush_wal(true).map_err(|flush_error| {
            DaemonError::Index(format!(
              "flush RocksDB WAL during stale-run recovery: {flush_error}"
            ))
          })?;
          db.flush().map_err(|flush_error| {
            DaemonError::Index(format!(
              "flush RocksDB memtables during stale-run recovery: {flush_error}"
            ))
          })?;
          Ok::<(), DaemonError>(())
        })
        .await
        .map_err(|join_error| {
          DaemonError::Index(format!(
            "flush rocksdb for stale-run recovery: {join_error}"
          ))
        })??;
        self
          .lifecycle
          .sleep(std::time::Duration::from_secs(1))
          .await;
        self.ingest_runtime.set_paused(false);
        Ok(())
      }
      RecoveryDisposition::Blocked(reason) => Err(DaemonError::Index(format!(
        "forward recovery blocked: {reason}"
      ))),
      RecoveryDisposition::Exhausted => Err(DaemonError::Index(
        "forward recovery exhausted retry budget".to_owned(),
      )),
      RecoveryDisposition::Fatal(reason) => Err(DaemonError::Index(format!(
        "forward recovery unavailable: {reason}"
      ))),
    }
  }
}

impl Fenceable for BlockIndexerWorker {
  fn fence(&self, run_id: Option<Self::RunId>) {
    if let Ok(mut state) = self.scheduler_state.lock() {
      state.fence_active_run(run_id);
    }
  }
}

#[must_use]
pub(crate) fn block_indexer_supervised_task(
  ctx: IndexerLoopCtx,
  block_batch_sizer: Arc<std::sync::Mutex<BatchSizer>>,
  prefetch_blocks: u32,
  block_handle: IndexerHandle,
) -> SupervisedTaskSpec<BlockIndexerWorker> {
  let worker = Arc::clone(&ctx.block_worker);
  let initial = crate::worker::WorkerTask::new(worker.as_ref().clone(), {
    let ctx = ctx.clone();
    let block_batch_sizer = Arc::clone(&block_batch_sizer);
    let block_handle = block_handle.clone();
    async move {
      crate::index::runner::block_indexer_loop(
        ctx,
        block_batch_sizer,
        prefetch_blocks,
        block_handle,
      )
      .await
    }
  });
  SupervisedTaskSpec::new("block indexer", initial, move || {
    Ok(crate::worker::WorkerTask::new(worker.as_ref().clone(), {
      let ctx = ctx.clone();
      let block_batch_sizer = Arc::clone(&block_batch_sizer);
      let block_handle = block_handle.clone();
      async move {
        crate::index::runner::block_indexer_loop(
          ctx,
          block_batch_sizer,
          prefetch_blocks,
          block_handle,
        )
        .await
      }
    }))
  })
}

#[cfg(test)]
mod tests {
  use std::sync::atomic::AtomicBool;
  use std::sync::Arc;

  use super::BlockIndexerWorker;
  use crate::index::runner::IndexSchedulerState;
  use crate::ingest::IngestRuntime;
  use crate::lifecycle::Lifecycle;
  use crate::visor::{ProgressState, RecoveryDisposition, RunState, ShutdownPhase, Supervisable};

  #[tokio::test]
  async fn watchdog_blocks_while_control_plane_pause_is_active(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let db = Arc::new(crate::tests::open_temp_db()?);
    let ingest_runtime = Arc::new(IngestRuntime::default());
    let lifecycle = Lifecycle::start();
    let scheduler_state = Arc::new(std::sync::Mutex::new(IndexSchedulerState::default()));
    let control_plane_pause_active = Arc::new(AtomicBool::new(true));
    let worker = BlockIndexerWorker::new(
      db,
      ingest_runtime,
      Arc::clone(&lifecycle),
      scheduler_state,
      Arc::clone(&control_plane_pause_active),
    );
    let state = RunState {
      run_id: Some(1),
      phase: "ApplyStagePlan",
      subphase: Some("spent"),
      detail: Some("spent_mark_outputs"),
      progress_state: ProgressState::Active,
      last_activity_epoch: None,
      last_progress_epoch: None,
      recovery_attempts: 0,
      shutdown_phase: ShutdownPhase::Running,
    };

    assert_eq!(
      worker.watchdog_disposition(&state, 0),
      RecoveryDisposition::Blocked("ipc pause/rewind active")
    );

    Ok(())
  }
}
