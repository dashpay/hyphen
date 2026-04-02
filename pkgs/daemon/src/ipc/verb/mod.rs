//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! IPC verb handlers dispatched from the control plane.

pub mod compact;
pub mod reindex;
pub mod rewind;

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::index;
use crate::visor::Fenceable;
use hyphend::wire::msg::{DaemonCommand, DaemonMessage};
use tracing::warn;

/// Resources the command handler needs.
#[derive(Clone)]
pub struct CommandCtx {
  pub pool: crate::sql::SqlPool,
  pub db: Arc<rocksdb::DB>,
  pub secondary_stores: Arc<crate::store::manager::SecondaryStoreSet>,
  pub event_tx: tokio::sync::broadcast::Sender<DaemonMessage>,
  pub controls:
    Arc<tokio::sync::Mutex<std::collections::HashMap<index::Stage, index::IndexerControl>>>,
  pub pausing_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  pub suspend_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  pub revalidate_schema: Arc<AtomicBool>,
  pub ingestion_monitor: Arc<crate::ingest::IngestRuntime>,
  pub block_worker: Arc<crate::index::BlockIndexerWorker>,
  pub control_plane_pause_active: Arc<AtomicBool>,
  pub operator_pause_requested: Arc<AtomicBool>,
  pub chain_state_lock: Arc<tokio::sync::RwLock<()>>,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub block_time_secs: f64,
}

impl std::fmt::Debug for CommandCtx {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("CommandCtx")
      .field("pool", &"<PgPool>")
      .field("db", &"<RocksDb>")
      .field("secondary_stores", &"<SecondaryStoreSet>")
      .field("event_tx", &self.event_tx)
      .field(
        "controls",
        &"<tokio::sync::Mutex<HashMap<Stage, IndexerControl>>>",
      )
      .field("pausing_tx", &"<watch::Sender>")
      .field("suspend_tx", &"<watch::Sender>")
      .field(
        "revalidate_schema",
        &self
          .revalidate_schema
          .load(std::sync::atomic::Ordering::Relaxed),
      )
      .field("ingestion_monitor", &self.ingestion_monitor.snapshot())
      .field("block_worker", &"<BlockIndexerWorker>")
      .field(
        "control_plane_pause_active",
        &self.control_plane_pause_active.load(Ordering::Relaxed),
      )
      .field(
        "operator_pause_requested",
        &self.operator_pause_requested.load(Ordering::Relaxed),
      )
      .field("chain_state_lock", &"<RwLock>")
      .field("p2pkh_version", &self.p2pkh_version)
      .field("p2sh_version", &self.p2sh_version)
      .field("block_time_secs", &self.block_time_secs)
      .finish()
  }
}

pub(crate) fn emit_command_progress(
  event_tx: &tokio::sync::broadcast::Sender<DaemonMessage>,
  id: u64,
  message: impl Into<String>,
) {
  let _ = event_tx.send(DaemonMessage::CommandProgress {
    id,
    message: message.into(),
  });
}

pub(crate) async fn pause_stages(
  controls: Arc<tokio::sync::Mutex<std::collections::HashMap<index::Stage, index::IndexerControl>>>,
  pausing_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  stages: &[index::Stage],
) -> Vec<(index::Stage, index::IndexerControl)> {
  let wait_controls = {
    let controls = controls.lock().await;
    stages
      .iter()
      .filter_map(|stage| controls.get(stage).cloned().map(|ctrl| (*stage, ctrl)))
      .collect::<Vec<_>>()
  };
  let controls = controls.lock().await;
  for &stage in stages {
    if let Some(ctrl) = controls.get(&stage) {
      ctrl.pause();
    }
  }
  let mut pausing = pausing_tx.borrow().clone();
  for &stage in stages {
    pausing.insert(stage.name().to_string());
  }
  let _ = pausing_tx.send(pausing);
  wait_controls
}

pub(crate) async fn wait_for_idle_with_timeout(
  wait_controls: &mut [(index::Stage, index::IndexerControl)],
  timeout: Duration,
) -> Vec<index::Stage> {
  let mut timed_out = Vec::new();
  for (stage, ctrl) in wait_controls.iter_mut() {
    if !ctrl.wait_idle_for(timeout).await {
      warn!(
        stage = stage.name(),
        timeout_secs = timeout.as_secs(),
        "timed out waiting for indexer idle"
      );
      timed_out.push(*stage);
    }
  }
  timed_out
}

