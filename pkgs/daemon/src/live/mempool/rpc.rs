//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! JSON-RPC-backed mempool summary refresh ownership.

use std::sync::Arc;

use crate::error::{DaemonError, RestErrorKind};
use crate::lifecycle::Lifecycle;
use crate::live::state::{LiveState, MempoolSummary, MempoolTx};
use crate::live::warn;
use crate::policy::api::API_REFRESH_WARN_INTERVAL_SECS;
use crate::sources::RpcSource;
use crate::utils::{self, HashEndian};
use hyphen_db::HyphenDb;

const SUMMARY_REFRESH_SECS: u64 = 15;

async fn build_snapshot_tx(
  db: &HyphenDb,
  txid_hex: &str,
  raw_tx: Vec<u8>,
  seen_at: i64,
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Result<MempoolTx, DaemonError> {
  let txid =
    utils::decode_hex_to(txid_hex, HashEndian::Ser).map_err(|e| DaemonError::RestSource {
      message: format!("mempool snapshot txid decode: {e}"),
      kind: RestErrorKind::Other,
    })?;
  crate::live::mempool::query::normalize_tx(
    db,
    &txid,
    &raw_tx,
    p2pkh_version,
    p2sh_version,
    seen_at,
    seen_at,
  )
  .await
}

async fn refresh_recent_snapshot(
  db: &HyphenDb,
  live_state: &LiveState,
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
  let mut txs = Vec::new();
  for (txid, entry) in entries.into_iter().take(keep) {
    match rpc.get_raw_transaction(&txid).await {
      Ok(raw_tx) => {
        match build_snapshot_tx(db, &txid, raw_tx, entry.time, p2pkh_version, p2sh_version).await {
          Ok(tx) => txs.push(tx),
          Err(err) => tracing::warn!(txid, "mempool snapshot tx hydrate failed: {err}"),
        }
      }
      Err(err) => tracing::warn!(txid, "mempool snapshot raw tx fetch failed: {err}"),
    }
  }
  live_state.replace_mempool(txs);
  Ok(())
}

async fn refresh_summary_once(
  db: &HyphenDb,
  live_state: &LiveState,
  rpc: &dyn RpcSource,
  keep: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Result<(), DaemonError> {
  let info = rpc.get_mempool_info().await?;
  let summary = MempoolSummary {
    tx_count: i64::try_from(info.size).map_err(|_| DaemonError::RestSource {
      message: "mempool size exceeds i64::MAX".to_owned(),
      kind: RestErrorKind::Other,
    })?,
    total_bytes: i64::try_from(info.bytes).map_err(|_| DaemonError::RestSource {
      message: "mempool bytes exceeds i64::MAX".to_owned(),
      kind: RestErrorKind::Other,
    })?,
    usage_bytes: Some(
      i64::try_from(info.usage).map_err(|_| DaemonError::RestSource {
        message: "mempool usage exceeds i64::MAX".to_owned(),
        kind: RestErrorKind::Other,
      })?,
    ),
    max_bytes: Some(
      i64::try_from(info.maxmempool).map_err(|_| DaemonError::RestSource {
        message: "mempool maxmempool exceeds i64::MAX".to_owned(),
        kind: RestErrorKind::Other,
      })?,
    ),
    updated_at: std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs() as i64,
  };
  let tx_count = summary.tx_count;
  live_state.set_mempool_summary(summary);

  if tx_count == 0 {
    live_state.replace_mempool(Vec::new());
    return Ok(());
  }

  refresh_recent_snapshot(db, live_state, rpc, keep, p2pkh_version, p2sh_version).await
}

/// What the refresh loop needs. A struct rather than a parameter list because
/// the list had grown past the point where the call site said anything about
/// which `u8` was which.
pub(crate) struct MempoolRefresh {
  pub db: HyphenDb,
  pub live_state: Arc<LiveState>,
  pub rpc: Arc<dyn RpcSource>,
  pub lifecycle: Arc<Lifecycle>,
  pub ingest_status: Arc<crate::ingest::IngestStatus>,
  pub keep: usize,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
}

/// Periodically refresh the in-RAM mempool summary + recent set until shutdown.
pub(crate) async fn refresh_summary_loop(params: MempoolRefresh) {
  let MempoolRefresh {
    db,
    live_state,
    rpc,
    lifecycle,
    ingest_status,
    keep,
    p2pkh_version,
    p2sh_version,
  } = params;
  while lifecycle.running() {
    // Nothing to do until the projection reaches the node. A mempool
    // transaction spends outputs that already exist, and resolving one needs
    // those outputs indexed; while the chain behind it is still being built
    // the answer is not stale, it is unavailable. Attempting it anyway is not
    // free — each refresh issues txid lookups that scan the partition stepped
    // sync has deliberately left unindexed, competing for the disk with the
    // ingest that would eventually make the answer possible, and failing
    // regardless. Measured on a mainnet sync: 47 consecutive hydrate
    // failures, every one of them unavoidable.
    if !ingest_status.projection_near_tip() {
      // Nothing is published rather than something wrong: before the first
      // successful refresh the summary is already absent, which is the honest
      // answer while the projection cannot resolve a mempool transaction.
      warn::warn_every(
        "live_mempool_awaiting_tip",
        std::time::Duration::from_secs(API_REFRESH_WARN_INTERVAL_SECS),
        |suppressed| {
          tracing::info!(
            suppressed,
            local_tip = ?ingest_status.local_tip(),
            node_tip = ?ingest_status.network_tip(),
            "mempool idle until the projection reaches the node tip; \
             its transactions cannot be resolved before then"
          );
        },
      );
      lifecycle
        .sleep(std::time::Duration::from_secs(SUMMARY_REFRESH_SECS))
        .await;
      continue;
    }
    if let Err(e) = refresh_summary_once(
      &db,
      &live_state,
      rpc.as_ref(),
      keep,
      p2pkh_version,
      p2sh_version,
    )
    .await
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
  use crate::error::{DaemonError, RestErrorKind};
  use crate::live::state::{LiveState, MempoolSummary, MempoolTx};
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
        .ok_or_else(|| DaemonError::RestSource {
          message: format!("missing test tx {txid}"),
          kind: RestErrorKind::Other,
        })
    }

    async fn get_chain_tx_stats(&self) -> Result<crate::sources::rpc::ChainTxStats, DaemonError> {
      Ok(crate::sources::rpc::ChainTxStats {
        tx_count: 0,
        height: 0,
      })
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
    assert!(
      payload.len() < 0xfd,
      "test payload must fit in one-byte CompactSize"
    );

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
  async fn refresh_summary_once_rebuilds_sparse_recent_snapshot(
  ) -> Result<(), Box<dyn std::error::Error>> {
    // nosemgrep: contrib.r2c.daemon-live-state-must-not-depend-on-rest -- test fixture
    let db = crate::rest::hydb_fake::MemDb::new().into_hydb();

    let live_state = LiveState::new();
    let stale_txid = vec![0xaa; 32];
    live_state.set_mempool_summary(MempoolSummary {
      tx_count: 99,
      total_bytes: 123,
      usage_bytes: Some(456),
      max_bytes: Some(789),
      updated_at: 10,
    });
    // A stale recent tx the refresh snapshot must clear.
    live_state.insert_mempool_tx(MempoolTx {
      txid: stale_txid.clone(),
      raw_tx: Vec::new(),
      version: 1,
      tx_type: "normal".to_owned(),
      lock_time: 0,
      size: 0,
      value_out: 0,
      input_count: 0,
      output_count: 0,
      fee: None,
      payload: None,
      inputs: Vec::new(),
      outputs: Vec::new(),
      first_seen_at: 1,
      last_seen_at: 1,
    });

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

    refresh_summary_once(&db, &live_state, &rpc, 25, 76, 16).await?;

    let summary = live_state
      .mempool_summary()
      .ok_or("summary should be set")?;
    let recent = live_state.mempool_recent(25, None);
    assert_eq!(summary.tx_count, 2);
    assert_eq!(recent.len(), 2);
    assert!(!recent.iter().any(|tx| tx.txid == stale_txid));
    Ok(())
  }

  #[tokio::test]
  async fn refresh_summary_once_persists_special_transactions(
  ) -> Result<(), Box<dyn std::error::Error>> {
    // nosemgrep: contrib.r2c.daemon-live-state-must-not-depend-on-rest -- test fixture
    let db = crate::rest::hydb_fake::MemDb::new().into_hydb();

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

    let live_state = LiveState::new();
    refresh_summary_once(&db, &live_state, &rpc, 25, 76, 16).await?;

    let recent = live_state.mempool_recent(25, None);
    assert_eq!(recent.len(), 1);
    assert_eq!(recent[0].tx_type, "provider_register");
    assert_eq!(recent[0].value_out, 0);
    // Payload enrichment is exercised by the dedicated `crate::tx` decoders
    // against real special-transaction fixtures; this hand-rolled ProRegTx body
    // is not a valid `dash_primitives` payload, so `decode_payload` yields
    // `None` here. The assertion under test is that a special tx flows through
    // the RAM refresh with its correct type.
    Ok(())
  }
}
