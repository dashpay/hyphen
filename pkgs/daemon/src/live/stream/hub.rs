//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Broadcast hub for public live stream events.

use std::collections::{HashMap, VecDeque};
use std::num::NonZeroUsize;
use std::sync::Arc;

use lru::LruCache;
use tokio::sync::broadcast;

use crate::lifecycle::Lifecycle;
use crate::live::islock::state::KnownLocks;
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

/// Unlocked mempool rows already published, kept for a late lock re-send.
const MAX_ANNOUNCED_UNLOCKED: usize = 10_000;

/// Forward applicable ZMQ events into the public stream hub until shutdown.
///
/// A `mempool_tx` row carries `live.tags.islock=true` when a lock for its txid
/// is known. A lock that arrives after an unlocked row was published re-sends
/// that row with the lock's topic sequence, while the transaction is still
/// known to be pending: a removal, a connected block, a gap in the `sequence`
/// topic or a lagged receiver ends that window. With `islock_enabled` off,
/// locks are ignored, as they are for REST.
pub async fn fanout_loop(
  mut rx: broadcast::Receiver<ZmqEvent>,
  hub: Arc<StreamHub>,
  islock_enabled: bool,
  lifecycle: Arc<Lifecycle>,
) {
  let mut pending_raw = PendingRawTxs::new(256);
  let mut pending_adds = PendingTxAdds::new(256);
  let mut locks = StreamLocks::default();
  let mut last_sequence: Option<u32> = None;
  let mut shutdown = lifecycle.subscribe();
  while lifecycle.running() {
    match tokio::select! {
      result = rx.recv() => Some(result),
      _ = shutdown.changed() => None,
    } {
      Some(Ok(event)) => {
        if let ZmqEvent::Sequence { sequence, .. } = &event {
          if last_sequence.is_some_and(|last| *sequence != last.wrapping_add(1)) {
            locks.forget_announced();
          }
          last_sequence = Some(*sequence);
        }
        match event {
          ZmqEvent::RawTx { tx, sequence } => {
            let txid = match crate::consensus::txid(&tx) {
              Ok(txid) => txid,
              Err(error) => {
                tracing::debug!(%error, "dropping undecodable raw mempool transaction");
                continue;
              }
            };
            if pending_adds.remove(&txid) {
              let locked = locks.published(txid);
              hub.publish(LiveEvent::mempool_tx(txid, tx, sequence, locked));
            } else {
              pending_raw.insert(txid, (tx, sequence));
            }
          }
          ZmqEvent::Sequence {
            event: SequenceEvent::TxAdded { txid, .. },
            ..
          } => {
            if let Some((tx, sequence)) = pending_raw.remove(&txid) {
              let locked = locks.published(txid);
              hub.publish(LiveEvent::mempool_tx(txid, tx, sequence, locked));
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
            locks.removed(&txid);
          }
          ZmqEvent::Sequence {
            event: SequenceEvent::BlockConnected { .. },
            ..
          } => {
            // Rows published before this block may now be mined.
            locks.forget_announced();
          }
          ZmqEvent::RawTxLock { tx, sequence } if islock_enabled => {
            let txid = match crate::consensus::txid(&tx) {
              Ok(txid) => txid,
              Err(error) => {
                tracing::debug!(%error, "dropping undecodable raw transaction lock");
                continue;
              }
            };
            if locks.locked(txid) {
              hub.publish(LiveEvent::mempool_tx(txid, tx, sequence, true));
            }
          }
          other => {
            if let Some(event) = LiveEvent::from_zmq(other) {
              hub.publish(event);
            }
          }
        }
      }
      Some(Err(broadcast::error::RecvError::Lagged(skipped))) => {
        locks.forget_announced();
        tracing::warn!(skipped, "live stream hub lagged behind ZMQ source");
      }
      Some(Err(broadcast::error::RecvError::Closed)) | None => break,
    }
  }
}

/// Known locks plus the unlocked rows still eligible for a late-lock re-send.
struct StreamLocks {
  known: KnownLocks,
  announced: LruCache<[u8; 32], ()>,
}

impl Default for StreamLocks {
  fn default() -> Self {
    Self {
      known: KnownLocks::default(),
      announced: LruCache::new(
        NonZeroUsize::new(MAX_ANNOUNCED_UNLOCKED).unwrap_or(NonZeroUsize::MIN),
      ),
    }
  }
}

impl StreamLocks {
  /// Note a published row; returns whether its lock is known.
  fn published(&mut self, txid: [u8; 32]) -> bool {
    let locked = self.known.contains(&txid);
    if !locked {
      self.announced.put(txid, ());
    }
    locked
  }

  /// Record a lock; returns whether a published unlocked row should be re-sent.
  fn locked(&mut self, txid: [u8; 32]) -> bool {
    self.known.insert(txid);
    self.announced.pop(&txid).is_some()
  }

  fn removed(&mut self, txid: &[u8; 32]) {
    self.announced.pop(txid);
  }

  fn forget_announced(&mut self) {
    self.announced.clear();
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
  use crate::sources::zmq::test_wire;
  use crate::sources::ZmqEvent;
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
  async fn fanout_pairs_raw_tx_with_the_wire_sequence_add() {
    let (tx, rx) = broadcast::channel(8);
    let hub = Arc::new(StreamHub::new(8));
    let lifecycle = Lifecycle::start();
    let mut out = hub.subscribe();

    let task = tokio::spawn(super::fanout_loop(rx, hub, true, lifecycle.clone()));
    tx.send(ZmqEvent::RawTx {
      tx: test_wire::raw_tx(),
      sequence: 7,
    })
    .expect("raw tx should send");
    assert!(
      tokio::time::timeout(std::time::Duration::from_millis(50), out.recv())
        .await
        .is_err()
    );

    tx.send(test_wire::sequence(
      test_wire::RAW_TX_DISPLAY_TXID,
      b'A',
      Some(9),
      8,
    ))
    .expect("sequence should send");
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), out.recv())
      .await
      .expect("mempool tx should publish")
      .expect("hub should stay open");
    let json = serde_json::to_value(event).expect("event should serialize");
    assert_eq!(json["type"], "mempool_tx");
    assert_eq!(json["payload"]["txid"], test_wire::RAW_TX_DISPLAY_TXID);
    assert_eq!(json["payload"]["sequence"], 7);
    drop(tx);
    lifecycle.stop();
    task.await.expect("fanout loop should stop");
  }

  #[tokio::test]
  async fn fanout_pairs_a_wire_sequence_add_that_arrives_first() {
    let (tx, rx) = broadcast::channel(8);
    let hub = Arc::new(StreamHub::new(8));
    let lifecycle = Lifecycle::start();
    let mut out = hub.subscribe();
    let task = tokio::spawn(super::fanout_loop(rx, hub, true, lifecycle.clone()));
    tx.send(test_wire::sequence(
      test_wire::RAW_TX_DISPLAY_TXID,
      b'A',
      Some(9),
      8,
    ))
    .expect("sequence should send");
    tx.send(ZmqEvent::RawTx {
      tx: test_wire::raw_tx(),
      sequence: 7,
    })
    .expect("raw tx should send");
    let event = tokio::time::timeout(std::time::Duration::from_secs(2), out.recv())
      .await
      .expect("mempool tx should publish")
      .expect("hub should stay open");
    let json = serde_json::to_value(event).expect("event should serialize");
    assert_eq!(json["payload"]["txid"], test_wire::RAW_TX_DISPLAY_TXID);
    drop(tx);
    lifecycle.stop();
    task.await.expect("fanout loop should stop");
  }

  async fn queued_events(events: Vec<ZmqEvent>) -> Vec<LiveEvent> {
    queued_events_with_islock(events, true).await
  }

  async fn queued_events_with_islock(
    events: Vec<ZmqEvent>,
    islock_enabled: bool,
  ) -> Vec<LiveEvent> {
    let (tx, rx) = broadcast::channel(64);
    let hub = Arc::new(StreamHub::new(64));
    let mut out = hub.subscribe();
    for event in events {
      tx.send(event).expect("event should queue");
    }
    drop(tx);
    tokio::time::timeout(
      std::time::Duration::from_secs(2),
      super::fanout_loop(rx, hub, islock_enabled, Lifecycle::start()),
    )
    .await
    .expect("fanout should drain the queue");
    let mut published = Vec::new();
    while let Ok(event) = out.try_recv() {
      published.push(event);
    }
    published
  }

  /// Wire-ordered `sequence` topic events with consecutive topic numbers.
  struct Wire(u32);

  impl Wire {
    fn next(&mut self, hash: &str, label: u8, mempool_sequence: Option<u64>) -> ZmqEvent {
      self.0 += 1;
      test_wire::sequence(hash, label, mempool_sequence, self.0)
    }

    fn admitted(&mut self, raw_first: bool) -> Vec<ZmqEvent> {
      let raw = ZmqEvent::RawTx {
        tx: test_wire::raw_tx(),
        sequence: 7,
      };
      let add = self.next(test_wire::RAW_TX_DISPLAY_TXID, b'A', Some(1));
      if raw_first {
        vec![raw, add]
      } else {
        vec![add, raw]
      }
    }

    fn removed(&mut self) -> ZmqEvent {
      self.next(test_wire::RAW_TX_DISPLAY_TXID, b'R', Some(2))
    }

    fn block(&mut self, label: u8) -> ZmqEvent {
      self.next(&"00".repeat(32), label, None)
    }
  }

  fn lock() -> ZmqEvent {
    ZmqEvent::RawTxLock {
      tx: test_wire::raw_tx(),
      sequence: 40,
    }
  }

  /// `live.tags.islock` of each published row; `None` when the flag is absent.
  fn lock_flags(events: Vec<LiveEvent>) -> Vec<Option<bool>> {
    events
      .into_iter()
      .map(|event| {
        let value = serde_json::to_value(event).expect("event should serialize");
        assert_eq!(value["type"], "mempool_tx");
        assert_eq!(value["payload"]["txid"], test_wire::RAW_TX_DISPLAY_TXID);
        let live = &value["payload"]["live"];
        assert!(live.is_object(), "every row carries a live object");
        let flag = live.get("tags").map(|tags| tags["islock"].clone());
        assert_ne!(flag, Some(serde_json::json!(false)), "never false");
        flag.and_then(|flag| flag.as_bool())
      })
      .collect()
  }

  #[tokio::test]
  async fn a_lock_seen_before_the_transaction_marks_its_row() {
    for raw_first in [false, true] {
      let mut wire = Wire(0);
      let mut events = vec![lock()];
      events.extend(wire.admitted(raw_first));
      assert_eq!(lock_flags(queued_events(events).await), vec![Some(true)]);
    }
  }

  #[tokio::test]
  async fn a_lock_seen_after_the_row_resends_it_once_with_the_lock_sequence() {
    for raw_first in [false, true] {
      let mut wire = Wire(0);
      let mut events = wire.admitted(raw_first);
      events.extend([lock(), lock()]);
      let published = queued_events(events).await;
      let resent = serde_json::to_value(&published[1]).expect("event should serialize");
      assert_eq!(resent["payload"]["sequence"], 40);
      assert_eq!(lock_flags(published), vec![None, Some(true)]);
    }
  }

  #[tokio::test]
  async fn a_transaction_whose_lock_was_never_seen_omits_the_flag() {
    let mut wire = Wire(0);
    let published = queued_events(wire.admitted(true)).await;
    let value = serde_json::to_value(&published[0]).expect("event should serialize");
    assert_eq!(value["payload"]["live"], serde_json::json!({}));
    assert_eq!(lock_flags(published), vec![None]);
  }

  #[tokio::test]
  async fn a_locked_transaction_stays_locked_after_mining_a_reorg_and_readmission() {
    for raw_first in [false, true] {
      let mut wire = Wire(0);
      let mut events = vec![lock()];
      events.extend(wire.admitted(raw_first));
      // Mined, then reorged back into the mempool; the node does not resend the lock.
      events.extend([wire.block(b'C'), wire.block(b'D')]);
      events.extend(wire.admitted(raw_first));
      // Evicted, then relayed again.
      events.push(wire.removed());
      events.extend(wire.admitted(raw_first));
      assert_eq!(
        lock_flags(queued_events(events).await),
        vec![Some(true), Some(true), Some(true)]
      );
    }
  }

  #[tokio::test]
  async fn rows_published_before_a_block_or_removal_are_not_resent() {
    let mut wire = Wire(0);
    let mut events = wire.admitted(true);
    events.extend([wire.block(b'C'), lock()]);
    assert_eq!(lock_flags(queued_events(events).await), vec![None]);

    let mut wire = Wire(0);
    let mut events = wire.admitted(true);
    events.extend([wire.removed(), lock()]);
    events.extend(wire.admitted(true));
    assert_eq!(
      lock_flags(queued_events(events).await),
      vec![None, Some(true)],
      "the lock is still remembered for re-admission"
    );
  }

  #[tokio::test]
  async fn a_sequence_gap_keeps_known_locks() {
    let mut wire = Wire(0);
    let mut events = vec![lock()];
    events.extend(wire.admitted(true));
    wire.0 += 5;
    events.push(wire.removed());
    events.extend(wire.admitted(true));
    assert_eq!(
      lock_flags(queued_events(events).await),
      vec![Some(true), Some(true)]
    );
  }

  #[tokio::test]
  async fn a_sequence_gap_ends_late_lock_resends() {
    let mut wire = Wire(0);
    let mut events = wire.admitted(true);
    wire.0 += 5;
    events.extend([wire.block(b'D'), lock()]);
    assert_eq!(lock_flags(queued_events(events).await), vec![None]);
  }

  #[tokio::test]
  async fn locks_are_ignored_when_the_islock_capability_is_off() {
    let mut wire = Wire(0);
    let mut events = vec![lock()];
    events.extend(wire.admitted(true));
    events.push(lock());
    assert_eq!(
      lock_flags(queued_events_with_islock(events, false).await),
      vec![None]
    );
  }

  #[tokio::test]
  async fn a_lagged_hub_ends_late_lock_resends_but_keeps_known_locks() {
    async fn next_row(out: &mut broadcast::Receiver<LiveEvent>) -> LiveEvent {
      tokio::time::timeout(std::time::Duration::from_secs(2), out.recv())
        .await
        .expect("row should publish")
        .expect("hub should stay open")
    }

    let (tx, rx) = broadcast::channel(4);
    let hub = Arc::new(StreamHub::new(16));
    let mut out = hub.subscribe();
    let lifecycle = Lifecycle::start();
    let task = tokio::spawn(super::fanout_loop(rx, hub, true, lifecycle.clone()));

    let mut wire = Wire(0);
    for event in wire.admitted(true) {
      tx.send(event).expect("event should queue");
    }
    assert_eq!(lock_flags(vec![next_row(&mut out).await]), vec![None]);

    // Six sends into a four-slot channel with no yield in between: the hub
    // lags by two, then reads a filler, the lock and a second transaction.
    let mut second = test_wire::raw_tx();
    second[0] = 2;
    let second_txid = crate::consensus::txid(&second).expect("transaction txid");
    let second_display = crate::utils::view_hex(&second_txid, crate::utils::HashEndian::Ser);
    for sequence in 0..3 {
      tx.send(ZmqEvent::RawChainLock {
        chainlock: vec![sequence],
        sequence: u32::from(sequence),
      })
      .expect("filler should queue");
    }
    tx.send(lock()).expect("lock should queue");
    tx.send(ZmqEvent::RawTx {
      tx: second,
      sequence: 8,
    })
    .expect("raw tx should queue");
    tx.send(wire.next(&second_display, b'A', Some(2)))
      .expect("sequence add should queue");

    let next = serde_json::to_value(next_row(&mut out).await).expect("event should serialize");
    assert_eq!(
      next["payload"]["txid"], second_display,
      "the row sent before the lag must not be re-sent"
    );

    // The lock read after the lag is kept: re-admission reports it.
    tx.send(wire.removed()).expect("removal should queue");
    for event in wire.admitted(true) {
      tx.send(event).expect("event should queue");
    }
    assert_eq!(lock_flags(vec![next_row(&mut out).await]), vec![Some(true)]);
    lifecycle.stop();
    task.await.expect("fanout loop should stop");
  }
}
