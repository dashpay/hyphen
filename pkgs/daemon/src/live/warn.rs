//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Fleeting in-process warning rate limiter for optional live subsystems.

use std::collections::HashMap;
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

#[derive(Debug, Default)]
struct WarnState {
  last_emitted: Option<Instant>,
  suppressed: u64,
}

static WARN_STATES: OnceLock<Mutex<HashMap<&'static str, WarnState>>> = OnceLock::new();

/// Emit an optional-live warning at most once per interval.
///
/// Suppressed repeats are counted and passed to the next emitted log line.
pub fn warn_every<F>(key: &'static str, interval: Duration, emit: F)
where
  F: FnOnce(u64),
{
  let states = WARN_STATES.get_or_init(|| Mutex::new(HashMap::new()));
  let Ok(mut states) = states.lock() else {
    emit(0);
    return;
  };
  let state = states.entry(key).or_default();
  let now = Instant::now();
  if state
    .last_emitted
    .is_some_and(|last| now.duration_since(last) < interval)
  {
    state.suppressed += 1;
    return;
  }

  let suppressed = state.suppressed;
  state.last_emitted = Some(now);
  state.suppressed = 0;
  emit(suppressed);
}

#[cfg(test)]
mod tests {
  use std::sync::{Arc, Mutex};
  use std::time::Duration;

  #[test]
  fn warn_every_suppresses_repeated_emits_until_interval_passes() {
    let hits = Arc::new(Mutex::new(Vec::new()));

    super::warn_every("test_warn_every", Duration::from_millis(250), {
      let hits = Arc::clone(&hits);
      move |suppressed| hits.lock().expect("hits").push(suppressed)
    });
    super::warn_every("test_warn_every", Duration::from_millis(250), {
      let hits = Arc::clone(&hits);
      move |suppressed| hits.lock().expect("hits").push(suppressed)
    });

    std::thread::sleep(Duration::from_millis(300));
    super::warn_every("test_warn_every", Duration::from_millis(250), {
      let hits = Arc::clone(&hits);
      move |suppressed| hits.lock().expect("hits").push(suppressed)
    });

    assert_eq!(*hits.lock().expect("hits"), vec![0, 1]);
  }
}
