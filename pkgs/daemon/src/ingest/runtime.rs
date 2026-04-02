//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared runtime state for ingest diagnostics.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use hyphend::wire::msg::IngestionStateName;

use super::state::{InFlightBatch, IngestState};

fn now_epoch() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

#[derive(Debug, Clone)]
pub struct IngestRuntimeSnapshot {
  pub pause_requested: bool,
  pub state: IngestionStateName,
  pub last_polled_tip: Option<u32>,
  pub last_stored_tip: Option<u32>,
  pub last_indexed_height: Option<i64>,
  pub current_batch_start: Option<u32>,
  pub current_batch_end: Option<u32>,
  pub retry_attempt: Option<u32>,
  pub backoff_until_epoch: Option<u64>,
  pub blocks_ingested_total: u64,
  pub last_transition_epoch: Option<u64>,
  pub last_progress_epoch: Option<u64>,
  pub last_fetch_start_epoch: Option<u64>,
  pub last_commit_start_epoch: Option<u64>,
}

#[derive(Debug, Clone)]
struct RuntimeData {
  state: IngestionStateName,
  last_polled_tip: Option<u32>,
  last_stored_tip: Option<u32>,
  last_indexed_height: Option<i64>,
  current_batch_start: Option<u32>,
  current_batch_end: Option<u32>,
  retry_attempt: Option<u32>,
  backoff_until_epoch: Option<u64>,
  blocks_ingested_total: u64,
  last_transition_epoch: Option<u64>,
  last_progress_epoch: Option<u64>,
  last_fetch_start_epoch: Option<u64>,
  last_commit_start_epoch: Option<u64>,
}

impl Default for RuntimeData {
  fn default() -> Self {
    Self {
      state: IngestionStateName::Boot,
      last_polled_tip: None,
      last_stored_tip: None,
      last_indexed_height: None,
      current_batch_start: None,
      current_batch_end: None,
      retry_attempt: None,
      backoff_until_epoch: None,
      blocks_ingested_total: 0,
      last_transition_epoch: None,
      last_progress_epoch: None,
      last_fetch_start_epoch: None,
      last_commit_start_epoch: None,
    }
  }
}

/// Shared ingest runtime state used by IPC and control-plane code.
#[derive(Debug, Default)]
pub struct IngestRuntime {
  paused: AtomicBool,
  data: Mutex<RuntimeData>,
}

impl IngestRuntime {
  /// Set whether the control plane is requesting an ingest pause.
  pub fn set_paused(&self, paused: bool) {
    self.paused.store(paused, Ordering::Release);
  }

  /// Read whether a control-plane pause is currently requested.
  pub fn pause_requested(&self) -> bool {
    self.paused.load(Ordering::Acquire)
  }

  /// Record that the machine entered a new state.
  pub fn transition(&self, state: &IngestState) {
    let epoch = now_epoch();
    if let Ok(mut data) = self.data.lock() {
      data.state = state.name();
      data.last_transition_epoch = Some(epoch);
      let current_batch = state.current_batch_range();
      data.current_batch_start = current_batch.map(|(start, _)| start);
      data.current_batch_end = current_batch.map(|(_, end)| end);
      data.retry_attempt = state.retry_attempt();
      data.backoff_until_epoch = match state {
        IngestState::Backoff { until_epoch, .. } => Some(*until_epoch),
        _ => None,
      };
    }
  }

  /// Record the latest upstream tip seen by polling.
  pub fn record_polled_tip(&self, tip: u32) {
    if let Ok(mut data) = self.data.lock() {
      data.last_polled_tip = Some(tip);
    }
  }

  /// Record the latest RocksDB tip.
  pub fn record_stored_tip(&self, tip: u32) {
    if let Ok(mut data) = self.data.lock() {
      data.last_stored_tip = Some(tip);
    }
  }

  /// Record the latest indexed PostgreSQL height.
  pub fn record_indexed_height(&self, height: i64) {
    if let Ok(mut data) = self.data.lock() {
      data.last_indexed_height = Some(height);
    }
  }

  /// Record the start of a fetch phase.
  pub fn record_fetch_start(&self, batch: &InFlightBatch) {
    if let Ok(mut data) = self.data.lock() {
      data.current_batch_start = Some(batch.start_height);
      data.current_batch_end = Some(batch.end_height);
      data.retry_attempt = Some(batch.attempt);
      data.last_fetch_start_epoch = Some(now_epoch());
    }
  }

  /// Record the start of a commit phase.
  pub fn record_commit_start(&self, batch: &InFlightBatch) {
    if let Ok(mut data) = self.data.lock() {
      data.current_batch_start = Some(batch.start_height);
      data.current_batch_end = Some(batch.end_height);
      data.retry_attempt = Some(batch.attempt);
      data.last_commit_start_epoch = Some(now_epoch());
    }
  }

