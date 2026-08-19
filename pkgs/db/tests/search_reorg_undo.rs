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
//!    instead of growing with the chain, and must never shed a height the
//!    reorg bound can still reach.

mod common;

use std::sync::Arc;

use bytes::Bytes;
use common::TempDb;
use hyphen_db::layout::undo_span;
use hyphen_db::query::HyDbSearchKind;
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHash, HyDbHeader,
  HyDbHeight, HyDbResult, HyDbTx, HyDbTxIn, HyDbTxOut,
};
use sqlx::PgPool;

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
  let tmp = TempDb::create().await?;
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
  let tmp = TempDb::create().await?;
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
// 3. Undo retention: pruned by dropping partitions, never below the horizon
// ===========================================================================
//
// The undo family is partitioned by height and pruned with `DROP TABLE` on a
// whole partition, so retention is the horizon plus up to one partition span
// of overshoot rather than exactly the horizon. What must hold regardless of
// where the boundary happens to fall is that nothing the reorg bound can
// still reach is ever dropped — the three tests below assert the retention
// window, exercise a reorg at the deepest still-reversible height after two
// prunes have run, and pin the two refusals that catch a disconnect whose
// undo is gone.

/// Heights 0..=2099. With a 1000-block horizon and a 500-block partition span
/// the prune has fired twice by this tip (`_p0` and `_p1` are gone), so a
/// reorg down to the deepest reversible height crosses ground the prune has
/// already worked over. The tip is deliberately not a multiple of the span:
/// the cutoff lands mid-partition, which is the case where retention
/// overshoots the horizon and the drop rule has to hold the line.
const PRUNE_BLOCKS: u64 = 2_100;

/// Undo horizon, mirrored from `[db.ingest] undo_keep_depth`'s default
/// (`hyphen_db::tuning::DEFAULT_UNDO_KEEP_DEPTH`).
const KEEP_DEPTH: i32 = 1_000;

/// Connect `count` blocks of value 10 from `seed` and advance the projection
/// onto them, returning their hashes in height order. An advance is capped at
/// `[db.ingest] max_advance_blocks`, so the tip is announced until it stops
/// moving.
async fn build_chain(
  ingest: &hyphen_db::HyDbIngest,
  seed: u64,
  count: u64,
) -> Result<Vec<[u8; 32]>, Box<dyn std::error::Error>> {
  let mut chain = Vec::with_capacity(count as usize);
  let mut prev = [0u8; 32];
  for i in 0..count {
    let hash = seeded(seed + i);
    ingest.on_block(raw_seeded(hash, prev, 10)).await?;
    chain.push(hash);
    prev = hash;
  }
  let tip = HyDbHash(prev);
  let target = Some(HyDbHeight(count as u32 - 1));
  loop {
    let report = ingest.on_tip(tip).await?;
    if report.to == target {
      return Ok(chain);
    }
    if report.from == report.to {
      return Err(format!("advance stalled at {:?}", report.to).into());
    }
  }
}

/// Which `_pN` partitions of `parent` currently exist.
async fn partitions_of(pool: &PgPool, parent: &str) -> Result<Vec<i32>, sqlx::Error> {
  let rows: Vec<(String,)> = sqlx::query_as(
    "SELECT c.relname
       FROM pg_inherits i
       JOIN pg_class c ON c.oid = i.inhrelid
       JOIN pg_class p ON p.oid = i.inhparent
      WHERE p.relname = $1
      ORDER BY 1",
  )
  .bind(parent)
  .fetch_all(pool)
  .await?;
  let prefix = format!("{parent}_p");
  let mut idx: Vec<i32> = rows
    .iter()
    .filter_map(|(n,)| n.strip_prefix(&prefix)?.parse().ok())
    .collect();
  idx.sort_unstable();
  Ok(idx)
}

