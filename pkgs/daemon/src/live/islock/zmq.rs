//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live InstantSend tx-lock tracking backed by node ZMQ.

use crate::sql::SqlPool;
use tokio::sync::broadcast;

use dash_primitives::hash;

use crate::error::DaemonError;
use crate::lifecycle::Lifecycle;
use crate::sources::{SequenceEvent, ZmqEvent};
use crate::sql;
use crate::utils::{self, HashEndian};

/// Persist current InstantSend tx-lock state from ZMQ until shutdown.
pub async fn sync_current_loop(
  pool: SqlPool,
  mut rx: broadcast::Receiver<ZmqEvent>,
  lifecycle: std::sync::Arc<Lifecycle>,
) {
  let mut shutdown = lifecycle.subscribe();
  while lifecycle.running() {
    match tokio::select! {
      result = rx.recv() => Some(result),
      _ = shutdown.changed() => None,
    } {
      Some(Ok(event)) => {
        if let Err(err) = handle_event(&pool, event).await {
          tracing::warn!("islock current sync failed: {err}");
        }
      }
      Some(Err(broadcast::error::RecvError::Lagged(skipped))) => {
        tracing::warn!(skipped, "islock ZMQ consumer lagged");
      }
      Some(Err(broadcast::error::RecvError::Closed)) | None => break,
    }
  }
}

async fn handle_event(pool: &SqlPool, event: ZmqEvent) -> Result<(), DaemonError> {
  match event {
    ZmqEvent::RawTxLock { tx, .. } => upsert_current(pool, tx).await,
    ZmqEvent::Sequence {
      event: SequenceEvent::TxRemoved { txid, .. },
      ..
    } => delete_current(pool, txid).await,
    _ => Ok(()),
  }
}

async fn upsert_current(pool: &SqlPool, raw_islock: Vec<u8>) -> Result<(), DaemonError> {
  // Dash `rawtxlock` publishes the locked transaction bytes, so hashing the
  // payload yields the same txid used by `sequence` removals and `getislocks`.
  let row = sql::live::islock::IslockCurrentRow {
    txid: utils::ser_endian(&hash::tx_hash(&raw_islock).0, HashEndian::Ser),
    raw_islock,
    seen_at: now_epoch_secs(),
  };
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin islock current tx: {e}")))?;
  sql::live::islock::upsert_current(&mut tx, &row).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit islock current tx: {e}")))?;
  Ok(())
}

async fn delete_current(pool: &SqlPool, txid: [u8; 32]) -> Result<(), DaemonError> {
  let txid = utils::ser_endian(&txid, HashEndian::Ser);
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin islock delete tx: {e}")))?;
  sql::live::islock::delete_current(&mut tx, &txid).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit islock delete tx: {e}")))?;
  Ok(())
}

fn now_epoch_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs() as i64
}
