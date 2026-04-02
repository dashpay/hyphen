//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Rewind and disconnect-tip verb handlers.

use std::sync::atomic::Ordering;
use std::sync::Arc;
use std::time::Instant;
use tracing::{info, warn};

use super::CommandCtx;
use crate::index;
use crate::policy::watchdog::{
  WATCHDOG_ESCALATED_IDLE_TIMEOUT_SECS, WATCHDOG_GRACEFUL_IDLE_TIMEOUT_SECS,
};
use crate::visor::Supervisable;

pub use crate::sql::verbs::rewind as sql;

/// Execute a multi-block rewind from the IPC control plane.
pub async fn rewind(ctx: CommandCtx, id: u64, target_height: u32) {
  info!(target_height, "IPC: rewind -- pausing indexers");
  let stages = index::Stage::Block.reset_stages().to_vec();
  ctx.ingestion_monitor.set_paused(true);
  ctx
    .control_plane_pause_active
    .store(true, Ordering::Relaxed);
  ctx.operator_pause_requested.store(false, Ordering::Relaxed);
  let mut wait_controls =
    super::pause_stages(ctx.controls.clone(), ctx.pausing_tx.clone(), &stages).await;
  info!(target_height, "IPC: rewind waiting_for_idle");
  super::emit_command_progress(
    &ctx.event_tx,
    id,
    format!("rewind waiting_for_idle target_height={target_height}"),
  );
  let blocked = super::wait_for_idle_with_timeout(
    &mut wait_controls,
    std::time::Duration::from_secs(WATCHDOG_GRACEFUL_IDLE_TIMEOUT_SECS),
  )
  .await;
  if !blocked.is_empty() {
    let state = ctx.block_worker.run_state();
    info!(
      target_height,
      run_id = ?state.run_id,
      phase = state.phase,
      subphase = ?state.subphase,
      "IPC: rewind idle_timeout_escalating"
    );
    super::emit_command_progress(
      &ctx.event_tx,
      id,
      format!(
        "rewind idle_timeout_escalating blocked_stages={}",
        blocked
          .iter()
          .map(|stage| stage.name())
          .collect::<Vec<_>>()
          .join(",")
      ),
    );
    info!(target_height, run_id = ?state.run_id, "IPC: rewind fencing_block_indexer");
    super::emit_command_progress(&ctx.event_tx, id, "rewind fencing_block_indexer");
    let still_blocked = match super::force_block_indexer_idle_after_escalation(
      &ctx,
      &mut wait_controls,
      state.run_id,
      std::time::Duration::from_secs(WATCHDOG_ESCALATED_IDLE_TIMEOUT_SECS),
    )
    .await
    {
      Ok(still_blocked) => still_blocked,
      Err(error) => {
        ctx
          .control_plane_pause_active
          .store(false, Ordering::Relaxed);
        ctx.operator_pause_requested.store(false, Ordering::Relaxed);
        ctx.ingestion_monitor.set_paused(false);
        let controls = ctx.controls.lock().await;
        super::resume_and_revive(
          &controls,
          ctx.pausing_tx.clone(),
          ctx.suspend_tx.clone(),
          stages,
        );
        let message = format!("rewind escalation failed: {error}");
        warn!(target_height, %message, "IPC: rewind failed");
        let _ = ctx
          .event_tx
          .send(hyphend::wire::msg::DaemonMessage::CommandResult {
            id,
            success: false,
            message,
          });
        return;
      }
    };
    if !still_blocked.is_empty() {
      ctx
        .control_plane_pause_active
        .store(false, Ordering::Relaxed);
      ctx.operator_pause_requested.store(false, Ordering::Relaxed);
      ctx.ingestion_monitor.set_paused(false);
      let controls = ctx.controls.lock().await;
      super::resume_and_revive(
        &controls,
        ctx.pausing_tx.clone(),
        ctx.suspend_tx.clone(),
        stages,
      );
      let message = format!(
        "rewind could not acquire idle block indexer after escalation: {}",
        still_blocked
          .iter()
          .map(|stage| stage.name())
          .collect::<Vec<_>>()
          .join(",")
      );
      warn!(target_height, %message, "IPC: rewind failed");
      let _ = ctx
        .event_tx
        .send(hyphend::wire::msg::DaemonMessage::CommandResult {
          id,
          success: false,
          message,
        });
      return;
    }
  }
  super::mark_stages_suspended(ctx.pausing_tx.clone(), ctx.suspend_tx.clone(), &stages);

  let result = {
    info!(target_height, "IPC: rewind rewind_started");
    super::emit_command_progress(
      &ctx.event_tx,
      id,
      format!("rewind_started target_height={target_height}"),
    );
    let guard =
      super::acquire_chain_state_write(Arc::clone(&ctx.chain_state_lock), "ipc_rewind").await;
    let hold_started = Instant::now();
    let result = sql::rewind_to_height_shared(
      ctx.pool.clone(),
      ctx.db.clone(),
      Some(ctx.secondary_stores.clone()),
      target_height,
      ctx.p2pkh_version,
      ctx.p2sh_version,
      ctx.block_time_secs,
    )
    .await;
    let hold_ms = hold_started.elapsed().as_millis() as u64;
    drop(guard);
    tracing::info!(
      reason = "ipc_rewind",
      hold_ms,
      "chain-state write lock released"
    );
    result
  };

  ctx
    .control_plane_pause_active
    .store(false, Ordering::Relaxed);
  ctx.operator_pause_requested.store(false, Ordering::Relaxed);
  ctx.ingestion_monitor.set_paused(false);
  let controls = ctx.controls.lock().await;
  super::resume_and_revive(
    &controls,
    ctx.pausing_tx.clone(),
    ctx.suspend_tx.clone(),
    stages,
  );
  let (success, message) = match result {
    Ok(msg) => (true, msg),
    Err(e) => (false, format!("{e}")),
  };
  if success {
    info!(%message, "IPC: rewind complete");
    super::emit_command_progress(&ctx.event_tx, id, format!("rewind_complete {message}"));
  } else {
    warn!(%message, "IPC: rewind failed");
  }
  let _ = ctx
    .event_tx
    .send(hyphend::wire::msg::DaemonMessage::CommandResult {
      id,
      success,
      message,
    });
}

