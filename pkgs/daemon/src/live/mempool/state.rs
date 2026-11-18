//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live mempool state synchronization from ZMQ events.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use hyphen_db::HyphenDb;
use tokio::sync::broadcast;

use crate::error::DaemonError;
use crate::lifecycle::Lifecycle;
use crate::live::state::LiveState;
use crate::sources::{SequenceEvent, ZmqEvent};

/// The incremental set may grow to this many times `keep` between RPC
/// snapshots, which re-bound it to `keep`; past that the oldest rows go.
const RECENT_SET_CAP_MULTIPLE: usize = 4;

struct RecentSyncCtx<'a> {
  // Retained only for `normalize_tx`'s transitional prevout resolution.
  db: &'a HyphenDb,
  live_state: &'a LiveState,
  ingest_status: &'a crate::ingest::IngestStatus,
  cap: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
}

/// What the incremental mempool loop needs.
pub(crate) struct RecentSync {
  pub db: HyphenDb,
  pub live_state: Arc<LiveState>,
  pub rx: broadcast::Receiver<ZmqEvent>,
  pub lifecycle: Arc<Lifecycle>,
  pub ingest_status: Arc<crate::ingest::IngestStatus>,
  pub keep: usize,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
}

/// Track recent mempool transaction changes in RAM from ZMQ until shutdown.
pub(crate) async fn sync_recent_loop(params: RecentSync) {
  let RecentSync {
    db,
    live_state,
    mut rx,
    lifecycle,
    ingest_status,
    keep,
    p2pkh_version,
    p2sh_version,
  } = params;
  let cache_cap = keep.saturating_mul(4).max(32);
  let mut pending_raw = PendingRawTxs::new(cache_cap);
  let mut pending_adds = PendingTxAdds::new(cache_cap);
  let mut shutdown = lifecycle.subscribe();
  let ctx = RecentSyncCtx {
    db: &db,
    live_state: live_state.as_ref(),
    ingest_status: ingest_status.as_ref(),
    cap: keep.saturating_mul(RECENT_SET_CAP_MULTIPLE).max(1),
    p2pkh_version,
    p2sh_version,
  };
  while lifecycle.running() {
    match tokio::select! {
      result = rx.recv() => Some(result),
      _ = shutdown.changed() => None,
    } {
      Some(Ok(event)) => {
        if let Err(err) = handle_event(&ctx, event, &mut pending_raw, &mut pending_adds).await {
          tracing::warn!("mempool recent sync failed: {err}");
        }
      }
      Some(Err(broadcast::error::RecvError::Lagged(skipped))) => {
        tracing::warn!(skipped, "mempool ZMQ consumer lagged");
      }
      Some(Err(broadcast::error::RecvError::Closed)) | None => break,
    }
  }
}

async fn handle_event(
  ctx: &RecentSyncCtx<'_>,
  event: ZmqEvent,
  pending_raw: &mut PendingRawTxs,
  pending_adds: &mut PendingTxAdds,
) -> Result<(), DaemonError> {
  match event {
    // Same gate as the RPC snapshot loop: until the projection reaches the
    // node, hydrating a mempool transaction scans the partition stepped sync
    // left unindexed, competes with ingest, and cannot resolve recent prevouts.
    // Removals below still apply.
    ZmqEvent::RawTx { .. }
    | ZmqEvent::Sequence {
      event: SequenceEvent::TxAdded { .. },
      ..
    } if !ctx.ingest_status.projection_near_tip() => Ok(()),
    ZmqEvent::RawTx { tx, .. } => handle_raw_tx(ctx, tx, pending_raw, pending_adds).await,
    ZmqEvent::Sequence {
      event: SequenceEvent::TxAdded { txid, .. },
      ..
    } => handle_tx_added(ctx, txid, pending_raw, pending_adds).await,
    ZmqEvent::Sequence {
      event: SequenceEvent::TxRemoved { txid, .. },
      ..
    } => {
      pending_raw.remove(&txid);
      pending_adds.remove(&txid);
      ctx.live_state.remove_mempool_tx(&txid);
      Ok(())
    }
    _ => Ok(()),
  }
}

async fn handle_raw_tx(
  ctx: &RecentSyncCtx<'_>,
  raw_tx: Vec<u8>,
  pending_raw: &mut PendingRawTxs,
  pending_adds: &mut PendingTxAdds,
) -> Result<(), DaemonError> {
  let txid = crate::consensus::txid(&raw_tx)?;
  if pending_adds.remove(&txid) {
    upsert_recent_tx(ctx, txid, raw_tx).await
  } else {
    pending_raw.insert(txid, raw_tx);
    Ok(())
  }
}

