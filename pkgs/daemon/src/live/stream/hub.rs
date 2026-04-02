//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Broadcast hub for public live stream events.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use tokio::sync::broadcast;

use crate::lifecycle::Lifecycle;
use crate::sources::{SequenceEvent, ZmqEvent};

use super::event::LiveEvent;

/// Broadcast sender for streamable live events.
#[derive(Debug)]
pub struct StreamHub {
  tx: broadcast::Sender<LiveEvent>,
}

impl StreamHub {
  /// Create a new stream hub with a bounded client buffer.
  pub fn new(capacity: usize) -> Self {
    let (tx, _) = broadcast::channel(capacity);
    Self { tx }
  }

  /// Subscribe one client to the live event stream.
  pub fn subscribe(&self) -> broadcast::Receiver<LiveEvent> {
    self.tx.subscribe()
  }

  /// Publish one event to all live stream subscribers.
  pub fn publish(&self, event: LiveEvent) {
    let _ = self.tx.send(event);
  }
}

/// Forward applicable ZMQ events into the public stream hub until shutdown.
pub async fn fanout_loop(
  mut rx: broadcast::Receiver<ZmqEvent>,
  hub: Arc<StreamHub>,
  lifecycle: Arc<Lifecycle>,
) {
  let mut pending_raw = PendingRawTxs::new(256);
  let mut pending_adds = PendingTxAdds::new(256);
  let mut shutdown = lifecycle.subscribe();
  while lifecycle.running() {
    match tokio::select! {
      result = rx.recv() => Some(result),
      _ = shutdown.changed() => None,
    } {
      Some(Ok(event)) => match event {
        ZmqEvent::RawTx { tx, sequence } => {
          let txid = dash_primitives::hash::tx_hash(&tx).0;
          if pending_adds.remove(&txid) {
            hub.publish(LiveEvent::mempool_tx(tx, sequence));
          } else {
            pending_raw.insert(txid, (tx, sequence));
          }
        }
        ZmqEvent::Sequence {
          event: SequenceEvent::TxAdded { txid, .. },
          ..
        } => {
          if let Some((tx, sequence)) = pending_raw.remove(&txid) {
            hub.publish(LiveEvent::mempool_tx(tx, sequence));
          } else {
            pending_adds.insert(txid);
          }
        }
        ZmqEvent::Sequence {
          event: SequenceEvent::TxRemoved { txid, .. },
          ..
        } => {
          pending_raw.remove(&txid);
          pending_adds.remove(&txid);
        }
        other => {
          if let Some(event) = LiveEvent::from_zmq(other) {
            hub.publish(event);
          }
        }
      },
      Some(Err(broadcast::error::RecvError::Lagged(skipped))) => {
        tracing::warn!(skipped, "live stream hub lagged behind ZMQ source");
      }
      Some(Err(broadcast::error::RecvError::Closed)) | None => break,
    }
  }
}

struct PendingRawTxs {
  cap: usize,
  order: VecDeque<[u8; 32]>,
  by_txid: HashMap<[u8; 32], (Vec<u8>, u32)>,
}

impl PendingRawTxs {
  fn new(cap: usize) -> Self {
    Self {
      cap,
      order: VecDeque::new(),
      by_txid: HashMap::new(),
    }
  }

  fn insert(&mut self, txid: [u8; 32], tx: (Vec<u8>, u32)) {
    if !self.by_txid.contains_key(&txid) {
      self.order.push_back(txid);
    }
    self.by_txid.insert(txid, tx);
    self.prune();
  }

  fn remove(&mut self, txid: &[u8; 32]) -> Option<(Vec<u8>, u32)> {
    self.by_txid.remove(txid)
  }

  fn prune(&mut self) {
    while self.by_txid.len() > self.cap {
      if let Some(oldest) = self.order.pop_front() {
        self.by_txid.remove(&oldest);
      } else {
        break;
      }
    }
  }
}

struct PendingTxAdds {
  cap: usize,
  order: VecDeque<[u8; 32]>,
  seen: HashMap<[u8; 32], ()>,
}

impl PendingTxAdds {
  fn new(cap: usize) -> Self {
    Self {
      cap,
      order: VecDeque::new(),
      seen: HashMap::new(),
    }
  }

  fn insert(&mut self, txid: [u8; 32]) {
    if !self.seen.contains_key(&txid) {
      self.order.push_back(txid);
    }
    self.seen.insert(txid, ());
    self.prune();
  }

  fn remove(&mut self, txid: &[u8; 32]) -> bool {
    self.seen.remove(txid).is_some()
  }

  fn prune(&mut self) {
    while self.seen.len() > self.cap {
      if let Some(oldest) = self.order.pop_front() {
        self.seen.remove(&oldest);
      } else {
        break;
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::StreamHub;
  use crate::lifecycle::Lifecycle;
  use crate::live::stream::LiveEvent;
  use crate::sources::{SequenceEvent, ZmqEvent};
  use std::sync::Arc;
  use tokio::sync::broadcast;

  #[tokio::test]
  async fn publish_delivers_event_to_subscribers() {
    let hub = StreamHub::new(8);
    let mut rx = hub.subscribe();
    hub.publish(LiveEvent::heartbeat());
    let event = rx.recv().await.expect("subscriber should receive event");
    assert!(matches!(event, LiveEvent::Heartbeat { .. }));
  }

  #[tokio::test]
  async fn fanout_only_streams_raw_tx_after_sequence_add() {
    let (tx, rx) = broadcast::channel(8);
    let hub = Arc::new(StreamHub::new(8));
    let lifecycle = Lifecycle::start();
    let mut out = hub.subscribe();

    let task = tokio::spawn(super::fanout_loop(rx, hub, lifecycle.clone()));
    tx.send(ZmqEvent::RawTx {
      tx: vec![1, 2, 3],
      sequence: 7,
    })
    .expect("raw tx should send");
    assert!(
      tokio::time::timeout(std::time::Duration::from_millis(50), out.recv())
        .await
        .is_err()
    );

    tx.send(ZmqEvent::Sequence {
      event: SequenceEvent::TxAdded {
        txid: dash_primitives::hash::tx_hash(&[1u8, 2, 3]).0,
        mempool_sequence: 9,
      },
      sequence: 8,
    })
    .expect("sequence should send");
    let event = out.recv().await.expect("mempool tx should publish");
    let json = serde_json::to_value(event).expect("event should serialize");
    assert_eq!(json["type"], "mempool_tx");
    drop(tx);
    lifecycle.stop();
    task.await.expect("fanout loop should stop");
  }
}