/// Execute a one-block disconnect from the IPC control plane.
pub async fn disconnect_tip(ctx: CommandCtx, id: u64) {
  info!("IPC: disconnect_tip -- pausing indexers");
  let stages = index::Stage::Block.reset_stages().to_vec();
  ctx.ingestion_monitor.set_paused(true);
  ctx
    .control_plane_pause_active
    .store(true, Ordering::Relaxed);
  ctx.operator_pause_requested.store(false, Ordering::Relaxed);
  let mut wait_controls =
    super::pause_stages(ctx.controls.clone(), ctx.pausing_tx.clone(), &stages).await;
  info!("IPC: disconnect_tip waiting_for_idle");
  super::emit_command_progress(&ctx.event_tx, id, "disconnect_tip waiting_for_idle");
  let blocked = super::wait_for_idle_with_timeout(
    &mut wait_controls,
    std::time::Duration::from_secs(WATCHDOG_GRACEFUL_IDLE_TIMEOUT_SECS),
  )
  .await;
  if !blocked.is_empty() {
    let state = ctx.block_worker.run_state();
    info!(
      run_id = ?state.run_id,
      phase = state.phase,
      subphase = ?state.subphase,
      "IPC: disconnect_tip idle_timeout_escalating"
    );
    super::emit_command_progress(&ctx.event_tx, id, "disconnect_tip idle_timeout_escalating");
    info!(run_id = ?state.run_id, "IPC: disconnect_tip fencing_block_indexer");
    super::emit_command_progress(&ctx.event_tx, id, "disconnect_tip fencing_block_indexer");
    let still_blocked = match super::force_block_indexer_idle_after_escalation(
      &ctx,
      &mut wait_controls,
      state.run_id,
      std::time::Duration::from_secs(WATCHDOG_ESCALATED_IDLE_TIMEOUT_SECS),
    )
    .await
    {
      Ok(still_blocked) => still_blocked,
      Err(error) => {
        ctx
          .control_plane_pause_active
          .store(false, Ordering::Relaxed);
        ctx.operator_pause_requested.store(false, Ordering::Relaxed);
        ctx.ingestion_monitor.set_paused(false);
        let controls = ctx.controls.lock().await;
        super::resume_and_revive(
          &controls,
          ctx.pausing_tx.clone(),
          ctx.suspend_tx.clone(),
          stages,
        );
        let message = format!("disconnect_tip escalation failed: {error}");
        warn!(%message, "IPC: disconnect_tip failed");
        let _ = ctx
          .event_tx
          .send(hyphend::wire::msg::DaemonMessage::CommandResult {
            id,
            success: false,
            message,
          });
        return;
      }
    };
    if !still_blocked.is_empty() {
      ctx
        .control_plane_pause_active
        .store(false, Ordering::Relaxed);
      ctx.operator_pause_requested.store(false, Ordering::Relaxed);
      ctx.ingestion_monitor.set_paused(false);
      let controls = ctx.controls.lock().await;
      super::resume_and_revive(
        &controls,
        ctx.pausing_tx.clone(),
        ctx.suspend_tx.clone(),
        stages,
      );
      let message = format!(
        "disconnect_tip could not acquire idle block indexer after escalation: {}",
        still_blocked
          .iter()
          .map(|stage| stage.name())
          .collect::<Vec<_>>()
          .join(",")
      );
      warn!(%message, "IPC: disconnect_tip failed");
      let _ = ctx
        .event_tx
        .send(hyphend::wire::msg::DaemonMessage::CommandResult {
          id,
          success: false,
          message,
        });
      return;
    }
  }
  super::mark_stages_suspended(ctx.pausing_tx.clone(), ctx.suspend_tx.clone(), &stages);

  let result = {
    info!("IPC: disconnect_tip rewind_started");
    super::emit_command_progress(&ctx.event_tx, id, "disconnect_tip rewind_started");
    let guard =
      super::acquire_chain_state_write(Arc::clone(&ctx.chain_state_lock), "ipc_disconnect_tip")
        .await;
    let hold_started = Instant::now();
    let result = sql::disconnect_tip_shared(
      ctx.pool.clone(),
      ctx.db.clone(),
      Some(ctx.secondary_stores.clone()),
      ctx.p2pkh_version,
      ctx.p2sh_version,
      ctx.block_time_secs,
    )
    .await;
    let hold_ms = hold_started.elapsed().as_millis() as u64;
    drop(guard);
    tracing::info!(
      reason = "ipc_disconnect_tip",
      hold_ms,
      "chain-state write lock released"
    );
    result
  };

  ctx
    .control_plane_pause_active
    .store(false, Ordering::Relaxed);
  ctx.operator_pause_requested.store(false, Ordering::Relaxed);
  ctx.ingestion_monitor.set_paused(false);
  let controls = ctx.controls.lock().await;
  super::resume_and_revive(
    &controls,
    ctx.pausing_tx.clone(),
    ctx.suspend_tx.clone(),
    stages,
  );
  let (success, message) = match result {
    Ok(msg) => (true, msg),
    Err(e) => (false, format!("{e}")),
  };
  if success {
    info!(%message, "IPC: disconnect_tip complete");
    super::emit_command_progress(
      &ctx.event_tx,
      id,
      format!("disconnect_tip rewind_complete {message}"),
    );
  } else {
    warn!(%message, "IPC: disconnect_tip failed");
  }
  let _ = ctx
    .event_tx
    .send(hyphend::wire::msg::DaemonMessage::CommandResult {
      id,
      success,
      message,
    });
}

