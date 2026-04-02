//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live mempool state synchronization from ZMQ events.

use std::collections::{HashMap, VecDeque};
use std::sync::Arc;

use crate::sql::SqlPool;
use dash_primitives::hash;
use tokio::sync::broadcast;

use crate::error::DaemonError;
use crate::index::spent::cache::SpentQueryCache;
use crate::lifecycle::Lifecycle;
use crate::sources::{SequenceEvent, ZmqEvent};
use crate::sql;

struct RecentSyncCtx<'a> {
  pool: &'a SqlPool,
  keep: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
  spent_cache: &'a SpentQueryCache,
}

/// Persist recent mempool transaction changes from ZMQ until shutdown.
pub async fn sync_recent_loop(
  pool: SqlPool,
  mut rx: broadcast::Receiver<ZmqEvent>,
  lifecycle: Arc<Lifecycle>,
  keep: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
  spent_cache: Arc<SpentQueryCache>,
) {
  let cache_cap = keep.saturating_mul(4).max(32);
  let mut pending_raw = PendingRawTxs::new(cache_cap);
  let mut pending_adds = PendingTxAdds::new(cache_cap);
  let mut shutdown = lifecycle.subscribe();
  let ctx = RecentSyncCtx {
    pool: &pool,
    keep,
    p2pkh_version,
    p2sh_version,
    spent_cache: spent_cache.as_ref(),
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
      let deleted = delete_recent_tx(ctx.pool, txid).await;
      if deleted.is_ok() {
        ctx.spent_cache.bump_live_generation();
      }
      deleted
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
  let txid = hash::tx_hash(&raw_tx).0;
  if pending_adds.remove(&txid) {
    let inserted = upsert_recent_tx(
      ctx.pool,
      raw_tx,
      ctx.keep,
      ctx.p2pkh_version,
      ctx.p2sh_version,
    )
    .await;
    if inserted.is_ok() {
      ctx.spent_cache.bump_live_generation();
    }
    inserted
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
    let inserted = upsert_recent_tx(
      ctx.pool,
      raw_tx,
      ctx.keep,
      ctx.p2pkh_version,
      ctx.p2sh_version,
    )
    .await;
    if inserted.is_ok() {
      ctx.spent_cache.bump_live_generation();
    }
    inserted
  } else {
    pending_adds.insert(txid);
    Ok(())
  }
}

async fn upsert_recent_tx(
  pool: &SqlPool,
  raw_tx: Vec<u8>,
  keep: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Result<(), DaemonError> {
  let now = now_epoch_secs();
  let txid = hash::tx_hash(&raw_tx).0.to_vec();
  let row = sql::live::mempool::MempoolRecentTxRow {
    txid: txid.clone(),
    raw_tx,
    first_seen_at: now,
    last_seen_at: now,
  };
  let normalized = crate::live::mempool::query::normalize_tx(
    pool,
    &txid,
    &row.raw_tx,
    p2pkh_version,
    p2sh_version,
    row.first_seen_at,
    row.last_seen_at,
  )
  .await?;
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin mempool recent tx insert: {e}")))?;
  sql::live::mempool::upsert_recent_tx(&mut tx, &row).await?;
  sql::live::mempool::upsert_tx_detail(
    &mut tx,
    &normalized.detail,
    &normalized.inputs,
    &normalized.outputs,
    &normalized.address_rows,
  )
  .await?;
  sql::live::mempool::prune_recent_txs(&mut tx, keep as i64).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit mempool recent tx insert: {e}")))?;
  Ok(())
}

async fn delete_recent_tx(pool: &SqlPool, txid: [u8; 32]) -> Result<(), DaemonError> {
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin mempool recent tx delete: {e}")))?;
  sql::live::mempool::delete_recent_tx(&mut tx, &txid).await?;
  sql::live::mempool::delete_tx_detail(&mut tx, &txid).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit mempool recent tx delete: {e}")))?;
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
}