  /// Record one successful RocksDB commit.
  pub fn record_progress(&self, batch: &InFlightBatch) {
    if let Ok(mut data) = self.data.lock() {
      let epoch = now_epoch();
      data.current_batch_start = None;
      data.current_batch_end = None;
      data.retry_attempt = None;
      data.backoff_until_epoch = None;
      data.blocks_ingested_total = data
        .blocks_ingested_total
        .saturating_add((batch.end_height - batch.start_height + 1) as u64);
      data.last_progress_epoch = Some(epoch);
      data.last_stored_tip = Some(batch.end_height);
    }
  }

  /// Read a point-in-time snapshot of the ingest runtime.
  pub fn snapshot(&self) -> IngestRuntimeSnapshot {
    let data = self
      .data
      .lock()
      .map(|guard| guard.clone())
      .unwrap_or_default();
    IngestRuntimeSnapshot {
      pause_requested: self.pause_requested(),
      state: data.state,
      last_polled_tip: data.last_polled_tip,
      last_stored_tip: data.last_stored_tip,
      last_indexed_height: data.last_indexed_height,
      current_batch_start: data.current_batch_start,
      current_batch_end: data.current_batch_end,
      retry_attempt: data.retry_attempt,
      backoff_until_epoch: data.backoff_until_epoch,
      blocks_ingested_total: data.blocks_ingested_total,
      last_transition_epoch: data.last_transition_epoch,
      last_progress_epoch: data.last_progress_epoch,
      last_fetch_start_epoch: data.last_fetch_start_epoch,
      last_commit_start_epoch: data.last_commit_start_epoch,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::IngestRuntime;
  use crate::ingest::state::{InFlightBatch, IngestState, ThrottleStatus};
  use hyphend::wire::msg::{ForwardProgressState, IngestionStateName, SchedulerLoopPhase};

  #[test]
  fn ingest_runtime_tracks_pause_requests() {
    let runtime = IngestRuntime::default();

    assert!(!runtime.pause_requested());
    runtime.set_paused(true);
    assert!(runtime.pause_requested());
    runtime.set_paused(false);
    assert!(!runtime.pause_requested());
  }

  #[test]
  fn ingest_runtime_snapshot_records_transition_and_backoff() {
    let runtime = IngestRuntime::default();
    let throttled = IngestState::Throttled {
      next_height: 7,
      network_tip: 9,
      throttle: ThrottleStatus {
        indexed_height: 1,
        stored_tip: 2,
        allowed_tip: 3,
        lead_blocks: 1,
        active_lanes: vec!["forward"],
        scheduler_loop_phase: SchedulerLoopPhase::RunLane,
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
        forward_progress_state: ForwardProgressState::Idle,
        forward_stalled: false,
        forward_recovery_attempts: 0,
      },
    };

    runtime.transition(&throttled);
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.state, IngestionStateName::Throttled);
    assert_eq!(snapshot.current_batch_start, None);
    assert_eq!(snapshot.current_batch_end, None);
    assert!(snapshot.last_transition_epoch.is_some());

    let backoff = IngestState::backoff(
      IngestState::PollingTip { next_height: 7 },
      50,
      "tip polling failed".to_owned(),
    );
    runtime.transition(&backoff);
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.state, IngestionStateName::Backoff);
    assert_eq!(snapshot.backoff_until_epoch, Some(52));
  }

  #[test]
  fn ingest_runtime_snapshot_records_fetch_commit_and_progress() {
    let runtime = IngestRuntime::default();
    let batch = InFlightBatch::new(7, 9, 2).with_next_attempt();

    runtime.record_polled_tip(12);
    runtime.record_indexed_height(6);
    runtime.record_fetch_start(&batch);
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.last_polled_tip, Some(12));
    assert_eq!(snapshot.last_indexed_height, Some(6));
    assert_eq!(snapshot.current_batch_start, Some(7));
    assert_eq!(snapshot.current_batch_end, Some(9));
    assert_eq!(snapshot.retry_attempt, Some(2));
    assert!(snapshot.last_fetch_start_epoch.is_some());

    runtime.record_commit_start(&batch);
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.current_batch_start, Some(7));
    assert_eq!(snapshot.current_batch_end, Some(9));
    assert_eq!(snapshot.retry_attempt, Some(2));
    assert!(snapshot.last_commit_start_epoch.is_some());

    runtime.record_progress(&batch);
    let snapshot = runtime.snapshot();
    assert_eq!(snapshot.current_batch_start, None);
    assert_eq!(snapshot.current_batch_end, None);
    assert_eq!(snapshot.retry_attempt, None);
    assert_eq!(snapshot.last_stored_tip, Some(9));
    assert_eq!(snapshot.blocks_ingested_total, 3);
    assert!(snapshot.last_progress_epoch.is_some());
  }
}
