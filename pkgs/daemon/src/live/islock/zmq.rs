//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live InstantSend tx-lock tracking backed by node ZMQ.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::lifecycle::Lifecycle;
use crate::live::state::LiveState;
use crate::sources::ZmqEvent;

/// Track current InstantSend tx-lock state in RAM from ZMQ until shutdown.
pub async fn sync_current_loop(
  live_state: Arc<LiveState>,
  mut rx: broadcast::Receiver<ZmqEvent>,
  lifecycle: Arc<Lifecycle>,
) {
  let mut shutdown = lifecycle.subscribe();
  while lifecycle.running() {
    match tokio::select! {
      result = rx.recv() => Some(result),
      _ = shutdown.changed() => None,
    } {
      Some(Ok(event)) => {
        if let Err(error) = handle_event(&live_state, event) {
          tracing::warn!(%error, "islock ZMQ event rejected");
        }
      }
      Some(Err(broadcast::error::RecvError::Lagged(skipped))) => {
        // Missed locks stay unknown; locks already seen are kept.
        tracing::warn!(skipped, "islock ZMQ consumer lagged");
      }
      Some(Err(broadcast::error::RecvError::Closed)) | None => break,
    }
  }
}

fn handle_event(live_state: &LiveState, event: ZmqEvent) -> Result<(), crate::error::DaemonError> {
  // Dash `rawtxlock` publishes the locked transaction bytes, so hashing the
  // payload yields the serialized-order txid used by every other live key.
  // A lock is permanent for its txid: removal, mining and reorgs keep it.
  if let ZmqEvent::RawTxLock { tx, .. } = event {
    live_state.record_islock(crate::consensus::txid(&tx)?);
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use std::sync::Arc;

  use tokio::sync::broadcast;

  use crate::live::state::LiveState;
  use crate::sources::zmq::test_wire;
  use crate::sources::ZmqEvent;

  fn lock() -> ZmqEvent {
    ZmqEvent::RawTxLock {
      tx: test_wire::raw_tx(),
      sequence: 1,
    }
  }

  fn txid() -> [u8; 32] {
    crate::consensus::txid(&test_wire::raw_tx()).expect("transaction txid")
  }

  #[test]
  fn locks_survive_removal_mining_and_readmission() {
    let state = LiveState::new();
    assert!(!state.has_islock(&txid()));
    super::handle_event(&state, lock()).expect("lock should apply");
    assert!(state.has_islock(&txid()));
    for event in [
      test_wire::sequence(test_wire::RAW_TX_DISPLAY_TXID, b'R', Some(2), 2),
      test_wire::sequence(&"11".repeat(32), b'C', None, 3),
      test_wire::sequence(&"11".repeat(32), b'D', None, 4),
      test_wire::sequence(test_wire::RAW_TX_DISPLAY_TXID, b'A', Some(3), 5),
    ] {
      super::handle_event(&state, event).expect("sequence event should apply");
      assert!(state.has_islock(&txid()));
    }
  }

  #[test]
  fn undecodable_lock_payloads_are_rejected_without_touching_known_locks() {
    let state = LiveState::new();
    super::handle_event(&state, lock()).expect("lock should apply");
    let unreadable = ZmqEvent::RawTxLock {
      tx: vec![1, 2, 3],
      sequence: 2,
    };
    assert!(super::handle_event(&state, unreadable).is_err());
    assert!(state.has_islock(&txid()));
  }

  #[tokio::test]
  async fn a_lagged_consumer_keeps_the_locks_it_saw() {
    let state = Arc::new(LiveState::new());
    let (tx, rx) = broadcast::channel(1);
    let lifecycle = crate::lifecycle::Lifecycle::start();
    let task = tokio::spawn(super::sync_current_loop(
      Arc::clone(&state),
      rx,
      lifecycle.clone(),
    ));
    tx.send(lock()).expect("lock should queue");
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
      while !state.has_islock(&txid()) {
        tokio::task::yield_now().await;
      }
    })
    .await
    .expect("lock should be recorded");
    // Overflow the one-slot channel so the consumer sees a gap, then wait for
    // a later lock to prove the gap was handled.
    for sequence in 0..4 {
      tx.send(ZmqEvent::RawTxLock {
        tx: vec![sequence],
        sequence: u32::from(sequence),
      })
      .expect("event should queue");
    }
    let mut later = test_wire::raw_tx();
    later[0] = 2;
    let later_txid = crate::consensus::txid(&later).expect("transaction txid");
    tx.send(ZmqEvent::RawTxLock {
      tx: later,
      sequence: 9,
    })
    .expect("later lock should queue");
    tokio::time::timeout(std::time::Duration::from_secs(2), async {
      while !state.has_islock(&later_txid) {
        tokio::task::yield_now().await;
      }
    })
    .await
    .expect("later lock should be recorded after the gap");
    lifecycle.stop();
    task.await.expect("islock loop should stop");
    assert!(state.has_islock(&txid()));
  }
}