#[cfg(test)]
mod tests {
  use std::collections::{BTreeSet, HashMap};
  use std::sync::atomic::AtomicBool;
  use std::sync::Arc;
  use std::time::Duration;

  use super::{rewind, CommandCtx};
  use crate::index::{self, Stage};
  use crate::ingest::IngestRuntime;
  use crate::lifecycle::Lifecycle;

  #[tokio::test(start_paused = true)]
  async fn rewind_emits_progress_through_escalation_and_completion(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let datadir = crate::tests::temp_datadir("hyphen-rewind-test");
    let db = Arc::new(crate::db::open(&datadir, None)?);
    let secondary_stores = Arc::new(crate::store::manager::SecondaryStoreSet::open(
      &datadir,
      &crate::config::ChainConfig::default(),
      None,
    )?);
    let lifecycle = Arc::new(Lifecycle::start());
    let ingest_runtime = Arc::new(IngestRuntime::default());
    let scheduler_state = Arc::new(std::sync::Mutex::new(
      index::runner::IndexSchedulerState::default(),
    ));
    let control_plane_pause_active = Arc::new(AtomicBool::new(false));
    let block_worker = Arc::new(index::BlockIndexerWorker::new(
      Arc::clone(&db),
      Arc::clone(&ingest_runtime),
      Arc::clone(&lifecycle),
      Arc::clone(&scheduler_state),
      Arc::clone(&control_plane_pause_active),
    ));
    let (event_tx, mut event_rx) = tokio::sync::broadcast::channel(32);
    let (pausing_tx, _) = tokio::sync::watch::channel(BTreeSet::new());
    let (suspend_tx, _) = tokio::sync::watch::channel(BTreeSet::new());
    let (control, handle) = index::indexer_channel(Duration::from_secs(1));
    let controls = Arc::new(tokio::sync::Mutex::new(HashMap::from([(
      Stage::Block,
      control,
    )])));
    let ctx = CommandCtx {
      pool,
      db,
      secondary_stores,
      event_tx,
      controls,
      pausing_tx,
      suspend_tx,
      revalidate_schema: Arc::new(AtomicBool::new(false)),
      ingestion_monitor: ingest_runtime,
      block_worker,
      control_plane_pause_active,
      operator_pause_requested: Arc::new(AtomicBool::new(false)),
      chain_state_lock: Arc::new(tokio::sync::RwLock::new(())),
      p2pkh_version: 0x4c,
      p2sh_version: 0x10,
      block_time_secs: 150.0,
    };

    tokio::spawn(async move {
      tokio::time::sleep(Duration::from_secs(6)).await;
      handle.idle_tx.send_replace(true);
    });

    let task = tokio::spawn(async move {
      rewind(ctx, 7, 0).await;
    });
    tokio::task::yield_now().await;
    tokio::time::advance(Duration::from_secs(7)).await;
    task.await?;

    let mut progress: Vec<String> = Vec::new();
    while let Ok(message) = event_rx.try_recv() {
      if let hyphend::wire::msg::DaemonMessage::CommandProgress { message, .. } = message {
        progress.push(message);
      }
    }

    assert!(progress
      .iter()
      .any(|message| message.contains("rewind waiting_for_idle")));
    assert!(progress
      .iter()
      .any(|message| message.contains("rewind idle_timeout_escalating")));
    assert!(progress
      .iter()
      .any(|message| message.contains("rewind fencing_block_indexer")));
    assert!(progress
      .iter()
      .any(|message| message.contains("rewind_started target_height=0")));
    assert!(progress
      .iter()
      .any(|message| message.contains("rewind_complete")));

    Ok(())
  }
}
