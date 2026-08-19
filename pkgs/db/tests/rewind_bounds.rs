//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! An operator rewind must be as safe as a reorg, and for the same reasons.
//!
//! The reorg path has always been depth-bounded and always committed its whole
//! span in one transaction. The operator rewind had neither property: it took
//! any target the wire handed it and committed one height per iteration, so a
//! target below the retained undo window disconnected committed height by
//! committed height until a stage trap fired — leaving the projection stranded
//! at an arbitrary height, still serving reads as canonical, and reporting a
//! failure that did not say where it stopped.
//!
//! Each test here fails if any one of the four properties is removed:
//!
//! 1. **Bounded** — a target deeper than `max_reorg_depth` is refused, and
//!    refused having changed nothing.
//! 2. **Checked against the data** — the config bound is a promise about
//!    configuration; a hole in `undo_blocks` inside the span is refused
//!    *before* the first disconnect, not discovered during it.
//! 3. **Atomic** — a failure part way down the span (a stage trap the
//!    pre-flight check cannot see) leaves the tip and every projected total
//!    exactly where they were.
//! 4. **Named** — the error identifies the height it stopped at, because that
//!    is the one fact that separates "this target was never reachable" from
//!    "something transient happened".

mod common;

use std::sync::Arc;

use bytes::Bytes;
use common::TempDb;
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHash, HyDbHeader,
  HyDbHeight, HyDbResult, HyDbTipTarget, HyDbTx, HyDbTxIn, HyDbTxOut,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ADDRESS: &str = "yRewindAddr00000000000000000000000";

/// `[db.ingest] max_reorg_depth`, read from the crate rather than mirrored as
/// a literal, so this test tracks the bound instead of restating it. Read
/// *after* the database is open: the tunables are a process-wide `OnceLock`
/// that `connect` installs and that otherwise initialises to the defaults.
fn max_reorg_depth() -> Result<i32, Box<dyn std::error::Error>> {
  Ok(i32::try_from(hyphen_db::tuning::max_reorg_depth())?)
}

/// Trivial data source over the `hash(32) || prev(32) || value(8 BE)`
/// encoding, matching the one `search_reorg_undo.rs` uses.
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

fn seeded(seed: u64) -> [u8; 32] {
  let mut bytes = [0u8; 32];
  bytes[24..].copy_from_slice(&seed.to_be_bytes());
  bytes[0] = 0x5a;
  bytes
}

fn raw_seeded(hash: [u8; 32], prev: [u8; 32], value: i64) -> Bytes {
  let mut v = Vec::with_capacity(72);
  v.extend_from_slice(&hash);
  v.extend_from_slice(&prev);
  v.extend_from_slice(&value.to_be_bytes());
  Bytes::from(v)
}

/// Absorb `count` linked blocks paying 10 each, then advance to the tip.
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
  let target = Some(HyDbHeight::new(count as u32 - 1));
  loop {
    let report = ingest.on_tip(HyDbTipTarget::AtNode(tip)).await?;
    if report.to == target {
      return Ok(chain);
    }
    if report.from == report.to {
      return Err(format!("advance stalled at {:?}", report.to).into());
    }
  }
}

// ---------------------------------------------------------------------------
// 1. The depth bound
// ---------------------------------------------------------------------------