pub(crate) async fn force_block_indexer_idle_after_escalation(
  ctx: &CommandCtx,
  wait_controls: &mut [(index::Stage, index::IndexerControl)],
  run_id: Option<u64>,
  timeout: Duration,
) -> Result<Vec<index::Stage>, crate::error::DaemonError> {
  ctx.operator_pause_requested.store(true, Ordering::Relaxed);
  ctx.block_worker.fence(run_id);
  ctx.block_worker.force_control_plane_recover(run_id).await?;
  Ok(wait_for_idle_with_timeout(wait_controls, timeout).await)
}

pub(crate) fn mark_stages_suspended(
  pausing_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  suspend_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  stages: &[index::Stage],
) {
  let mut pausing = pausing_tx.borrow().clone();
  let mut suspended = suspend_tx.borrow().clone();
  for &stage in stages {
    pausing.remove(stage.name());
    suspended.insert(stage.name().to_string());
  }
  let _ = pausing_tx.send(pausing);
  let _ = suspend_tx.send(suspended);
}

/// Drain the inbound command channel and dispatch each verb handler.
pub async fn handle_commands(
  mut cmd_rx: tokio::sync::mpsc::Receiver<DaemonCommand>,
  ctx: CommandCtx,
  lifecycle: std::sync::Arc<crate::lifecycle::Lifecycle>,
) {
  let mut shutdown_rx = lifecycle.subscribe();
  while lifecycle.running() {
    let cmd = tokio::select! {
      maybe_cmd = cmd_rx.recv() => maybe_cmd,
      changed = shutdown_rx.changed() => {
        if changed.is_err() || *shutdown_rx.borrow_and_update() {
          None
        } else {
          continue;
        }
      }
    };
    let Some(cmd) = cmd else {
      break;
    };
    match cmd {
      DaemonCommand::Rewind { id, target_height } => {
        rewind::rewind(ctx.clone(), id, target_height).await;
      }
      DaemonCommand::DisconnectTip { id } => {
        rewind::disconnect_tip(ctx.clone(), id).await;
      }
      DaemonCommand::ForceCompact { id, ref cf } => {
        compact::compact(ctx.clone(), id, cf.clone()).await;
      }
      DaemonCommand::ForceReindex { id, ref stage } => {
        reindex::reindex(ctx.clone(), id, stage.clone()).await;
      }
    }
  }
}

/// Pause all listed stages, wait for idle, then mark suspended.
pub(crate) async fn pause_and_wait(
  controls: Arc<tokio::sync::Mutex<std::collections::HashMap<index::Stage, index::IndexerControl>>>,
  pausing_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  suspend_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  stages: Vec<index::Stage>,
) {
  let mut wait_controls = pause_stages(controls, pausing_tx.clone(), &stages).await;
  let _ = wait_for_idle_with_timeout(&mut wait_controls, Duration::from_secs(60)).await;
  mark_stages_suspended(pausing_tx, suspend_tx, &stages);
}

/// Resume all listed stages and clear from pausing/suspended sets.
pub(crate) fn resume_and_revive(
  controls: &std::collections::HashMap<index::Stage, index::IndexerControl>,
  pausing_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  suspend_tx: tokio::sync::watch::Sender<std::collections::BTreeSet<String>>,
  stages: Vec<index::Stage>,
) {
  for &stage in &stages {
    if let Some(ctrl) = controls.get(&stage) {
      ctrl.resume();
    }
  }
  let mut pausing = pausing_tx.borrow().clone();
  let mut suspended = suspend_tx.borrow().clone();
  for &stage in &stages {
    pausing.remove(stage.name());
    suspended.remove(stage.name());
  }
  let _ = pausing_tx.send(pausing);
  let _ = suspend_tx.send(suspended);
}

pub(crate) async fn acquire_chain_state_write(
  lock: Arc<tokio::sync::RwLock<()>>,
  reason: &'static str,
) -> tokio::sync::OwnedRwLockWriteGuard<()> {
  let wait_started = Instant::now();
  let guard = lock.write_owned().await;
  let wait_ms = wait_started.elapsed().as_millis() as u64;
  if wait_ms >= 1_000 {
    tracing::warn!(reason, wait_ms, "chain-state write lock waited");
  } else {
    tracing::debug!(reason, wait_ms, "chain-state write lock acquired");
  }
  guard
}
