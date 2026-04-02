//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! External-facing ingest diagnostics.

use hyphend::wire::msg::{IngestionActivity, IngestionDiag, IngestionStateName};

use super::{watchdog, IngestRuntimeSnapshot};

fn activity_for_state(state: IngestionStateName) -> IngestionActivity {
  match state {
    IngestionStateName::Boot
    | IngestionStateName::Reconciling
    | IngestionStateName::PollingTip
    | IngestionStateName::Admitting
    | IngestionStateName::Fetching
    | IngestionStateName::Committing => IngestionActivity::Active,
    IngestionStateName::Paused | IngestionStateName::Throttled | IngestionStateName::Backoff => {
      IngestionActivity::Sleeping
    }
    IngestionStateName::Idle => IngestionActivity::Idle,
  }
}

/// Build the IPC-facing ingest diagnostic snapshot.
pub fn build_ingestion_diag(
  snapshot: &IngestRuntimeSnapshot,
  max_lead_blocks: u32,
  now_epoch: u64,
) -> IngestionDiag {
  let lead_blocks = match (snapshot.last_stored_tip, snapshot.last_indexed_height) {
    (Some(stored_tip), Some(indexed_height)) if indexed_height >= 0 => {
      Some(stored_tip.saturating_sub(indexed_height as u32))
    }
    _ => None,
  };
  IngestionDiag {
    activity: activity_for_state(snapshot.state),
    state: snapshot.state,
    paused: snapshot.pause_requested,
    throttled: snapshot.state == IngestionStateName::Throttled,
    last_polled_tip: snapshot.last_polled_tip,
    lead_blocks,
    max_lead_blocks,
    last_progress_epoch: snapshot.last_progress_epoch,
    last_throttle_clear_epoch: None,
    last_fetch_start_epoch: snapshot.last_fetch_start_epoch,
    current_batch_start: snapshot.current_batch_start,
    current_batch_end: snapshot.current_batch_end,
    last_state_transition_epoch: snapshot.last_transition_epoch,
    retry_attempt: snapshot.retry_attempt,
    backoff_until_epoch: snapshot.backoff_until_epoch,
    stalled: watchdog::is_stalled(snapshot, now_epoch),
  }
}
