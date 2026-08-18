//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Regression tests for the three behaviours the original design shipped
//! broken or unbounded:
//!
//! 1. **Search byte order** — a prefix copied from the UI (display-order hex,
//!    the byte-reverse of the stored bytes) must find its row. The original
//!    matched internal-order hex, so UI prefixes could never match.
//! 2. **Multi-block reorgs** — including the same-height reorg the original
//!    height-only comparison could not see.
//! 3. **Undo-table pruning** — the undo tables must stay depth-bounded
//!    instead of growing with the chain.

mod common;

use std::sync::Arc;

use bytes::Bytes;
use common::TempDb;
use hyphen_db::query::HyDbSearchKind;
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHash, HyDbHeader,
  HyDbHeight, HyDbResult, HyDbTx, HyDbTxIn, HyDbTxOut,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ADDRESS: &str = "ySearchAddr00000000000000000000000";

/// Trivial data source over the `hash(32) || prev(32) || value(8 BE)` encoding.
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

/// A distinct 32-byte hash from a u64 seed. Deliberately asymmetric (seed in
/// the HIGH bytes of the internal representation) so display order and
/// internal order never coincide by accident.
fn seeded(seed: u64) -> [u8; 32] {
  let mut bytes = [0u8; 32];
  bytes[24..].copy_from_slice(&seed.to_be_bytes());
  bytes[0] = 0x5a; // ensure the low end is non-zero too
  bytes
}

fn raw_seeded(hash: [u8; 32], prev: [u8; 32], value: i64) -> Bytes {
  let mut v = Vec::with_capacity(72);
  v.extend_from_slice(&hash);
  v.extend_from_slice(&prev);
  v.extend_from_slice(&value.to_be_bytes());
  Bytes::from(v)
}

/// Render a hash the way the UI does: display order = reversed-byte hex.
/// (This is exactly `HyDbHash`'s `Debug` rendering; spelled out here so the
/// test cannot drift with the impl.)
fn ui_hex(hash: &[u8; 32]) -> String {
  hash.iter().rev().map(|b| format!("{b:02x}")).collect()
}

// ===========================================================================
// 1. Search byte-order round trip
// ===========================================================================