#[tokio::test]
async fn undo_is_pruned_by_dropping_partitions() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_prune(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_prune(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  build_chain(&ingest, 0x9000, PRUNE_BLOCKS).await?;
  let tip_h = (PRUNE_BLOCKS - 1) as i32;
  assert_eq!(
    db.status().tip().await?.map(|t| t.height),
    Some(HyDbHeight(tip_h as u32))
  );

  let pool = tmp.seed_pool().await?;
  let cutoff = tip_h - KEEP_DEPTH;
  let span = undo_span();

  // Every undo parent is pruned the same way, and pruned to the same place:
  // partitions whose highest possible height is at or below the cutoff are
  // gone, everything a reorg could still reach is present.
  for parent in ["undo_address_stats", "undo_blocks", "undo_validators"] {
    let live = partitions_of(&pool, parent).await?;
    assert!(
      !live.is_empty(),
      "{parent} must have partitions covering the tip"
    );
    for idx in &live {
      let lowest = idx * span;
      assert!(
        (idx + 1) * span - 1 > cutoff,
        "{parent}_p{idx} is entirely below the cutoff {cutoff} and should have been dropped"
      );
      assert!(
        lowest <= tip_h,
        "{parent}_p{idx} sits above the tip {tip_h}"
      );
    }
    let oldest = live.first().copied().ok_or("no live partitions")?;
    assert!(
      oldest * span > cutoff - span,
      "{parent} retains more than one span of overshoot below the cutoff"
    );
  }

  // The undo rows themselves: everything within the horizon is there, the
  // overshoot below it is bounded by one span, and nothing older survives.
  let (count, min_h, max_h): (i64, Option<i32>, Option<i32>) =
    sqlx::query_as("SELECT COUNT(*), MIN(height), MAX(height) FROM undo_blocks")
      .fetch_one(&pool)
      .await?;
  assert_eq!(max_h, Some(tip_h));
  assert!(
    min_h > Some(cutoff - span),
    "retention must not exceed the horizon by more than one partition span; min {min_h:?}"
  );
  assert!(
    min_h <= Some(cutoff + 1),
    "every height inside the horizon must still be reversible; min {min_h:?}"
  );
  assert!(
    count <= i64::from(KEEP_DEPTH + span),
    "undo_blocks must stay bounded; got {count} rows"
  );

  // The pruning mechanism, asserted as such: rows below the cutoff *do*
  // survive (they are the overshoot a partition drop cannot avoid), which is
  // exactly what a row-by-row DELETE prune would have removed. Reading zero
  // here would mean the deletes came back.
  let (stale,): (i64,) =
    sqlx::query_as("SELECT COUNT(*) FROM undo_address_stats WHERE height <= $1")
      .bind(cutoff)
      .fetch_one(&pool)
      .await?;
  assert!(
    stale > 0,
    "the prune drops whole partitions; sub-cutoff rows inside a live partition are kept"
  );

  // Nothing was deleted from the undo family to get here. `n_tup_del` is
  // maintained per relation by the statistics collector and the prune path is
  // the only writer that could raise it on a chain that never disconnected.
  sqlx::query("SELECT pg_stat_force_next_flush()")
    .execute(&pool)
    .await?;
  let (deleted,): (i64,) = sqlx::query_as(
    "SELECT COALESCE(SUM(n_tup_del), 0)::int8 FROM pg_stat_all_tables
      WHERE relname LIKE 'undo\\_%'",
  )
  .fetch_one(&pool)
  .await?;
  assert_eq!(deleted, 0, "the prune path must issue no DELETE");

  // And the retained window still disconnects: rewind a few blocks.
  let report = ingest.rewind_to(HyDbHeight(tip_h as u32 - 3)).await?;
  assert_eq!(report.to, Some(HyDbHeight(tip_h as u32 - 3)));

  Ok(())
}

/// The horizon itself: after two prunes have run, a branch switch forking at
/// exactly `tip - undo_keep_depth` — the deepest fork the reorg bound admits,
/// and so the deepest undo row that must still exist — has to succeed.
#[tokio::test]
async fn deepest_reversible_reorg_survives_the_prune() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_deep_reorg(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_deep_reorg(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  let chain = build_chain(&ingest, 0xA000, PRUNE_BLOCKS).await?;
  let tip_h = (PRUNE_BLOCKS - 1) as i32;

  // Two prunes have fired by now — the test is only worth anything if the
  // drop path has actually run more than once.
  let pool = tmp.seed_pool().await?;
  let ever_created = tip_h / undo_span() + 1;
  let dropped = ever_created - partitions_of(&pool, "undo_address_stats").await?.len() as i32;
  assert!(
    dropped >= 2,
    "the scenario must exercise at least two prunes; only {dropped} partitions were dropped"
  );

  // Fork at the deepest height the reorg bound allows: the replacement block
  // sits at `fork + 1`, so the disconnect unwinds exactly KEEP_DEPTH blocks
  // and the oldest undo row it reads is at `tip - KEEP_DEPTH + 1`.
  let fork = tip_h - KEEP_DEPTH;
  let alt = seeded(0xB000);
  ingest
    .on_block(raw_seeded(alt, chain[fork as usize], 25))
    .await?;
  let report = ingest.on_tip(HyDbHash(alt)).await?;
  let reorg = report
    .reorg
    .ok_or("the deepest admissible fork must reorg, not no-op")?;
  assert_eq!(reorg.fork_height, HyDbHeight(fork as u32));
  assert_eq!(
    reorg.disconnected.len(),
    KEEP_DEPTH as usize,
    "every block above the fork must be disconnected"
  );
  assert_eq!(report.to, Some(HyDbHeight(fork as u32 + 1)));

  // The projection is the replacement branch exactly: heights 0..=fork at 10
  // each, plus the one replacement block at 25.
  let expected = i64::from(fork + 1) * 10 + 25;
  assert_eq!(db.addrs().supply().await?.total, expected);
  assert_eq!(
    db.status().tip().await?.map(|t| t.hash),
    Some(HyDbHash(alt))
  );

  Ok(())
}

/// Below the horizon the answer must be a refusal. A disconnect that finds no
/// undo cannot reverse the block, and the one thing it must never do is
/// return success having left the projection where the block put it.
///
/// The refusal now arrives *before* any work rather than part way down the
/// span. This test used to assert the opposite — that the rewind unwound what
/// it could, committed it, and then stopped at the first height whose undo
/// partition was gone — which is the behaviour the operator rewind was fixed
/// to stop having: every height above the exhaustion point was already
/// permanently committed, the projection served an arbitrary intermediate tip
/// as canonical, and the error named no height. The depth bound the reorg path
/// always had now applies here too, so the target is rejected on arrival and
/// the projection is untouched.
#[tokio::test]
async fn disconnect_below_the_horizon_is_refused() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_below_horizon(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_below_horizon(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  build_chain(&ingest, 0xC000, PRUNE_BLOCKS).await?;
  let tip_h = (PRUNE_BLOCKS - 1) as i32;

  let supply_before = db.addrs().supply().await?.total;

  // A rewind that walks past the retention window is refused outright: the
  // undo it would need is gone, so there is no depth at which any part of it
  // is admissible.
  let Err(err) = ingest.rewind_to(HyDbHeight(0)).await else {
    return Err("a rewind past the retention window must fail, not silently no-op".into());
  };
  let msg = err.to_string();
  assert!(
    msg.contains("undo"),
    "the refusal must name its cause; got {msg}"
  );

  // Nothing was rewound. This is the part that matters: the tip is exactly
  // where the operator left it, so no reader ever saw an intermediate chain,
  // and the undo the retention window still holds was not consumed on the way
  // to the refusal.
  let tip = db
    .status()
    .tip()
    .await?
    .ok_or("the rewind must not unwind the whole chain")?;
  assert_eq!(
    tip.height,
    HyDbHeight(tip_h as u32),
    "a refused rewind must leave the tip untouched, not stranded part way down"
  );
  assert_eq!(db.addrs().supply().await?.total, supply_before);
  let (undo_rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM undo_blocks WHERE height = $1")
    .bind(tip_h)
    .fetch_one(&tmp.seed_pool().await?)
    .await?;
  assert_eq!(
    undo_rows, 1,
    "the tip's undo must still be there — the refusal consumed nothing"
  );

  // The horizon itself is still reachable, so this is a bound and not a
  // blanket refusal: `deepest_reversible_reorg_survives_the_prune` proves the
  // undo is there, and this proves the rewind path will use it.
  let edge = tip_h - KEEP_DEPTH;
  let report = ingest.rewind_to(HyDbHeight(edge as u32)).await?;
  assert_eq!(report.to, Some(HyDbHeight(edge as u32)));

  Ok(())
}

/// A2's own guard, reached directly: the address undo for a height inside the
/// horizon is missing while the block demonstrably moved balances. This is the
/// case that used to disconnect silently, leaving every balance the block
/// moved exactly where the block put it.
#[tokio::test]
async fn missing_address_undo_is_refused_by_name() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_missing_address_undo(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_missing_address_undo(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  build_chain(&ingest, 0xD000, 6).await?;

  // Every block here pays an address, so the balance stage captured undo for
  // height 5. Take it away and leave the rest of the undo intact, so the
  // refusal has to come from the balance stage rather than from the summary
  // stage that disconnects ahead of it.
  let pool = tmp.seed_pool().await?;
  let removed = sqlx::query("DELETE FROM undo_address_stats WHERE height = 5")
    .execute(&pool)
    .await?
    .rows_affected();
  assert!(removed > 0, "height 5 must have had address undo to remove");

  let Err(err) = ingest.rewind_to(HyDbHeight(4)).await else {
    return Err("a disconnect with no address undo must refuse".into());
  };
  let msg = err.to_string();
  assert!(
    msg.contains("balances") && msg.contains("cannot be reversed"),
    "the balance stage must name itself and its cause; got {msg}"
  );

  // And it refused instead of half-reversing: the tip is untouched.
  assert_eq!(
    db.status().tip().await?.map(|t| t.height),
    Some(HyDbHeight(5))
  );

  Ok(())
}
