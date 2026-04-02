//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Ingest state-machine coordinator.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::error::DaemonError;
use crate::policy::log::LOG_INTERVAL_SECS;

use super::admit::{self, AdmissionDecision};
use super::state::{IngestState, Transition};
use super::{commit, fetch, poll, reconcile, IngestCtx};

const POLL_INTERVAL_SECS: u64 = 60;
const PAUSE_POLL_DELAY_MS: u64 = 500;
const THROTTLE_POLL_SECS: u64 = 2;

fn now_epoch() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

fn log_state_entry(state: &IngestState) {
  match state {
    IngestState::Paused { next_height } => {
      tracing::info!(next_height, "ingest state -> paused");
    }
    IngestState::Throttled {
      next_height,
      network_tip,
      throttle,
    } => {
      tracing::info!(
        next_height,
        network_tip,
        indexed_height = throttle.indexed_height,
        stored_tip = throttle.stored_tip,
        allowed_tip = throttle.allowed_tip,
        lead_blocks = throttle.lead_blocks,
        active_lanes = ?throttle.active_lanes,
        scheduler_loop_phase = ?throttle.scheduler_loop_phase,
        scheduler_snapshot_epoch = throttle.scheduler_snapshot_epoch,
        scheduler_loop_phase_elapsed_ms = throttle.scheduler_loop_phase_elapsed_ms,
        scheduler_last_turn_age_ms = throttle.scheduler_last_turn_age_ms,
        forward_phase = ?throttle.forward_phase,
        forward_stage = throttle.forward_stage.as_deref().unwrap_or("-"),
        forward_detail = throttle.forward_detail.as_deref().unwrap_or("-"),
        forward_height = throttle.forward_height,
        forward_last_success_epoch = throttle.forward_last_success_epoch,
        forward_last_attempt_epoch = throttle.forward_last_attempt_epoch,
        forward_last_progress_epoch = throttle.forward_last_progress_epoch,
        forward_progress_state = ?throttle.forward_progress_state,
        forward_stalled = throttle.forward_stalled,
        forward_recovery_attempts = throttle.forward_recovery_attempts,
        "ingest state -> throttled"
      );
    }
    IngestState::Backoff {
      until_epoch,
      reason,
      ..
    } => {
      tracing::warn!(until_epoch, "ingest state -> backoff: {reason}");
    }
    IngestState::Idle {
      next_height,
      network_tip,
    } => {
      tracing::info!(next_height, network_tip, "ingest state -> idle");
    }
    _ => {}
  }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum StateLogKey {
  Other,
  Paused {
    next_height: u32,
  },
  Throttled {
    next_height: u32,
    network_tip: u32,
    indexed_height: u32,
    stored_tip: u32,
    allowed_tip: u32,
    lead_blocks: u32,
    active_lanes: Vec<String>,
    scheduler_loop_phase: String,
    scheduler_snapshot_epoch: Option<u64>,
    scheduler_loop_phase_elapsed_ms: Option<u64>,
    scheduler_last_turn_age_ms: Option<u64>,
    forward_phase: Option<String>,
    forward_stage: Option<String>,
    forward_detail: Option<String>,
    forward_height: Option<u32>,
    forward_last_success_epoch: Option<u64>,
    forward_last_attempt_epoch: Option<u64>,
    forward_last_progress_epoch: Option<u64>,
    forward_progress_state: hyphend::wire::msg::ForwardProgressState,
    forward_stalled: bool,
    forward_recovery_attempts: u32,
  },
  Backoff {
    until_epoch: u64,
    reason: String,
  },
  Idle {
    next_height: u32,
    network_tip: u32,
  },
}

fn state_log_key(state: &IngestState) -> StateLogKey {
  match state {
    IngestState::Paused { next_height } => StateLogKey::Paused {
      next_height: *next_height,
    },
    IngestState::Throttled {
      next_height,
      network_tip,
      throttle,
    } => StateLogKey::Throttled {
      next_height: *next_height,
      network_tip: *network_tip,
      indexed_height: throttle.indexed_height,
      stored_tip: throttle.stored_tip,
      allowed_tip: throttle.allowed_tip,
      lead_blocks: throttle.lead_blocks,
      active_lanes: throttle
        .active_lanes
        .iter()
        .map(|lane| (*lane).to_owned())
        .collect(),
      scheduler_loop_phase: format!("{:?}", throttle.scheduler_loop_phase),
      scheduler_snapshot_epoch: throttle.scheduler_snapshot_epoch,
      scheduler_loop_phase_elapsed_ms: throttle.scheduler_loop_phase_elapsed_ms,
      scheduler_last_turn_age_ms: throttle.scheduler_last_turn_age_ms,
      forward_phase: throttle.forward_phase.map(|phase| format!("{phase:?}")),
      forward_stage: throttle.forward_stage.clone(),
      forward_detail: throttle.forward_detail.clone(),
      forward_height: throttle.forward_height,
      forward_last_success_epoch: throttle.forward_last_success_epoch,
      forward_last_attempt_epoch: throttle.forward_last_attempt_epoch,
      forward_last_progress_epoch: throttle.forward_last_progress_epoch,
      forward_progress_state: throttle.forward_progress_state,
      forward_stalled: throttle.forward_stalled,
      forward_recovery_attempts: throttle.forward_recovery_attempts,
    },
    IngestState::Backoff {
      until_epoch,
      reason,
      ..
    } => StateLogKey::Backoff {
      until_epoch: *until_epoch,
      reason: reason.clone(),
    },
    IngestState::Idle {
      next_height,
      network_tip,
    } => StateLogKey::Idle {
      next_height: *next_height,
      network_tip: *network_tip,
    },
    _ => StateLogKey::Other,
  }
}

fn should_force_pause_transition(state: &IngestState, pause_requested: bool) -> bool {
  pause_requested && state.can_pause() && !matches!(state, IngestState::Paused { .. })
}

async fn transition_to(
  ctx: &IngestCtx,
  current: Option<&StateLogKey>,
  next: IngestState,
) -> Result<IngestState, DaemonError> {
  let next_key = state_log_key(&next);
  ctx.runtime.transition(&next);
  if current != Some(&next_key) {
    log_state_entry(&next);
  }
  Ok(next)
}

async fn apply_transition(
  ctx: &IngestCtx,
  current: &StateLogKey,
  transition: Transition,
) -> Result<IngestState, DaemonError> {
  match transition {
    Transition::To(next) => transition_to(ctx, Some(current), next).await,
    Transition::Sleep(delay, next) => {
      let next = transition_to(ctx, Some(current), next).await?;
      ctx.lifecycle.sleep(delay).await;
      Ok(next)
    }
  }
}

/// Run the ingest state machine until shutdown or fatal failure.
///
/// # Errors
///
/// Returns `DaemonError` when reconciliation, polling, admission, fetch, or
/// RocksDB commit work fails permanently.
pub async fn run(ctx: IngestCtx) -> Result<(), DaemonError> {
  let next_height = reconcile::stored_tip_height(&ctx.db)?.map_or(0, |tip| tip.saturating_add(1));
  let mut state = transition_to(&ctx, None, IngestState::Boot { next_height }).await?;
  let mut last_progress_log: Option<Instant> = None;

  while ctx.lifecycle.running() {
    if should_force_pause_transition(&state, ctx.runtime.pause_requested()) {
      state = transition_to(
        &ctx,
        Some(&state_log_key(&state)),
        IngestState::Paused {
          next_height: state.next_height().unwrap_or(0),
        },
      )
      .await?;
    }
    let current_key = state_log_key(&state);

    let transition = match state {
      IngestState::Boot { next_height } => Transition::To(IngestState::Reconciling { next_height }),
      IngestState::Reconciling { next_height } => {
        let reconciled = reconcile::reconcile_start(&ctx, next_height).await?;
        Transition::To(IngestState::PollingTip {
          next_height: reconciled,
        })
      }
      IngestState::PollingTip { next_height } => match poll::poll_tip(&ctx).await {
        Ok(network_tip) => {
          if next_height <= network_tip {
            Transition::To(IngestState::Admitting {
              next_height,
              network_tip,
            })
          } else {
            Transition::Sleep(
              Duration::from_secs(POLL_INTERVAL_SECS),
              IngestState::Idle {
                next_height,
                network_tip,
              },
            )
          }
        }
        Err(error) => Transition::To(IngestState::backoff(
          IngestState::PollingTip { next_height },
          now_epoch(),
          format!("tip polling failed: {error}"),
        )),
      },
      IngestState::Idle { next_height, .. } => {
        Transition::To(IngestState::PollingTip { next_height })
      }
      IngestState::Paused { next_height } => {
        if ctx.runtime.pause_requested() {
          Transition::Sleep(
            Duration::from_millis(PAUSE_POLL_DELAY_MS),
            IngestState::Paused { next_height },
          )
        } else {
          Transition::To(IngestState::Reconciling { next_height })
        }
      }
      IngestState::Admitting {
        next_height,
        network_tip,
      } => match admit::admit_batch(&ctx, next_height, network_tip).await? {
        AdmissionDecision::Idle => Transition::Sleep(
          Duration::from_secs(POLL_INTERVAL_SECS),
          IngestState::Idle {
            next_height,
            network_tip,
          },
        ),
        AdmissionDecision::Throttle(throttle) => Transition::Sleep(
          Duration::from_secs(THROTTLE_POLL_SECS),
          IngestState::Throttled {
            next_height,
            network_tip,
            throttle,
          },
        ),
        AdmissionDecision::Admit(batch) => Transition::To(IngestState::Fetching {
          next_height,
          network_tip,
          batch,
        }),
      },
      IngestState::Throttled {
        next_height,
        network_tip,
        throttle: _,
      } => Transition::To(IngestState::Admitting {
        next_height,
        network_tip,
      }),
      IngestState::Fetching {
        next_height,
        network_tip,
        batch,
      } => match fetch::fetch_batch(&ctx, &batch).await {
        Ok(blocks) => Transition::To(IngestState::Committing {
          next_height,
          network_tip,
          batch,
          blocks,
        }),
        Err(error) => Transition::To(IngestState::backoff(
          IngestState::Fetching {
            next_height,
            network_tip,
            batch: batch.with_next_attempt(),
          },
          now_epoch(),
          format!("batch fetch failed: {error}"),
        )),
      },
      IngestState::Committing {
        next_height,
        network_tip,
        batch,
        blocks,
      } => match commit::commit_batch(&ctx, &batch, &blocks) {
        Ok(()) => {
          let should_log_progress = last_progress_log
            .is_none_or(|instant| instant.elapsed() >= Duration::from_secs(LOG_INTERVAL_SECS));
          if should_log_progress || batch.end_height == network_tip {
            tracing::info!(
              batch_end = batch.end_height,
              tip = network_tip,
              "ingest progress"
            );
            last_progress_log = Some(Instant::now());
          }
          let next_height = batch.end_height.saturating_add(1);
          if next_height <= network_tip {
            Transition::To(IngestState::Admitting {
              next_height,
              network_tip,
            })
          } else {
            Transition::To(IngestState::PollingTip { next_height })
          }
        }
        Err(error) => Transition::To(IngestState::backoff(
          IngestState::Committing {
            next_height,
            network_tip,
            batch: batch.with_next_attempt(),
            blocks,
          },
          now_epoch(),
          format!("batch commit failed: {error}"),
        )),
      },
      IngestState::Backoff {
        next,
        delay,
        until_epoch: _,
        reason: _,
      } => Transition::Sleep(delay, *next),
    };

    state = apply_transition(&ctx, &current_key, transition).await?;
  }

  Ok(())
}

#[cfg(test)]
mod tests {
  use super::should_force_pause_transition;
  use crate::ingest::state::{InFlightBatch, IngestState, ThrottleStatus};

  #[test]
  fn pause_requested_does_not_reenter_paused_state() {
    assert!(!should_force_pause_transition(
      &IngestState::Paused { next_height: 42 },
      true,
    ));
  }

  #[test]
  fn pause_requested_transitions_from_polling_state() {
    assert!(should_force_pause_transition(
      &IngestState::PollingTip { next_height: 42 },
      true,
    ));
  }

  #[test]
  fn pause_requested_does_not_interrupt_fetch_or_commit() {
    let batch = InFlightBatch::new(7, 9, 1);
    assert!(!should_force_pause_transition(
      &IngestState::Fetching {
        next_height: 7,
        network_tip: 9,
        batch: batch.clone(),
      },
      true,
    ));
    assert!(!should_force_pause_transition(
      &IngestState::Committing {
        next_height: 7,
        network_tip: 9,
        batch,
        blocks: vec![],
      },
      true,
    ));
  }

  #[test]
  fn pause_requested_can_transition_from_throttled_state() {
    assert!(should_force_pause_transition(
      &IngestState::Throttled {
        next_height: 7,
        network_tip: 9,
        throttle: ThrottleStatus {
          indexed_height: 1,
          stored_tip: 2,
          allowed_tip: 3,
          lead_blocks: 1,
          active_lanes: vec!["forward"],
          scheduler_loop_phase: hyphend::wire::msg::SchedulerLoopPhase::RunLane,
          scheduler_snapshot_epoch: Some(10),
          scheduler_loop_phase_elapsed_ms: Some(100),
          scheduler_last_turn_age_ms: Some(50),
          forward_phase: None,
          forward_stage: None,
          forward_detail: None,
          forward_height: None,
          forward_last_success_epoch: Some(9),
          forward_last_attempt_epoch: Some(10),
          forward_last_progress_epoch: Some(10),
          forward_progress_state: hyphend::wire::msg::ForwardProgressState::Idle,
          forward_stalled: false,
          forward_recovery_attempts: 0,
        },
      },
      true,
    ));
  }
}
