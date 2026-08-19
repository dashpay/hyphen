//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! End-to-end ingest tests: absorb a synthetic chain, canonicalize/connect via
//! `on_tip`, and read it back through the query API — including a reorg that
//! disconnects and re-derives. Targets `HYPHEN_TEST_DATABASE_URL`, or the
//! default local server when it is unset; an unreachable server fails the test.
//!
//! The synthetic block encoding is `hash(32) || prev_hash(32) || value(8 BE)`;
//! each block is one coinbase transaction crediting a fixed address, so a chain
//! of N blocks leaves that address with the summed value and N UTXOs. (Spend-
//! path ingest coverage arrives with a richer data source.)

mod common;

use std::sync::Arc;

use bytes::Bytes;
use common::TempDb;
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHash, HyDbHeader,
  HyDbHeight, HyDbResult, HyDbTx, HyDbTxIn, HyDbTxOut,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ADDRESS: &str = "yIngestAddr00000000000000000000000";

/// A trivial data source over the `hash || prev || value` encoding.
struct TestSrc;

fn ids(raw: &[u8]) -> HyDbResult<(HyDbHash, HyDbHash)> {
  Ok((
    HyDbHash::from_slice(&raw[..32])?,
    HyDbHash::from_slice(&raw[32..64])?,
  ))
}

impl HyDbDataSrc for TestSrc {
  fn identify(&self, raw: &[u8]) -> HyDbResult<HyDbBlockId> {
    let (hash, prev_hash) = ids(raw)?;
    Ok(HyDbBlockId { hash, prev_hash })
  }

  fn decode(&self, raw: &[u8]) -> HyDbResult<HyDbBlock> {
    let (hash, prev_hash) = ids(raw)?;
    let mut v = [0u8; 8];
    v.copy_from_slice(&raw[64..72]);
    let value = i64::from_be_bytes(v);
    let tx = HyDbTx {
      txid: hash,
      block_pos: 0,
      version: 1,
      tx_type: hyphen_db::HyDbTxType::Coinbase,
      lock_time: 0,
      size: 200,
      value_out: HyDbAmount(value),
      inputs: vec![HyDbTxIn {
        vin_index: 0,
        prev_outpoint: None,
        coinbase: true,
        script_sig: vec![1],
        sequence: 0xffff_ffff,
      }],
      outputs: vec![HyDbTxOut {
        vout: 0,
        value: HyDbAmount(value),
        script_pubkey: vec![0x76, 0xa9, 0x14],
        address: Some(HyDbAddress(ADDRESS.to_owned())),
        address_type: HyDbAddrType::P2pkh,
      }],
      payload: None,
      validator_events: Vec::new(),
    };
    Ok(HyDbBlock {
      header: HyDbHeader {
        hash,
        prev_hash,
        merkle_root: hash,
        time: 1_600_000_000,
        bits: 0x1e0f_ffff,
        difficulty: 1.0,
        nonce: 0,
        version: 1,
        size: 285,
        tx_count: 1,
        chainwork: None,
        coinbase: None,
      },
      txs: vec![tx],
    })
  }
}

fn h(byte: u8) -> [u8; 32] {
  [byte; 32]
}

fn raw(hash: u8, prev: u8, value: i64) -> Bytes {
  let mut v = Vec::with_capacity(72);
  v.extend_from_slice(&h(hash));
  v.extend_from_slice(&h(prev));
  v.extend_from_slice(&value.to_be_bytes());
  Bytes::from(v)
}

#[tokio::test]
async fn ingest_connects_a_chain_and_reorgs() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  // Absorb a 4-block chain (heights 0..=3), genesis first so parents resolve.
  for (hash, prev) in [(1u8, 0u8), (2, 1), (3, 2), (4, 3)] {
    let seen = ingest.on_block(raw(hash, prev, 50)).await?;
    assert!(
      matches!(seen, hyphen_db::HyDbBlockSeen::New),
      "block {hash} should be newly absorbed, got {seen:?}"
    );
  }
  // Re-absorbing is idempotent.
  assert!(matches!(
    ingest.on_block(raw(1, 0, 50)).await?,
    hyphen_db::HyDbBlockSeen::Known
  ));

  // Canonicalize to the tip.
  let report = ingest.on_tip(HyDbHash(h(4))).await?;
  assert_eq!(report.to, Some(HyDbHeight::new(3)));
  assert!(report.reorg.is_none(), "linear connect is not a reorg");

  assert_eq!(
    db.status().tip().await?.map(|t| t.height),
    Some(HyDbHeight::new(3))
  );
  assert!(db.blocks().by_height(HyDbHeight::new(0)).await?.is_some());
  assert_eq!(db.txs().total().await?, 4);

  let summary = db.addrs().summary(HyDbAddress(ADDRESS.to_owned())).await?;
  let summary = summary.ok_or("address should exist after ingest")?;
  assert_eq!(summary.balance, 200, "4 blocks x 50");
  assert_eq!(summary.utxo_count, 4);
  assert_eq!(db.addrs().supply().await?.total, 200);

  // Reorg: a heavier branch off height 1 (b1 = hash 2). alt2/alt3 pay 70 each.
  ingest.on_block(raw(0x12, 2, 70)).await?; // alt2 @ height 2, parent b1
  ingest.on_block(raw(0x13, 0x12, 70)).await?; // alt3 @ height 3, parent alt2
  let report = ingest.on_tip(HyDbHash(h(0x13))).await?;

  let reorg = report
    .reorg
    .ok_or("switching branches must report a reorg")?;
  assert_eq!(
    reorg.disconnected.len(),
    2,
    "old heights 2 and 3 disconnected"
  );
  assert_eq!(report.to, Some(HyDbHeight::new(3)));

  // New canonical tip is alt3.
  let tip = db.status().tip().await?.ok_or("tip after reorg")?;
  assert_eq!(tip.hash, HyDbHash(h(0x13)));

  // Balance reflects the new branch: genesis50 + b1 50 + alt2 70 + alt3 70.
  let summary = db
    .addrs()
    .summary(HyDbAddress(ADDRESS.to_owned()))
    .await?
    .ok_or("address after reorg")?;
  assert_eq!(summary.balance, 240);
  assert_eq!(summary.utxo_count, 4);
  assert_eq!(db.txs().total().await?, 4, "tx count restored after reorg");

  // The old tip block is gone from the canonical projection.
  assert!(
    db.blocks().by_hash(HyDbHash(h(4))).await?.is_none(),
    "disconnected block must not be canonical"
  );
  // ...but survives in the raw store as an orphan.
  assert!(db.blocks().raw_bytes(HyDbHash(h(4))).await?.is_some());
  let topo = db
    .blocks()
    .topology(HyDbHash(h(4)))
    .await?
    .ok_or("orphan topology")?;
  assert!(!topo.canonical, "old block is now non-canonical");

  Ok(())
}
