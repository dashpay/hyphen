//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! State-aware ingest watchdog classification.

use super::IngestRuntimeSnapshot;
use hyphend::wire::msg::IngestionStateName;

const STALL_WARN_SECS: u64 = 180;

/// Decide whether the ingest machine appears unexpectedly stalled.
pub fn is_stalled(snapshot: &IngestRuntimeSnapshot, now_epoch: u64) -> bool {
  match snapshot.state {
    IngestionStateName::Paused | IngestionStateName::Throttled | IngestionStateName::Backoff => {
      false
    }
    IngestionStateName::Fetching => snapshot
      .last_fetch_start_epoch
      .is_some_and(|epoch| now_epoch.saturating_sub(epoch) >= STALL_WARN_SECS),
    IngestionStateName::Committing => snapshot
      .last_commit_start_epoch
      .is_some_and(|epoch| now_epoch.saturating_sub(epoch) >= STALL_WARN_SECS),
    IngestionStateName::Idle => match (snapshot.last_polled_tip, snapshot.last_stored_tip) {
      (Some(polled_tip), Some(stored_tip)) if polled_tip > stored_tip => snapshot
        .last_transition_epoch
        .is_some_and(|epoch| now_epoch.saturating_sub(epoch) >= STALL_WARN_SECS),
      _ => false,
    },
    _ => false,
  }
}