/// A target deeper than `max_reorg_depth` is refused, and refused having done
/// nothing at all.
///
/// This is the property the reorg path has had all along
/// (`from_height - floor > max_reorg_depth` is a hard `Err`) and the operator
/// path did not: `ipc/verb/rewind` handed the wire's `target_height` straight
/// through. Without the bound this rewind does not fail here — it starts
/// disconnecting, commits its way down past the retention window, and only
/// then trips a stage trap, by which point the tip is a thousand blocks below
/// where the operator left it.
#[tokio::test]
async fn a_target_below_the_reorg_bound_is_refused_before_any_work() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_depth_bound(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_depth_bound(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;
  let bound = max_reorg_depth()?;

  // One block past the bound, so height 0 is exactly one deeper than a rewind
  // may reach and the admissible edge is still on the chain.
  let count = bound as u64 + 2;
  build_chain(&ingest, 0xE000, count).await?;
  let tip_h = (count - 1) as i32;
  let supply_before = db.addrs().supply().await?.total;

  let Err(err) = ingest.rewind_to(HyDbHeight::new(0)).await else {
    return Err("a rewind deeper than max_reorg_depth must be refused".into());
  };
  let msg = err.to_string();
  assert!(
    msg.contains(&format!("{}", tip_h - bound)),
    "the refusal must name the deepest target it would accept; got {msg}"
  );
  assert!(
    msg.contains("nothing was rewound"),
    "the refusal must say it changed nothing; got {msg}"
  );

  // Nothing moved. Not the tip, and not a single projected total.
  assert_eq!(
    db.status().tip().await?.map(|t| t.height),
    Some(HyDbHeight::new(tip_h as u32)),
    "a refused rewind must leave the tip exactly where it was"
  );
  assert_eq!(db.addrs().supply().await?.total, supply_before);

  // ...and the edge of the bound is still admissible, so the bound is a bound
  // and not a blanket refusal.
  let edge = tip_h - bound;
  let report = ingest.rewind_to(HyDbHeight::new(edge as u32)).await?;
  assert_eq!(report.to, Some(HyDbHeight::new(edge as u32)));
  assert_eq!(
    report.reorg.map_or(0, |r| r.disconnected.len()),
    bound as usize
  );
  assert_eq!(db.addrs().supply().await?.total, i64::from(edge + 1) * 10);

  Ok(())
}

// ---------------------------------------------------------------------------
// 1b. The width of the target itself
// ---------------------------------------------------------------------------

/// A target that cannot be represented as the `INTEGER` the projection stores
/// heights at must be refused, not narrowed.
///
/// `u32::MAX as i32` is `-1`. A floor of `-1` against a tip of 5 measures a
/// span of 6 blocks, which every depth bound accepts, and the disconnect loop
/// then walks the chain to height 0 and commits an empty projection. The
/// operator asked for an absurd height and got a wiped database.
#[tokio::test]
async fn a_target_wider_than_the_stored_height_is_refused() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_target_width(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_target_width(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;
  build_chain(&ingest, 0xE100, 6).await?;
  let supply_before = db.addrs().supply().await?.total;

  for target in [u32::MAX, 0x8000_0000, i32::MAX as u32 + 1] {
    let Err(err) = ingest.rewind_to(HyDbHeight::new(target)).await else {
      return Err(format!("a target of {target} must be refused, not narrowed").into());
    };
    assert!(
      err.to_string().contains("exceeds the stored INTEGER range"),
      "the refusal must name the width, not a wrapped depth; got {err}"
    );
  }

  assert_eq!(
    db.status().tip().await?.map(|t| t.height),
    Some(HyDbHeight::new(5)),
    "a refused rewind must leave the tip where it was"
  );
  assert_eq!(db.addrs().supply().await?.total, supply_before);

  Ok(())
}

/// A target at or above the tip disconnects nothing and touches nothing.
///
/// The admissibility gates pass trivially for a non-positive span, so without
/// an early return the call still leaves stepped mode, re-applies the storage
/// profile and settles the open partition's deferred indexes — minutes of DDL
/// under the ingest lock, for a request that changes no rows.
#[tokio::test]
async fn a_target_at_or_above_the_tip_is_a_no_op() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_no_op(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_no_op(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;
  build_chain(&ingest, 0xE200, 6).await?;
  let supply_before = db.addrs().supply().await?.total;

  for target in [5_u32, 6, 99] {
    let report = ingest.rewind_to(HyDbHeight::new(target)).await?;
    assert!(
      report.reorg.is_none(),
      "a target at or above the tip must disconnect nothing; got {:?}",
      report.reorg
    );
    assert_eq!(report.from, report.to, "the tip must not move");
    assert_eq!(report.to, Some(HyDbHeight::new(5)));
  }
  assert_eq!(db.addrs().supply().await?.total, supply_before);

  Ok(())
}

// ---------------------------------------------------------------------------
// 2. The data-side check
// ---------------------------------------------------------------------------

/// The config bound says the undo is there; this asks the database.
///
/// `HyDbIngestCfg::validate` refuses `max_reorg_depth > undo_keep_depth` at
/// load, but that is a statement about two numbers in a file. Lower
/// `undo_span` and restart against an existing database, race a prune against
/// a config change, or raise the bound on a projection synced under a
/// shallower one, and the config check still passes. A hole in `undo_blocks`
/// inside the requested span must be a refusal *before* the first disconnect —
/// `status().undo_span()` cannot see one, because MIN/MAX bounds a range and
/// says nothing about what is inside it.
///
/// Without the pre-flight check this rewind disconnects and commits heights 5
/// and 4, then traps at 3 with the tip permanently at 4.
#[tokio::test]
async fn a_hole_in_the_undo_is_refused_before_the_first_disconnect() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_undo_hole(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_undo_hole(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  build_chain(&ingest, 0xF000, 6).await?;
  let supply_before = db.addrs().supply().await?.total;

  // Punch one hole, well inside the depth bound, and leave the summary undo
  // for 5 and 4 intact so the span is only unreversible *below* them.
  let pool = tmp.seed_pool().await?;
  let removed = sqlx::query("DELETE FROM undo_blocks WHERE height = 3")
    .execute(&pool)
    .await?
    .rows_affected();
  assert_eq!(removed, 1, "height 3 must have had a summary undo row");

  let Err(err) = ingest.rewind_to(HyDbHeight::new(1)).await else {
    return Err("a rewind across a hole in undo_blocks must be refused".into());
  };
  let msg = err.to_string();
  assert!(
    msg.contains("undo coverage is incomplete"),
    "the refusal must name its cause; got {msg}"
  );
  assert!(
    msg.contains("highest at 3"),
    "the refusal must name the height it would have trapped on; got {msg}"
  );

  // The whole point: heights 5 and 4 were reversible and were NOT reversed,
  // because the span is admissible or it is not.
  assert_eq!(
    db.status().tip().await?.map(|t| t.height),
    Some(HyDbHeight::new(5)),
    "a refused rewind must not have disconnected the reversible prefix"
  );
  assert_eq!(db.addrs().supply().await?.total, supply_before);
  let (undo_rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM undo_blocks WHERE height >= 4")
    .fetch_one(&pool)
    .await?;
  assert_eq!(
    undo_rows, 2,
    "the undo for the reversible prefix must still be there — nothing consumed it"
  );

  // A target that clears the hole is still accepted.
  let report = ingest.rewind_to(HyDbHeight::new(4)).await?;
  assert_eq!(report.to, Some(HyDbHeight::new(4)));

  Ok(())
}

/// The three undo families share one horizon, so the pre-flight has to ask
/// about all three.
///
/// It probed `undo_blocks` only. That table carries a row per block, so the
/// per-height check works there and nowhere else — the other two legitimately
/// hold no row for a height that touched no address or validator. A span
/// covered in `undo_blocks` but pruned in `undo_address_stats` therefore read
/// as admissible, and the balance stage refused it part-way down the
/// disconnect, which is the stage trap this pre-flight exists to prevent.
#[tokio::test]
async fn a_pruned_sibling_undo_family_is_refused_before_the_first_disconnect() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_sibling_prune(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_sibling_prune(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  build_chain(&ingest, 0xF100, 6).await?;
  let supply_before = db.addrs().supply().await?.total;
  let pool = tmp.seed_pool().await?;

  // Exactly the divergence a per-relation prune could leave behind: one
  // family's partition gone, the one the pre-flight reads still present.
  sqlx::query("DROP TABLE IF EXISTS undo_address_stats_p0")
    .execute(&pool)
    .await?;
  let (blocks_rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM undo_blocks")
    .fetch_one(&pool)
    .await?;
  assert!(
    blocks_rows > 0,
    "undo_blocks must still cover the span, or this proves nothing"
  );

  let Err(err) = ingest.rewind_to(HyDbHeight::new(1)).await else {
    return Err("a rewind whose address-stats undo was pruned must be refused".into());
  };
  let msg = err.to_string();
  assert!(
    msg.contains("undo_address_stats_p0"),
    "the refusal must name the family and partition that is missing; got {msg}"
  );

  assert_eq!(
    db.status().tip().await?.map(|t| t.height),
    Some(HyDbHeight::new(5)),
    "a refused rewind must not have disconnected anything"
  );
  assert_eq!(db.addrs().supply().await?.total, supply_before);

  Ok(())
}

/// Every `address_stats` accumulation, as a comparable snapshot.
async fn address_stats(
  pool: &sqlx::PgPool,
) -> Result<Vec<(String, i64, i64, i64, i64, i64)>, sqlx::Error> {
  sqlx::query_as(
    "SELECT address, balance::bigint, total_received::bigint, tx_count::bigint,
            utxo_count::bigint, last_seen_height::bigint
       FROM address_stats ORDER BY address",
  )
  .fetch_all(pool)
  .await
}

// ---------------------------------------------------------------------------
// 3 + 4. Atomicity, and the height in the message
// ---------------------------------------------------------------------------

/// A trap the pre-flight check cannot see, sprung mid-span.
///
/// `undo_blocks` coverage is complete, so the pre-flight check passes and the
/// rewind genuinely starts. Height 5 disconnects cleanly; height 4 then hits
/// the balances trap, because its `undo_address_stats` rows are gone while the
/// block demonstrably moved balances. This is the residual case atomicity is
/// for, and there will always be one — the pre-flight check is a filter, not a
/// proof.
///
/// With one commit per height (the old shape) height 5's disconnect is already
/// durable when 4 fails: the tip lands on 4, the supply is short by 10, and
/// the operator is told `rewind failed: <stage error>` with no height in it.
/// With one transaction for the span, the tip is 5, the supply is unchanged,
/// and the error names 4.
#[tokio::test]
async fn a_stage_trap_mid_span_rolls_back_the_whole_rewind() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_mid_span_trap(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_mid_span_trap(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;

  build_chain(&ingest, 0x1_0000, 6).await?;
  let tip_before = db.status().tip().await?.ok_or("chain must have a tip")?;
  let supply_before = db.addrs().supply().await?.total;

  let pool = tmp.seed_pool().await?;
  let stats_before = address_stats(&pool).await?;
  let removed = sqlx::query("DELETE FROM undo_address_stats WHERE height = 4")
    .execute(&pool)
    .await?
    .rows_affected();
  assert!(removed > 0, "height 4 must have had address undo to remove");

  let Err(err) = ingest.rewind_to(HyDbHeight::new(1)).await else {
    return Err("a disconnect with no address undo must refuse".into());
  };
  let msg = err.to_string();

  // 4. The height, by name. Both of these fail on the old error string.
  assert!(
    msg.contains("failed at height 4"),
    "the error must name the height it stopped at; got {msg}"
  );
  assert!(
    msg.contains("nothing was committed") && msg.contains("tip is still 5"),
    "the error must tell the operator the projection is unchanged, and where; got {msg}"
  );
  assert!(
    msg.contains("balances") && msg.contains("cannot be reversed"),
    "the underlying stage error must survive being wrapped; got {msg}"
  );

  // 3. Atomicity. Height 5 was disconnected inside the transaction and the
  //    transaction was never sent, so every one of these is untouched.
  let tip_after = db.status().tip().await?.ok_or("the tip must survive")?;
  assert_eq!(
    (tip_after.height, tip_after.hash),
    (tip_before.height, tip_before.hash),
    "the tip must be the pre-rewind tip, hash included"
  );
  assert_eq!(
    db.addrs().supply().await?.total,
    supply_before,
    "a rolled-back rewind must not have moved the supply"
  );
  assert_eq!(
    address_stats(&pool).await?,
    stats_before,
    "nor any address total"
  );
  let (undo_rows,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM undo_blocks WHERE height = 5")
    .fetch_one(&pool)
    .await?;
  assert_eq!(
    undo_rows, 1,
    "height 5's undo must still be there: its disconnect was rolled back with the rest"
  );

  Ok(())
}
