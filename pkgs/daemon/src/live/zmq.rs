//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared ZMQ event fanout for live domain consumers.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::lifecycle::Lifecycle;
use crate::sources::{ZmqEvent, ZmqSource};

const EVENT_BUFFER_CAPACITY: usize = 1024;

/// Create a broadcast hub for typed node ZMQ events.
pub fn event_channel() -> (broadcast::Sender<ZmqEvent>, broadcast::Receiver<ZmqEvent>) {
  broadcast::channel(EVENT_BUFFER_CAPACITY)
}

/// Pump typed ZMQ events into a broadcast channel until shutdown.
pub async fn fanout_loop(
  source: Arc<dyn ZmqSource>,
  tx: broadcast::Sender<ZmqEvent>,
  lifecycle: Arc<Lifecycle>,
) {
  let mut shutdown = lifecycle.subscribe();
  while lifecycle.running() {
    let recv = source.recv_event();
    tokio::pin!(recv);
    match tokio::select! {
      result = &mut recv => Some(result),
      _ = shutdown.changed() => None,
    } {
      Some(Ok(event)) => {
        let _ = tx.send(event);
      }
      Some(Err(err)) => {
        tracing::warn!("live zmq receive failed: {err}");
        lifecycle.sleep(std::time::Duration::from_millis(250)).await;
      }
      None => break,
    }
  }
}