async fn handle_tx_added(
  ctx: &RecentSyncCtx<'_>,
  txid: [u8; 32],
  pending_raw: &mut PendingRawTxs,
  pending_adds: &mut PendingTxAdds,
) -> Result<(), DaemonError> {
  if let Some(raw_tx) = pending_raw.remove(&txid) {
    upsert_recent_tx(ctx, txid, raw_tx).await
  } else {
    pending_adds.insert(txid);
    Ok(())
  }
}

async fn upsert_recent_tx(
  ctx: &RecentSyncCtx<'_>,
  txid: [u8; 32],
  raw_tx: Vec<u8>,
) -> Result<(), DaemonError> {
  let now = now_epoch_secs();
  let txid = txid.to_vec();
  // The 15s RPC snapshot re-bounds the set to `keep`. Between snapshots, or
  // when they keep failing, mined transactions get no removal event, so the
  // incremental path keeps only the newest `cap` rows.
  let tx = crate::live::mempool::query::normalize_tx(
    ctx.db,
    &txid,
    &raw_tx,
    ctx.p2pkh_version,
    ctx.p2sh_version,
    now,
    now,
  )
  .await?;
  ctx.live_state.insert_mempool_tx_capped(tx, ctx.cap);
  Ok(())
}

fn now_epoch_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs() as i64
}

struct PendingRawTxs {
  cap: usize,
  order: VecDeque<[u8; 32]>,
  by_txid: HashMap<[u8; 32], Vec<u8>>,
}

impl PendingRawTxs {
  fn new(cap: usize) -> Self {
    Self {
      cap,
      order: VecDeque::new(),
      by_txid: HashMap::new(),
    }
  }

  fn insert(&mut self, txid: [u8; 32], raw_tx: Vec<u8>) {
    if !self.by_txid.contains_key(&txid) {
      self.order.push_back(txid);
    }
    self.by_txid.insert(txid, raw_tx);
    self.prune();
  }

