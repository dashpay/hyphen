//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Force-reindex verb: pause indexers, reset stage, resume.

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

pub use crate::sql::verbs::reindex::force_reindex_stage_shared;

/// Execute a force-reindex command from the IPC control plane.
pub async fn reindex(ctx: CommandCtx, id: u64, stage_name: String) {
  let Some(stage) = index::Stage::from_name(&stage_name) else {
    let message = format!("unknown stage: {stage_name}");
    warn!(%message, "IPC: force_reindex rejected");
    let _ = ctx
      .event_tx
      .send(hyphend::wire::msg::DaemonMessage::CommandResult {
        id,
        success: false,
        message,
      });
    return;
  };

  info!(
    stage = stage.name(),
    "IPC: force_reindex -- pausing indexers"
  );
  let stages = stage.reset_stages().to_vec();
  ctx.ingestion_monitor.set_paused(true);
  ctx
    .control_plane_pause_active
    .store(true, Ordering::Relaxed);
  ctx.operator_pause_requested.store(false, Ordering::Relaxed);
  let mut wait_controls =
    super::pause_stages(ctx.controls.clone(), ctx.pausing_tx.clone(), &stages).await;
  info!(stage = stage.name(), "IPC: force_reindex waiting_for_idle");
  super::emit_command_progress(
    &ctx.event_tx,
    id,
    format!("reindex waiting_for_idle stage={}", stage.name()),
  );
  let blocked = super::wait_for_idle_with_timeout(
    &mut wait_controls,
    std::time::Duration::from_secs(WATCHDOG_GRACEFUL_IDLE_TIMEOUT_SECS),
  )
  .await;
  if !blocked.is_empty() {
    let state = ctx.block_worker.run_state();
    info!(
      stage = stage.name(),
      run_id = ?state.run_id,
      phase = state.phase,
      subphase = ?state.subphase,
      "IPC: force_reindex idle_timeout_escalating"
    );
    super::emit_command_progress(
      &ctx.event_tx,
      id,
      format!(
        "reindex idle_timeout_escalating blocked_stages={}",
        blocked
          .iter()
          .map(|blocked_stage| blocked_stage.name())
          .collect::<Vec<_>>()
          .join(",")
      ),
    );
    info!(stage = stage.name(), run_id = ?state.run_id, "IPC: force_reindex fencing_block_indexer");
    super::emit_command_progress(&ctx.event_tx, id, "reindex fencing_block_indexer");
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
        let message = format!("reindex escalation failed: {error}");
        warn!(stage = stage.name(), %message, "IPC: force_reindex failed");
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
        "reindex could not acquire idle block indexer after escalation: {}",
        still_blocked
          .iter()
          .map(|blocked_stage| blocked_stage.name())
          .collect::<Vec<_>>()
          .join(",")
      );
      warn!(stage = stage.name(), %message, "IPC: force_reindex failed");
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
    info!(stage = stage.name(), "IPC: force_reindex reindex_started");
    super::emit_command_progress(
      &ctx.event_tx,
      id,
      format!("reindex_started stage={}", stage.name()),
    );
    let guard =
      super::acquire_chain_state_write(Arc::clone(&ctx.chain_state_lock), "ipc_reindex").await;
    let hold_started = Instant::now();
    let result =
      force_reindex_stage_shared(ctx.pool.clone(), ctx.db.clone(), stage.name().to_owned()).await;
    let hold_ms = hold_started.elapsed().as_millis() as u64;
    drop(guard);
    tracing::info!(
      reason = "ipc_reindex",
      hold_ms,
      "chain-state write lock released"
    );
    result
  };

  ctx
    .control_plane_pause_active
    .store(false, Ordering::Relaxed);
  ctx.operator_pause_requested.store(false, Ordering::Relaxed);
  ctx
    .revalidate_schema
    .store(true, std::sync::atomic::Ordering::Relaxed);

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
    info!(%message, "IPC: force_reindex complete");
    super::emit_command_progress(&ctx.event_tx, id, format!("reindex_complete {message}"));
  } else {
    warn!(%message, "IPC: force_reindex failed");
  }
  let _ = ctx
    .event_tx
    .send(hyphend::wire::msg::DaemonMessage::CommandResult {
      id,
      success,
      message,
    });
}