#[tokio::test]
async fn ui_rendered_prefix_finds_its_row() -> TestResult {
  let tmp = TempDb::create().await?.ok_or("test database unavailable")?;
  let result = run_search(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_search(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  // A 3-block chain with asymmetric hashes.
  let genesis = seeded(0xA1);
  let b1 = seeded(0xB2);
  let b2 = seeded(0xC3);
  ingest.on_block(raw_seeded(genesis, [0u8; 32], 50)).await?;
  ingest.on_block(raw_seeded(b1, genesis, 50)).await?;
  ingest.on_block(raw_seeded(b2, b1, 50)).await?;
  ingest.on_tip(HyDbHash(b2)).await?;

  // Render the way the UI does, copy a prefix, search with it. This is the
  // case the original code could never satisfy: it matched internal-order
  // hex against a display-order prefix.
  let display = ui_hex(&b1);
  let prefix = &display[..16];

  // The unified search finds it (as both block and tx: the synthetic txid
  // equals the block hash) and returns display-order ids.
  let hits = db.search().prefix(prefix.to_owned(), 10).await?;
  assert!(
    hits
      .iter()
      .any(|h| h.kind == HyDbSearchKind::Block && h.id == display),
    "unified search must find the block by its UI prefix; got {hits:?}"
  );
  assert!(
    hits
      .iter()
      .any(|h| h.kind == HyDbSearchKind::Tx && h.id == display),
    "unified search must find the tx by its UI prefix; got {hits:?}"
  );

  // Domain-level prefix searches agree, and are case-insensitive on hex.
  let blocks = db.blocks().search_prefix(prefix.to_uppercase(), 10).await?;
  assert!(
    blocks.iter().any(|b| b.hash == HyDbHash(b1)),
    "block domain search must match a UI prefix regardless of case"
  );
  let txs = db.txs().search_prefix(prefix.to_owned(), 10).await?;
  assert!(
    txs.contains(&HyDbHash(b1)),
    "tx domain search must match a UI prefix"
  );

  // An internal-order prefix (what the old code indexed) must NOT match:
  // the contract is display order, one order only.
  let internal = hex::encode(b1);
  let internal_prefix = internal[..16].to_owned();
  assert_ne!(
    internal[..16],
    display[..16],
    "orders must differ for this to test anything"
  );
  let hits = db.search().prefix(internal_prefix, 10).await?;
  assert!(
    !hits.iter().any(|h| h.id == display),
    "internal-order prefixes are not part of the contract"
  );

  // LIKE metacharacters are escaped: a bare '%' matches nothing rather than
  // everything.
  let hits = db.search().prefix("%".to_owned(), 10).await?;
  assert!(
    hits.is_empty(),
    "a bare %% must not match every row; got {hits:?}"
  );

  Ok(())
}

// ===========================================================================
// 2. Multi-block and same-height reorgs
// ===========================================================================

#[tokio::test]
async fn multi_block_and_same_height_reorgs() -> TestResult {
  let tmp = TempDb::create().await?.ok_or("test database unavailable")?;
  let result = run_reorg(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_reorg(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  // Heights 0..=5 on the original branch.
  let mut chain: Vec<[u8; 32]> = Vec::new();
  let mut prev = [0u8; 32];
  for i in 0..6u64 {
    let hash = seeded(0x1000 + i);
    ingest.on_block(raw_seeded(hash, prev, 10)).await?;
    chain.push(hash);
    prev = hash;
  }
  ingest.on_tip(HyDbHash(chain[5])).await?;
  assert_eq!(
    db.status().tip().await?.map(|t| t.height),
    Some(HyDbHeight(5))
  );
  let supply_before = db.addrs().supply().await?.total;
  assert_eq!(supply_before, 60);

  // A 4-block replacement branch forking above height 2 (replaces heights
  // 3, 4, 5 and adds 6).
  let mut alt_prev = chain[2];
  let mut alt: Vec<[u8; 32]> = Vec::new();
  for i in 0..4u64 {
    let hash = seeded(0x2000 + i);
    ingest.on_block(raw_seeded(hash, alt_prev, 20)).await?;
    alt.push(hash);
    alt_prev = hash;
  }
  let report = ingest.on_tip(HyDbHash(alt[3])).await?;
  let reorg = report
    .reorg
    .ok_or("multi-block branch switch must report a reorg")?;
  assert_eq!(reorg.disconnected.len(), 3, "heights 3..=5 disconnected");
  assert_eq!(reorg.fork_height, HyDbHeight(2));
  // Disconnection order: tip first.
  assert_eq!(reorg.disconnected.first(), Some(&HyDbHash(chain[5])));
  assert_eq!(report.to, Some(HyDbHeight(6)));

  // Projection reflects the new branch exactly: 3 x 10 + 4 x 20.
  assert_eq!(db.addrs().supply().await?.total, 110);
  assert_eq!(
    db.status().tip().await?.map(|t| t.hash),
    Some(HyDbHash(alt[3]))
  );
  assert!(db.blocks().by_hash(HyDbHash(chain[4])).await?.is_none());

  // Same-height reorg: a sibling for height 6 (same parent, different hash).
  // The original detector compared heights only and was blind to this.
  let sibling = seeded(0x3000);
  ingest.on_block(raw_seeded(sibling, alt[2], 25)).await?;
  let report = ingest.on_tip(HyDbHash(sibling)).await?;
  let reorg = report
    .reorg
    .ok_or("same-height branch switch must report a reorg")?;
  assert_eq!(reorg.disconnected, vec![HyDbHash(alt[3])]);
  assert_eq!(
    db.status().tip().await?.map(|t| t.hash),
    Some(HyDbHash(sibling))
  );
  assert_eq!(db.addrs().supply().await?.total, 115, "20 swapped for 25");

  Ok(())
}

// ===========================================================================
// 3. Undo tables stay depth-bounded
// ===========================================================================

#[tokio::test]
async fn undo_tables_are_pruned() -> TestResult {
  let tmp = TempDb::create().await?.ok_or("test database unavailable")?;
  let result = run_prune(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_prune(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  // 1,100 blocks: comfortably past the 1,000-block undo retention depth.
  const BLOCKS: u64 = 1_100;
  let mut prev = [0u8; 32];
  let mut tip = [0u8; 32];
  for i in 0..BLOCKS {
    tip = seeded(0x9000 + i);
    ingest.on_block(raw_seeded(tip, prev, 10)).await?;
    prev = tip;
  }
  let report = ingest.on_tip(HyDbHash(tip)).await?;
  assert_eq!(report.to, Some(HyDbHeight(BLOCKS as u32 - 1)));

  // Undo rows exist only within the retention window below the tip.
  let pool = tmp.seed_pool().await?;
  let (count, min_h, max_h): (i64, Option<i32>, Option<i32>) =
    sqlx::query_as("SELECT COUNT(*), MIN(height), MAX(height) FROM undo_blocks")
      .fetch_one(&pool)
      .await?;
  let tip_h = (BLOCKS - 1) as i32;
  assert_eq!(max_h, Some(tip_h));
  assert!(
    count <= 1_000,
    "undo_blocks must stay depth-bounded; got {count} rows (min {min_h:?})"
  );
  assert!(
    min_h > Some(tip_h - 1_001),
    "rows below the retention window must be pruned; min {min_h:?}"
  );
  let (addr_count,): (i64,) =
    sqlx::query_as("SELECT COUNT(*) FROM undo_address_stats WHERE height <= $1")
      .bind(tip_h - 1_001)
      .fetch_one(&pool)
      .await?;
  assert_eq!(addr_count, 0, "undo_address_stats pruned below the window");

  // And the retained window still disconnects: rewind a few blocks.
  let report = ingest.rewind_to(HyDbHeight(tip_h as u32 - 3)).await?;
  assert_eq!(report.to, Some(HyDbHeight(tip_h as u32 - 3)));

  Ok(())
}
