//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! JSON-RPC-backed mempool summary refresh ownership.

use std::sync::Arc;

use crate::error::DaemonError;
use crate::lifecycle::Lifecycle;
use crate::live::warn;
use crate::policy::api::API_REFRESH_WARN_INTERVAL_SECS;
use crate::sources::RpcSource;
use crate::sql;
use crate::utils::{self, HashEndian};

const SUMMARY_REFRESH_SECS: u64 = 15;

async fn build_snapshot_tx(
  pool: &crate::sql::SqlPool,
  txid_hex: &str,
  raw_tx: Vec<u8>,
  seen_at: i64,
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Result<
  (
    sql::live::mempool::MempoolRecentTxRow,
    crate::live::mempool::query::NormalizedMempoolTx,
  ),
  DaemonError,
> {
  let txid = utils::decode_hex_to(txid_hex, HashEndian::Ser)
    .map_err(|e| DaemonError::RestSource(format!("mempool snapshot txid decode: {e}")))?;
  let row = sql::live::mempool::MempoolRecentTxRow {
    txid: txid.clone(),
    raw_tx,
    first_seen_at: seen_at,
    last_seen_at: seen_at,
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
  Ok((row, normalized))
}

async fn refresh_recent_snapshot(
  pool: &crate::sql::SqlPool,
  rpc: &dyn RpcSource,
  keep: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Result<(), DaemonError> {
  let mut entries = rpc.get_raw_mempool().await?;
  entries.sort_by(|left, right| {
    right
      .1
      .time
      .cmp(&left.1.time)
      .then_with(|| left.0.cmp(&right.0))
  });
  let mut snapshot_rows = Vec::new();
  for (txid, entry) in entries.into_iter().take(keep) {
    match rpc.get_raw_transaction(&txid).await {
      Ok(raw_tx) => {
        match build_snapshot_tx(pool, &txid, raw_tx, entry.time, p2pkh_version, p2sh_version).await
        {
          Ok(snapshot) => snapshot_rows.push(snapshot),
          Err(err) => tracing::warn!(txid, "mempool snapshot tx hydrate failed: {err}"),
        }
      }
      Err(err) => tracing::warn!(txid, "mempool snapshot raw tx fetch failed: {err}"),
    }
  }

  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin mempool snapshot refresh tx: {e}")))?;
  sql::live::mempool::clear_recent_snapshot(&mut tx).await?;
  for (row, normalized) in snapshot_rows {
    sql::live::mempool::upsert_recent_tx(&mut tx, &row).await?;
    sql::live::mempool::upsert_tx_detail(
      &mut tx,
      &normalized.detail,
      &normalized.inputs,
      &normalized.outputs,
      &normalized.address_rows,
    )
    .await?;
  }
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit mempool snapshot refresh tx: {e}")))?;
  Ok(())
}

async fn refresh_summary_once(
  pool: &crate::sql::SqlPool,
  rpc: &dyn RpcSource,
  keep: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Result<(), DaemonError> {
  let info = rpc.get_mempool_info().await?;
  let row = sql::live::mempool::MempoolSummaryRow {
    tx_count: i64::try_from(info.size)
      .map_err(|_| DaemonError::RestSource("mempool size exceeds i64::MAX".to_owned()))?,
    total_bytes: i64::try_from(info.bytes)
      .map_err(|_| DaemonError::RestSource("mempool bytes exceeds i64::MAX".to_owned()))?,
    usage_bytes: Some(
      i64::try_from(info.usage)
        .map_err(|_| DaemonError::RestSource("mempool usage exceeds i64::MAX".to_owned()))?,
    ),
    max_bytes: Some(
      i64::try_from(info.maxmempool)
        .map_err(|_| DaemonError::RestSource("mempool maxmempool exceeds i64::MAX".to_owned()))?,
    ),
    updated_at: std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs() as i64,
  };
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin mempool summary tx: {e}")))?;
  sql::live::mempool::upsert_summary(&mut tx, &row).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit mempool summary tx: {e}")))?;

  if row.tx_count == 0 {
    let mut tx = pool
      .begin()
      .await
      .map_err(|e| DaemonError::Pg(format!("begin mempool snapshot clear: {e}")))?;
    sql::live::mempool::clear_recent_snapshot(&mut tx).await?;
    tx.commit()
      .await
      .map_err(|e| DaemonError::Pg(format!("commit mempool snapshot clear: {e}")))?;
    return Ok(());
  }

  refresh_recent_snapshot(pool, rpc, keep, p2pkh_version, p2sh_version).await?;

  Ok(())
}

/// Periodically refresh the persisted mempool summary until shutdown.
pub async fn refresh_summary_loop(
  pool: crate::sql::SqlPool,
  rpc: Arc<dyn RpcSource>,
  lifecycle: Arc<Lifecycle>,
  keep: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
) {
  while lifecycle.running() {
    if let Err(e) =
      refresh_summary_once(&pool, rpc.as_ref(), keep, p2pkh_version, p2sh_version).await
    {
      warn::warn_every(
        "live_mempool_summary_refresh_failed",
        std::time::Duration::from_secs(API_REFRESH_WARN_INTERVAL_SECS),
        |suppressed| {
          tracing::warn!(suppressed, "mempool summary refresh failed: {e}");
        },
      );
    }
    lifecycle
      .sleep(std::time::Duration::from_secs(SUMMARY_REFRESH_SECS))
      .await;
  }
}

#[cfg(test)]
mod tests {
  use async_trait::async_trait;

  use super::refresh_summary_once;
  use crate::error::DaemonError;
  use crate::sources::rpc::{
    BestChainLock, MempoolInfo, MempoolTxEntry, MempoolTxMap, NetworkInfo, PeerInfo, ProTxListEntry,
  };
  use crate::sources::RpcSource;

  #[derive(Debug)]
  struct FakeRpc {
    info: MempoolInfo,
    entries: MempoolTxMap,
    raw: std::collections::HashMap<String, Vec<u8>>,
  }

  #[async_trait]
  impl RpcSource for FakeRpc {
    async fn get_mempool_info(&self) -> Result<MempoolInfo, DaemonError> {
      Ok(self.info.clone())
    }

    async fn get_raw_mempool(&self) -> Result<MempoolTxMap, DaemonError> {
      Ok(self.entries.clone())
    }

    async fn get_raw_transaction(&self, txid: &str) -> Result<Vec<u8>, DaemonError> {
      self
        .raw
        .get(txid)
        .cloned()
        .ok_or_else(|| DaemonError::RestSource(format!("missing test tx {txid}")))
    }

    async fn get_network_info(&self) -> Result<NetworkInfo, DaemonError> {
      unreachable!("unused in mempool rpc tests")
    }

    async fn get_peer_info(&self) -> Result<PeerInfo, DaemonError> {
      unreachable!("unused in mempool rpc tests")
    }

    async fn get_best_chainlock(&self) -> Result<BestChainLock, DaemonError> {
      unreachable!("unused in mempool rpc tests")
    }

    async fn get_protx_list_registered(&self) -> Result<Vec<ProTxListEntry>, DaemonError> {
      unreachable!("unused in mempool rpc tests")
    }

    async fn get_protx_list_valid(&self) -> Result<Vec<ProTxListEntry>, DaemonError> {
      unreachable!("unused in mempool rpc tests")
    }

    fn diag(&self) -> hyphend::wire::msg::RpcDiag {
      hyphend::wire::msg::RpcDiag::default()
    }
  }

  fn p2pkh_tx_bytes() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1i32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&[0x00; 32]);
    buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&42i64.to_le_bytes());
    buf.push(0x19);
    buf.extend_from_slice(&[
      0x76, 0xa9, 0x14, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
      0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x88, 0xac,
    ]);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
  }

  fn provider_register_payload_bytes() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&[0x11; 32]);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(&[0x22; 16]);
    buf.extend_from_slice(&9999u16.to_be_bytes());
    buf.extend_from_slice(&[0x33; 20]);
    buf.extend_from_slice(&[0x44; 48]);
    buf.extend_from_slice(&[0x55; 20]);
    buf.extend_from_slice(&100u16.to_le_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&[0x66; 32]);
    buf.push(0x03);
    buf.extend_from_slice(&[0x77; 3]);
    buf
  }

  fn provider_register_tx_bytes() -> Vec<u8> {
    let payload = provider_register_payload_bytes();
    assert!(payload.len() < 0xfd, "test payload must fit in one-byte CompactSize");

    let mut buf = Vec::new();
    let n_version: i32 = 3 | (1 << 16);
    buf.extend_from_slice(&n_version.to_le_bytes());
    buf.push(0x00);
    buf.push(0x00);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.push(payload.len() as u8);
    buf.extend_from_slice(&payload);
    buf
  }

  #[tokio::test]
  async fn refresh_summary_once_rebuilds_sparse_recent_snapshot_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };
    let pool = crate::sql::connect_test_pool(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
    )
    .await?;
    crate::sql::bootstrap::reset(&pool).await?;

    let stale_txid = vec![0xaa; 32];
    let mut tx = pool.begin().await?;
    crate::sql::live::mempool::upsert_summary(
      &mut tx,
      &crate::sql::live::mempool::MempoolSummaryRow {
        tx_count: 99,
        total_bytes: 123,
        usage_bytes: Some(456),
        max_bytes: Some(789),
        updated_at: 10,
      },
    )
    .await?;
    crate::sql::live::mempool::upsert_recent_tx(
      &mut tx,
      &crate::sql::live::mempool::MempoolRecentTxRow {
        txid: stale_txid.clone(),
        raw_tx: p2pkh_tx_bytes(),
        first_seen_at: 1,
        last_seen_at: 1,
      },
    )
    .await?;
    tx.commit().await?;

    let txid_a = "11".repeat(32);
    let txid_b = "22".repeat(32);
    let rpc = FakeRpc {
      info: MempoolInfo {
        size: 2,
        bytes: 170,
        usage: 170,
        maxmempool: 300_000_000,
      },
      entries: vec![
        (txid_a.clone(), MempoolTxEntry { time: 20 }),
        (txid_b.clone(), MempoolTxEntry { time: 19 }),
      ],
      raw: std::collections::HashMap::from([
        (txid_a, p2pkh_tx_bytes()),
        (txid_b, p2pkh_tx_bytes()),
      ]),
    };

    refresh_summary_once(&pool, &rpc, 25, 76, 16).await?;

    let summary = crate::sql::live::mempool::summary(&pool)
      .await?
      .expect("summary should persist");
    let recent = crate::sql::live::mempool::recent_txs(&pool, 25).await?;
    assert_eq!(summary.tx_count, 2);
    assert_eq!(recent.len(), 2);
    assert!(!recent.iter().any(|row| row.txid == stale_txid));
    Ok(())
  }

  #[tokio::test]
  async fn refresh_summary_once_persists_special_transactions_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };
    let pool = crate::sql::connect_test_pool(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
    )
    .await?;
    crate::sql::bootstrap::reset(&pool).await?;

    let txid = "11".repeat(32);
    let rpc = FakeRpc {
      info: MempoolInfo {
        size: 1,
        bytes: 64,
        usage: 64,
        maxmempool: 300_000_000,
      },
      entries: vec![(txid.clone(), MempoolTxEntry { time: 20 })],
      raw: std::collections::HashMap::from([(txid, provider_register_tx_bytes())]),
    };

    refresh_summary_once(&pool, &rpc, 25, 76, 16).await?;

    let recent = crate::sql::live::mempool::recent_txs(&pool, 25).await?;
    let detail = crate::sql::live::mempool::tx_detail(&pool, &[0x11; 32])
      .await?
      .expect("special tx detail should persist");
    assert_eq!(recent.len(), 1);
    assert_eq!(detail.tx_type, "provider_register");
    assert_eq!(detail.value_out, 0);
    assert!(detail.payload.is_some());
    Ok(())
  }
}
