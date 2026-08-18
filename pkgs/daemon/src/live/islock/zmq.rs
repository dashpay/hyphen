//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live InstantSend tx-lock tracking backed by node ZMQ.

use std::sync::Arc;

use tokio::sync::broadcast;

use dash_primitives::hash;

use crate::lifecycle::Lifecycle;
use crate::live::state::{IsLock, LiveState};
use crate::sources::{SequenceEvent, ZmqEvent};
use crate::utils::{self, HashEndian};

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
      Some(Ok(event)) => handle_event(&live_state, event),
      Some(Err(broadcast::error::RecvError::Lagged(skipped))) => {
        tracing::warn!(skipped, "islock ZMQ consumer lagged");
      }
      Some(Err(broadcast::error::RecvError::Closed)) | None => break,
    }
  }
}

fn handle_event(live_state: &LiveState, event: ZmqEvent) {
  match event {
    // Dash `rawtxlock` publishes the locked transaction bytes, so hashing the
    // payload yields the same txid used by `sequence` removals and `getislocks`.
    ZmqEvent::RawTxLock { tx, .. } => {
      let txid = utils::ser_endian(&hash::tx_hash(&tx).to_bytes(), HashEndian::Ser);
      live_state.upsert_islock(IsLock {
        txid,
        raw_islock: tx,
        seen_at: now_epoch_secs(),
      });
    }
    ZmqEvent::Sequence {
      event: SequenceEvent::TxRemoved { txid, .. },
      ..
    } => {
      let txid = utils::ser_endian(&txid, HashEndian::Ser);
      live_state.remove_islock(&txid);
    }
    _ => {}
  }
}

fn now_epoch_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs() as i64
}