  fn remove(&mut self, txid: &[u8; 32]) -> Option<Vec<u8>> {
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
  use super::{PendingRawTxs, PendingTxAdds};
  use crate::sources::{SequenceEvent, ZmqEvent};

  #[test]
  fn non_mempool_zmq_event_shape_stays_distinct() {
    assert!(matches!(
      ZmqEvent::Sequence {
        event: SequenceEvent::BlockConnected { hash: [0u8; 32] },
        sequence: 1,
      },
      ZmqEvent::Sequence {
        event: SequenceEvent::BlockConnected { .. },
        ..
      }
    ));
  }

  #[test]
  fn pending_tx_match_requires_sequence_add_signal() {
    let txid = [0x11; 32];
    let raw_tx = vec![0xaa, 0xbb];
    let mut pending_raw = PendingRawTxs::new(4);
    let mut pending_adds = PendingTxAdds::new(4);

    pending_raw.insert(txid, raw_tx.clone());
    assert!(pending_raw.remove(&txid).is_some());

    pending_raw.insert(txid, raw_tx);
    pending_adds.insert(txid);
    assert!(pending_adds.remove(&txid));
    assert!(pending_raw.remove(&txid).is_some());
  }

  /// A decodable transaction that differs from the others by its lock time.
  fn raw_tx(lock_time: u32) -> Vec<u8> {
    let mut raw = 1i32.to_le_bytes().to_vec();
    raw.push(1);
    raw.extend_from_slice(&[0; 32]);
    raw.extend_from_slice(&u32::MAX.to_le_bytes());
    raw.push(0);
    raw.extend_from_slice(&u32::MAX.to_le_bytes());
    raw.push(1);
    raw.extend_from_slice(&42i64.to_le_bytes());
    raw.push(0);
    raw.extend_from_slice(&lock_time.to_le_bytes());
    raw
  }

  async fn admit(
    ctx: &super::RecentSyncCtx<'_>,
    raw: Vec<u8>,
    pending_raw: &mut PendingRawTxs,
    pending_adds: &mut PendingTxAdds,
  ) -> [u8; 32] {
    let txid = crate::consensus::txid(&raw).expect("transaction txid");
    for event in [
      ZmqEvent::RawTx {
        tx: raw,
        sequence: 1,
      },
      ZmqEvent::Sequence {
        event: SequenceEvent::TxAdded {
          txid,
          mempool_sequence: 1,
        },
        sequence: 1,
      },
    ] {
      super::handle_event(ctx, event, pending_raw, pending_adds)
        .await
        .expect("event should apply");
    }
    txid
  }

  #[tokio::test]
  async fn nothing_is_hydrated_or_kept_until_the_projection_nears_the_tip() {
    // nosemgrep: contrib.r2c.daemon-live-state-must-not-depend-on-rest -- test fixture
    let db = crate::rest::hydb_fake::MemDb::new().into_hydb();
    let live_state = crate::live::state::LiveState::new();
    let ingest_status = crate::ingest::IngestStatus::new();
    ingest_status.set_tips_for_test(1_000, 2_000);
    let ctx = super::RecentSyncCtx {
      db: &db,
      live_state: &live_state,
      ingest_status: &ingest_status,
      cap: 8,
      p2pkh_version: 76,
      p2sh_version: 16,
    };
    let mut pending_raw = PendingRawTxs::new(4);
    let mut pending_adds = PendingTxAdds::new(4);

    let txid = admit(&ctx, raw_tx(1), &mut pending_raw, &mut pending_adds).await;
    assert_eq!(live_state.mempool_len(), 0, "no row while syncing");
    assert!(pending_raw.by_txid.is_empty() && pending_adds.seen.is_empty());

    // Removals still apply while syncing.
    let held = crate::live::mempool::query::normalize_tx(&db, &txid, &raw_tx(1), 76, 16, 1, 1)
      .await
      .expect("transaction should normalize");
    live_state.insert_mempool_tx(held);
    super::handle_event(
      &ctx,
      ZmqEvent::Sequence {
        event: SequenceEvent::TxRemoved {
          txid,
          mempool_sequence: 2,
        },
        sequence: 2,
      },
      &mut pending_raw,
      &mut pending_adds,
    )
    .await
    .expect("removal should apply");
    assert_eq!(live_state.mempool_len(), 0);

    ingest_status.set_tips_for_test(2_000, 2_000);
    admit(&ctx, raw_tx(1), &mut pending_raw, &mut pending_adds).await;
    assert!(
      live_state.mempool_tx(&txid).is_some(),
      "rows resume at the tip"
    );
  }

  #[tokio::test]
  async fn the_incremental_set_stays_within_its_cap() {
    // nosemgrep: contrib.r2c.daemon-live-state-must-not-depend-on-rest -- test fixture
    let db = crate::rest::hydb_fake::MemDb::new().into_hydb();
    let live_state = crate::live::state::LiveState::new();
    let ingest_status = crate::ingest::IngestStatus::new();
    ingest_status.set_tips_for_test(2_000, 2_000);
    let ctx = super::RecentSyncCtx {
      db: &db,
      live_state: &live_state,
      ingest_status: &ingest_status,
      cap: 4,
      p2pkh_version: 76,
      p2sh_version: 16,
    };
    let mut pending_raw = PendingRawTxs::new(16);
    let mut pending_adds = PendingTxAdds::new(16);
    for lock_time in 0..10 {
      admit(&ctx, raw_tx(lock_time), &mut pending_raw, &mut pending_adds).await;
      assert!(live_state.mempool_len() <= 4);
    }
    assert_eq!(live_state.mempool_len(), 4);
  }

  #[tokio::test]
  async fn raw_tx_and_wire_sequence_events_pair_and_remove_the_same_row() {
    use crate::sources::zmq::test_wire;
    // nosemgrep: contrib.r2c.daemon-live-state-must-not-depend-on-rest -- test fixture
    let db = crate::rest::hydb_fake::MemDb::new().into_hydb();
    let live_state = crate::live::state::LiveState::new();
    let ingest_status = crate::ingest::IngestStatus::new();
    ingest_status.set_tips_for_test(2_000, 2_000);
    let ctx = super::RecentSyncCtx {
      db: &db,
      live_state: &live_state,
      ingest_status: &ingest_status,
      cap: 8,
      p2pkh_version: 76,
      p2sh_version: 16,
    };
    let raw = test_wire::raw_tx();
    let txid = crate::consensus::txid(&raw).expect("transaction txid");
    let mut pending_raw = PendingRawTxs::new(4);
    let mut pending_adds = PendingTxAdds::new(4);
    super::handle_event(
      &ctx,
      ZmqEvent::RawTx {
        tx: raw,
        sequence: 1,
      },
      &mut pending_raw,
      &mut pending_adds,
    )
    .await
    .expect("raw tx should apply");
    super::handle_event(
      &ctx,
      test_wire::sequence(test_wire::RAW_TX_DISPLAY_TXID, b'A', Some(1), 1),
      &mut pending_raw,
      &mut pending_adds,
    )
    .await
    .expect("sequence add should apply");
    assert!(live_state.mempool_tx(&txid).is_some());
    super::handle_event(
      &ctx,
      test_wire::sequence(test_wire::RAW_TX_DISPLAY_TXID, b'R', Some(2), 2),
      &mut pending_raw,
      &mut pending_adds,
    )
    .await
    .expect("sequence removal should apply");
    assert!(live_state.mempool_tx(&txid).is_none());
  }
}
