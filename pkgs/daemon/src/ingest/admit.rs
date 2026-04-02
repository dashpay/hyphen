//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Batch admission and backpressure decisions.

use std::time::{SystemTime, UNIX_EPOCH};

use crate::error::DaemonError;
use crate::index::Stage;
use crate::sql;

use super::reconcile::stored_tip_height;
use super::state::{InFlightBatch, ThrottleStatus};
use super::IngestCtx;

/// One scheduling decision for the next ingest step.
#[derive(Debug, Clone)]
pub enum AdmissionDecision {
  Idle,
  Throttle(ThrottleStatus),
  Admit(InFlightBatch),
}

/// Decide whether the next range can be admitted without exceeding the lead budget.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when the indexed block height query fails.
pub async fn admit_batch(
  ctx: &IngestCtx,
  next_height: u32,
  network_tip: u32,
) -> Result<AdmissionDecision, DaemonError> {
  if next_height > network_tip {
    return Ok(AdmissionDecision::Idle);
  }

  let indexed_height = sql::query::indexed_stage_height(&ctx.pool, Stage::Block).await?;
  ctx.runtime.record_indexed_height(i64::from(indexed_height));
  let indexed_height = if indexed_height < 0 {
    0
  } else {
    indexed_height as u32
  };
  let stored_tip = stored_tip_height(&ctx.db)?.unwrap_or(0);
  ctx.runtime.record_stored_tip(stored_tip);

  let allowed_tip = indexed_height.saturating_add(ctx.max_lead_blocks);
  if next_height > allowed_tip {
    let scheduler = ctx.block_worker.scheduler_diag();
    let active_lanes = scheduler
      .active_lanes
      .into_iter()
      .map(|lane| match lane {
        hyphend::wire::msg::IndexSchedulerLane::Idle => "idle",
        hyphend::wire::msg::IndexSchedulerLane::Forward => "forward",
        hyphend::wire::msg::IndexSchedulerLane::TxTreeCatchUp => "tx_tree_catch_up",
        hyphend::wire::msg::IndexSchedulerLane::MetricsCatchUp => "metrics_catch_up",
        hyphend::wire::msg::IndexSchedulerLane::ValidatorCatchUp => "validator_catch_up",
      })
      .collect();
    return Ok(AdmissionDecision::Throttle(ThrottleStatus {
      indexed_height,
      stored_tip,
      allowed_tip,
      lead_blocks: stored_tip.saturating_sub(indexed_height),
      active_lanes,
      scheduler_loop_phase: scheduler.loop_phase,
      scheduler_snapshot_epoch: scheduler.snapshot_epoch,
      scheduler_loop_phase_elapsed_ms: scheduler.loop_phase_elapsed_ms,
      scheduler_last_turn_age_ms: scheduler.last_turn_age_ms,
      forward_phase: scheduler.forward.phase,
      forward_stage: scheduler.forward.stage,
      forward_detail: scheduler.forward.substep,
      forward_height: scheduler.forward.height,
      forward_last_success_epoch: scheduler.forward.last_success_epoch,
      forward_last_attempt_epoch: scheduler.forward.last_attempt_epoch,
      forward_last_progress_epoch: scheduler.forward.last_progress_epoch,
      forward_progress_state: scheduler.forward.progress_state,
      forward_stalled: scheduler.forward.stalled,
      forward_recovery_attempts: scheduler.forward.recovery_attempts,
    }));
  }

  let desired_end = next_height
    .saturating_add(ctx.batch_size.saturating_sub(1))
    .min(network_tip);
  let approved_end = desired_end.min(allowed_tip);
  let admitted_at_epoch = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs();
  Ok(AdmissionDecision::Admit(InFlightBatch::new(
    next_height,
    approved_end,
    admitted_at_epoch,
  )))
}
