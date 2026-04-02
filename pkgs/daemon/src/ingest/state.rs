//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Ingest state-machine types.

use std::time::Duration;

use hyphend::wire::msg::IngestionStateName;

const INITIAL_BACKOFF_SECS: u64 = 2;
const MAX_BACKOFF_SECS: u64 = 60;

/// One admitted batch that is moving through fetch and commit phases.
#[derive(Debug, Clone)]
pub struct InFlightBatch {
  pub start_height: u32,
  pub end_height: u32,
  pub attempt: u32,
  pub fetch_started_at_epoch: Option<u64>,
  pub commit_started_at_epoch: Option<u64>,
}

impl InFlightBatch {
  pub fn new(start_height: u32, end_height: u32, admitted_at_epoch: u64) -> Self {
    let _ = admitted_at_epoch;
    Self {
      start_height,
      end_height,
      attempt: 1,
      fetch_started_at_epoch: None,
      commit_started_at_epoch: None,
    }
  }

  pub fn with_next_attempt(&self) -> Self {
    let mut next = self.clone();
    next.attempt = next.attempt.saturating_add(1);
    next.fetch_started_at_epoch = None;
    next.commit_started_at_epoch = None;
    next
  }
}

/// One fetched raw block ready for RocksDB commit.
#[derive(Debug, Clone)]
pub struct FetchedBlock {
  pub raw: Vec<u8>,
}

/// Backpressure details for one throttle decision.
#[derive(Debug, Clone)]
pub struct ThrottleStatus {
  pub indexed_height: u32,
  pub stored_tip: u32,
  pub allowed_tip: u32,
  pub lead_blocks: u32,
  pub active_lanes: Vec<&'static str>,
  pub scheduler_loop_phase: hyphend::wire::msg::SchedulerLoopPhase,
  pub scheduler_snapshot_epoch: Option<u64>,
  pub scheduler_loop_phase_elapsed_ms: Option<u64>,
  pub scheduler_last_turn_age_ms: Option<u64>,
  pub forward_phase: Option<hyphend::wire::msg::ForwardLanePhase>,
  pub forward_stage: Option<String>,
  pub forward_detail: Option<String>,
  pub forward_height: Option<u32>,
  pub forward_last_success_epoch: Option<u64>,
  pub forward_last_attempt_epoch: Option<u64>,
  pub forward_last_progress_epoch: Option<u64>,
  pub forward_progress_state: hyphend::wire::msg::ForwardProgressState,
  pub forward_stalled: bool,
  pub forward_recovery_attempts: u32,
}

/// Explicit ingest machine states.
#[derive(Debug)]
pub enum IngestState {
  Boot {
    next_height: u32,
  },
  Reconciling {
    next_height: u32,
  },
  PollingTip {
    next_height: u32,
  },
  Idle {
    next_height: u32,
    network_tip: u32,
  },
  Paused {
    next_height: u32,
  },
  Admitting {
    next_height: u32,
    network_tip: u32,
  },
  Throttled {
    next_height: u32,
    network_tip: u32,
    throttle: ThrottleStatus,
  },
  Fetching {
    next_height: u32,
    network_tip: u32,
    batch: InFlightBatch,
  },
  Committing {
    next_height: u32,
    network_tip: u32,
    batch: InFlightBatch,
    blocks: Vec<FetchedBlock>,
  },
  Backoff {
    next: Box<IngestState>,
    delay: Duration,
    until_epoch: u64,
    reason: String,
  },
}

impl IngestState {
  pub fn name(&self) -> IngestionStateName {
    match self {
      Self::Boot { .. } => IngestionStateName::Boot,
      Self::Reconciling { .. } => IngestionStateName::Reconciling,
      Self::PollingTip { .. } => IngestionStateName::PollingTip,
      Self::Idle { .. } => IngestionStateName::Idle,
      Self::Paused { .. } => IngestionStateName::Paused,
      Self::Admitting { .. } => IngestionStateName::Admitting,
      Self::Throttled { .. } => IngestionStateName::Throttled,
      Self::Fetching { .. } => IngestionStateName::Fetching,
      Self::Committing { .. } => IngestionStateName::Committing,
      Self::Backoff { .. } => IngestionStateName::Backoff,
    }
  }

  pub fn next_height(&self) -> Option<u32> {
    match self {
      Self::Boot { next_height }
      | Self::Reconciling { next_height }
      | Self::PollingTip { next_height }
      | Self::Idle { next_height, .. }
      | Self::Paused { next_height }
      | Self::Admitting { next_height, .. }
      | Self::Throttled { next_height, .. }
      | Self::Fetching { next_height, .. }
      | Self::Committing { next_height, .. } => Some(*next_height),
      Self::Backoff { next, .. } => next.next_height(),
    }
  }

  pub fn current_batch_range(&self) -> Option<(u32, u32)> {
    match self {
      Self::Fetching { batch, .. } | Self::Committing { batch, .. } => {
        Some((batch.start_height, batch.end_height))
      }
      Self::Backoff { next, .. } => next.current_batch_range(),
      _ => None,
    }
  }

  pub fn retry_attempt(&self) -> Option<u32> {
    match self {
      Self::Fetching { batch, .. } | Self::Committing { batch, .. } => Some(batch.attempt),
      Self::Backoff { next, .. } => next.retry_attempt(),
      _ => None,
    }
  }

  pub fn can_pause(&self) -> bool {
    !matches!(self, Self::Fetching { .. } | Self::Committing { .. })
  }

  pub fn backoff(next: IngestState, now_epoch: u64, reason: String) -> Self {
    let attempt = next.retry_attempt().unwrap_or(1).max(1);
    let mut delay_secs = INITIAL_BACKOFF_SECS;
    for _ in 1..attempt {
      delay_secs = (delay_secs * 2).min(MAX_BACKOFF_SECS);
    }
    Self::Backoff {
      until_epoch: now_epoch.saturating_add(delay_secs),
      delay: Duration::from_secs(delay_secs),
      next: Box::new(next),
      reason,
    }
  }
}

/// Typed transition result from one state handler.
#[derive(Debug)]
pub enum Transition {
  To(IngestState),
  Sleep(Duration, IngestState),
}
