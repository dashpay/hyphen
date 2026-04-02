//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared log-throttling policy for block-oriented maintenance work.

use std::time::{Duration, Instant};

/// Emit one progress log after this many blocks even without time elapsed.
pub(crate) const LOG_BLOCK_STEP: u32 = 10_000;

/// Emit one progress log after this interval even without block movement.
pub(crate) const LOG_INTERVAL_SECS: u64 = 10;

/// Return `true` when one bounded progress log should be emitted.
pub(crate) fn log_threshold_met(last: Option<(u32, Instant)>, current: u32, end: u32) -> bool {
  current == end
    || last.is_none_or(|(last_height, last_time)| {
      current.saturating_sub(last_height) >= LOG_BLOCK_STEP
        || last_time.elapsed() >= Duration::from_secs(LOG_INTERVAL_SECS)
    })
}

#[cfg(test)]
mod tests {
  use super::{log_threshold_met, LOG_BLOCK_STEP};
  use std::time::{Duration, Instant};

  #[test]
  fn log_threshold_logs_at_boundaries() {
    let now = Instant::now();
    assert!(log_threshold_met(None, 0, 100));
    assert!(log_threshold_met(Some((0, now)), LOG_BLOCK_STEP, 20_000));
    assert!(log_threshold_met(Some((42, now)), 100, 100));
    assert!(!log_threshold_met(Some((1, now)), 2, 100));
  }

  #[test]
  fn log_threshold_logs_when_interval_elapses() {
    let earlier = Instant::now()
      .checked_sub(Duration::from_secs(11))
      .expect("instant subtraction should succeed");
    assert!(log_threshold_met(Some((1, earlier)), 2, 100));
  }
}
