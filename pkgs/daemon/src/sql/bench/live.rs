//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Benchmarks and seed helpers for live SQL state.

use crate::error::DaemonError;
use hyphend::bench::BenchDataset;
use crate::sql::pg_err;

const PAGE_LIMIT: i64 = 100;

/// Prime live benchmark data.
///
/// # Errors
///
/// Returns an error when PostgreSQL rejects the seed writes.
pub async fn seed(pool: &crate::sql::SqlPool, dataset: BenchDataset) -> Result<(), DaemonError> {
  let row_count = live_rows(dataset);
  let mut tx = pool
    .begin()
    .await
    .map_err(|error| pg_err("begin live seed tx", error))?;
  crate::sql::live::mempool::upsert_summary(
    &mut tx,
    &crate::sql::live::mempool::MempoolSummaryRow {
      tx_count: i64::from(row_count),
      total_bytes: i64::from(row_count) * 250,
      usage_bytes: Some(i64::from(row_count) * 512),
      max_bytes: Some(100_000_000),
      updated_at: 1_700_000_000,
    },
  )
  .await?;
  for index in 0..row_count {
    let txid = super::synthetic_hash(b'L', i64::from(index));
    crate::sql::live::mempool::upsert_recent_tx(
      &mut tx,
      &crate::sql::live::mempool::MempoolRecentTxRow {
        txid: txid.clone(),
        raw_tx: vec![0_u8; 128],
        first_seen_at: 1_700_000_000 + i64::from(index),
        last_seen_at: 1_700_000_000 + i64::from(index),
      },
    )
    .await?;
    crate::sql::live::mempool::upsert_tx_detail(
      &mut tx,
      &crate::sql::live::mempool::MempoolTxDetailRow {
        txid: txid.clone(),
        version: 3,
        tx_type: "normal".to_owned(),
        lock_time: 0,
        size: 250,
        value_out: 1_000,
        input_count: 1,
        output_count: 2,
        fee: Some(10),
        payload: None,
        first_seen_at: 1_700_000_000 + i64::from(index),
        last_seen_at: 1_700_000_000 + i64::from(index),
      },
      &[crate::sql::live::mempool::MempoolTxInputRow {
        txid: txid.clone(),
        vin_index: 0,
        sequence: 0,
        prev_txid: Some(super::synthetic_hash(b'P', i64::from(index))),
        prev_vout: Some(0),
        coinbase: None,
        script_sig: vec![0x51],
        prev_address: Some(format!("XliveIn{index:05}")),
        prev_value: Some(500),
      }],
      &[crate::sql::live::mempool::MempoolTxOutputRow {
        txid: txid.clone(),
        vout_index: 0,
        value: 1_000,
        script_pubkey: vec![0x76, 0xa9, 0x14],
        address: Some(format!("XliveOut{index:05}")),
      }],
      &[crate::sql::live::mempool::MempoolAddressTxRow {
        address: format!("XliveAddr{:05}", index % 100),
        txid: txid.clone(),
        last_seen_at: 1_700_000_000 + i64::from(index),
      }],
    )
    .await?;
  }
  crate::sql::live::network::upsert_snapshot(
    &mut tx,
    &crate::sql::live::network::NetworkSnapshotRow {
      payload: serde_json::json!({"connections": row_count}),
      updated_at: 1_700_000_100,
    },
  )
  .await?;
  crate::sql::live::chainlock::upsert_snapshot(
    &mut tx,
    &crate::sql::live::chainlock::ChainLockSnapshotRow {
      block_hash: super::synthetic_hash(b'C', 1),
      block_height: Some(100),
      core_height: Some(100),
      raw_chainlock: vec![0x11; 96],
      seen_at: 1_700_000_101,
    },
  )
  .await?;
  crate::sql::live::islock::upsert_current(
    &mut tx,
    &crate::sql::live::islock::IslockCurrentRow {
      txid: super::synthetic_hash(b'I', 1),
      raw_islock: vec![0x33; 96],
      seen_at: 1_700_000_102,
    },
  )
  .await?;
  crate::sql::live::gov::replace_objects(
    &mut tx,
    &(0..(row_count / 10).max(1))
      .map(|index| crate::sql::live::gov::GovernanceObjectRow {
        object_hash: super::synthetic_hash(b'G', i64::from(index)),
        parent_hash: None,
        revision: Some(index),
        object_time: Some(1_700_000_200 + i64::from(index)),
        object_type: 1,
        data_json: Some(serde_json::json!({"title": format!("Proposal {index}")})),
        raw_object: vec![0x44; 128],
        updated_at: 1_700_000_200 + i64::from(index),
      })
      .collect::<Vec<_>>(),
  )
  .await?;
  crate::sql::live::gov::replace_votes(
    &mut tx,
    &(0..(row_count / 5).max(1))
      .map(|index| crate::sql::live::gov::GovernanceVoteRow {
        vote_hash: super::synthetic_hash(b'V', i64::from(index)),
        object_hash: super::synthetic_hash(b'G', i64::from(index % (row_count / 10).max(1))),
        signal: Some(1),
        outcome: Some(if index % 2 == 0 { 1 } else { 2 }),
        vote_time: Some(1_700_000_300 + i64::from(index)),
        raw_vote: vec![0x55; 96],
        updated_at: 1_700_000_300 + i64::from(index),
      })
      .collect::<Vec<_>>(),
  )
  .await?;
  crate::sql::live::protx::replace_snapshot(
    &mut tx,
    &(0..(row_count / 20).max(1))
      .map(|index| crate::sql::live::protx::ValidatorProTxSnapshotRow {
        pro_tx_hash: super::synthetic_hash(b'R', i64::from(index)),
        validator_type: Some(if index % 2 == 0 { "Evo" } else { "Regular" }.to_owned()),
        current_status: Some("ENABLED".to_owned()),
        last_paid_height: Some(index),
        consecutive_payments: Some(index % 10),
        pose_penalty: Some(0),
        pose_revived_height: None,
        pose_ban_height: None,
        revocation_reason: None,
        updated_at: 1_700_000_400 + i64::from(index),
      })
      .collect::<Vec<_>>(),
  )
  .await?;
  tx.commit()
    .await
    .map_err(|error| pg_err("commit live seed tx", error))?;
  Ok(())
}

fn live_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 1_000,
    BenchDataset::Default => 5_000,
    BenchDataset::Large => 20_000,
  }
}
