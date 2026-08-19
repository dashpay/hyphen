//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The undo-coverage check, against a real database.
//!
//! `src/verify.rs` unit-tests the policy table (which defect refuses a
//! startup, what the refusal says). This file tests the half only a live
//! database can show: that a count-against-span check *finds a hole* where
//! `MIN`/`MAX` cannot, that it tells a dropped partition apart from
//! individually missing rows, that a legitimately pruned floor is not a
//! defect, and that `HyphenDb::connect` refuses to hand out a facade over a
//! database whose tip cannot be rolled back.
//!
//! The scenarios share one 1,300-block chain and mutate the undo ledger in
//! place, restoring between steps: building the chain is the expensive part,
//! and every scenario is a different injury to the same healthy starting
//! point.

mod common;

use std::sync::Arc;

use bytes::Bytes;
use common::TempDb;
use hyphen_db::layout::undo_span;
use hyphen_db::verify::{self, HyDbUndoDefect};
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbError, HyDbHash,
  HyDbHeader, HyDbHeight, HyDbResult, HyDbTx, HyDbTxIn, HyDbTxOut,
};
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ADDRESS: &str = "yUndoCoverage000000000000000000000";

/// Blocks in the shared chain: enough for three `undo_span`-sized partitions
/// (`p0`..`p2` at the default span of 500), so a *middle* partition exists to
/// drop. Deliberately below the point where the real prune fires
/// (`tip - undo_keep_depth`, i.e. 2,000+ blocks at the defaults) so the
/// starting state is "nothing pruned yet, coverage from genesis".
const BLOCKS: u64 = 1_300;

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

/// Connect `count` blocks and advance the projection onto them. An advance is
/// capped at `max_advance_blocks`, so the tip is announced until it stops
/// moving.
async fn build_chain(ingest: &hyphen_db::HyDbIngest, count: u64) -> TestResult {
  let mut prev = [0u8; 32];
  for i in 0..count {
    let hash = seeded(0xC0DE_0000 + i);
    ingest.on_block(raw_seeded(hash, prev, 10)).await?;
    prev = hash;
  }
  let tip = HyDbHash(prev);
  let target = Some(HyDbHeight::new(count as u32 - 1));
  loop {
    let report = ingest.on_tip(tip).await?;
    if report.to == target {
      return Ok(());
    }
    if report.from == report.to {
      return Err(format!("advance stalled at {:?}", report.to).into());
    }
  }
}

/// Copy a partition's rows aside so a scenario can put them back.
async fn stash(pool: &PgPool, name: &str) -> TestResult {
  sqlx::query(&format!(
    "CREATE TABLE IF NOT EXISTS stash_{name} AS SELECT * FROM {name}"
  ))
  .execute(pool)
  .await?;
  Ok(())
}

/// Drop an undo partition outright — what a prune with a stale horizon, or an
/// operator with a `psql` prompt, leaves behind.
async fn drop_partition(pool: &PgPool, name: &str) -> TestResult {
  sqlx::query(&format!("DROP TABLE {name}"))
    .execute(pool)
    .await?;
  Ok(())
}

/// Put a stashed partition back, rows and all.
async fn restore_partition(pool: &PgPool, idx: i32) -> TestResult {
  let span = undo_span();
  let name = format!("undo_blocks_p{idx}");
  let lo = idx * span;
  let hi = lo + span;
  sqlx::query(&format!(
    "CREATE TABLE {name} PARTITION OF undo_blocks FOR VALUES FROM ({lo}) TO ({hi})"
  ))
  .execute(pool)
  .await?;
  sqlx::query(&format!("INSERT INTO {name} SELECT * FROM stash_{name}"))
    .execute(pool)
    .await?;
  Ok(())
}

/// The whole check, exercised as a sequence of injuries to one healthy chain.
#[tokio::test]
async fn undo_coverage_finds_holes_and_forgives_pruning() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = scenarios(&tmp).await;
  tmp.drop_now().await;
  result
}

#[expect(
  clippy::too_many_lines,
  reason = "one chain, many injuries; splitting it would rebuild the chain per case"
)]
async fn scenarios(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(TestSrc)).await?;
  build_chain(&ingest, BLOCKS).await?;
  drop(ingest); // release the single-writer advisory lock before reconnecting

  let pool = tmp.seed_pool().await?;
  let tip = (BLOCKS - 1) as u32;
  let span = undo_span();

  // ---------------------------------------------------------------------
  // 1. A healthy chain: coverage from genesis to the tip, nothing missing.
  // ---------------------------------------------------------------------
  let report = verify::undo_coverage(&pool).await?;
  assert!(
    report.is_clean(),
    "a freshly synced chain must be clean, got {:?}",
    report.defects
  );
  assert_eq!(report.tip.map(|h| h.get()), Some(tip));
  assert_eq!(report.undo_lo.map(|h| h.get()), Some(0));
  assert_eq!(report.undo_hi.map(|h| h.get()), Some(tip));
  assert_eq!(report.expected_rows, i64::from(BLOCKS as u32));
  assert_eq!(report.covered_rows, report.expected_rows);
  assert!(report.startup_refusal().is_none());
  // The same answer through the operator-facing surface: one implementation,
  // two callers, so a health verb can never disagree with the gate.
  let via_status = db.status().undo_coverage().await?;
  assert_eq!(via_status.tip, report.tip);
  assert_eq!(via_status.covered_rows, report.covered_rows);
  assert!(via_status.is_clean());

  // ---------------------------------------------------------------------
  // 2. Undo rows above the canonical tip: reported, never fatal. They are
  //    inert (a disconnect walks *down* from the tip) so refusing here would
  //    cost an operator a restart for nothing.
  // ---------------------------------------------------------------------
  sqlx::query(
    "INSERT INTO undo_blocks (height, block_hash, prev_hash, summary_tx_total,
                              summary_supply_total, summary_supply_included,
                              summary_supply_unspendable)
     SELECT height + 1, block_hash, prev_hash, summary_tx_total,
            summary_supply_total, summary_supply_included, summary_supply_unspendable
       FROM undo_blocks WHERE height = $1",
  )
  .bind(tip as i32)
  .execute(&pool)
  .await?;
  let report = verify::undo_coverage(&pool).await?;
  assert_eq!(report.rows_above_tip, 1);
  assert!(
    matches!(
      report.defects.as_slice(),
      [HyDbUndoDefect::StaleAboveTip { rows: 1, .. }]
    ),
    "a row above the tip is the only defect, got {:?}",
    report.defects
  );
  assert!(
    report.startup_refusal().is_none(),
    "a stale row above the tip must not refuse startup"
  );
  // The floor and the covered window ignore it entirely.
  assert_eq!(report.undo_hi.map(|h| h.get()), Some(tip));
  // ...and a daemon can still open the database.
  drop(db);
  let db = tmp.open().await?;
  sqlx::query("DELETE FROM undo_blocks WHERE height > $1")
    .bind(tip as i32)
    .execute(&pool)
    .await?;

  // ---------------------------------------------------------------------
  // 3. A pruned floor is not a hole. Dropping the lowest partition is exactly
  //    what `prune_undo` does; the retained window simply starts higher, and
  //    the check must stay silent.
  // ---------------------------------------------------------------------
  stash(&pool, "undo_blocks_p0").await?;
  drop_partition(&pool, "undo_blocks_p0").await?;
  let report = verify::undo_coverage(&pool).await?;
  assert!(
    report.startup_refusal().is_none(),
    "a pruned floor must never refuse startup, got {:?}",
    report.defects
  );
  assert!(
    !report
      .defects
      .iter()
      .any(|d| matches!(d, HyDbUndoDefect::Hole { .. })),
    "a pruned floor is not a hole, got {:?}",
    report.defects
  );
  // Dropped this early it *is* shallower than the reorg bound, and that is
  // reported — as a warning, because the data on hand is intact.
  assert!(
    matches!(
      report.defects.as_slice(),
      [HyDbUndoDefect::ShallowRetention { retained: 800, .. }]
    ),
    "got {:?}",
    report.defects
  );
  assert_eq!(report.undo_lo.map(|h| h.get()), Some(span as u32));
  assert_eq!(report.expected_rows, i64::from(tip) - i64::from(span) + 1);
  assert_eq!(report.covered_rows, report.expected_rows);
  // Put it back: with the floor at genesis again, the *next* partition to go
  // is an interior one, which is a different state entirely.
  restore_partition(&pool, 0).await?;

  // ---------------------------------------------------------------------
  // 4. A hole in the middle. This is the case `MIN`/`MAX` cannot see: with
  //    `p1` gone the bounds still read 500..=1299 and look perfectly healthy,
  //    while 500 heights inside them have no pre-image at all.
  // ---------------------------------------------------------------------
  stash(&pool, "undo_blocks_p1").await?;
  drop_partition(&pool, "undo_blocks_p1").await?;

  let (bound_lo, bound_hi): (Option<i32>, Option<i32>) =
    sqlx::query_as("SELECT MIN(height), MAX(height) FROM undo_blocks")
      .fetch_one(&pool)
      .await?;
  assert_eq!(
    (bound_lo, bound_hi),
    (Some(0), Some(tip as i32)),
    "the bounds must still look plausible — that is the point of counting"
  );

  let report = verify::undo_coverage(&pool).await?;
  assert_eq!(report.covered_rows, report.expected_rows - i64::from(span));
  match report.defects.as_slice() {
    [HyDbUndoDefect::Hole {
      missing_heights,
      gaps,
      truncated,
      dropped_partitions,
    }] => {
      assert_eq!(*missing_heights, i64::from(span));
      assert!(!truncated);
      assert_eq!(gaps.len(), 1, "one contiguous run, got {gaps:?}");
      assert_eq!(gaps[0].lo.get(), span as u32);
      assert_eq!(gaps[0].hi.get(), (span * 2 - 1) as u32);
      assert_eq!(gaps[0].heights(), span as u32);
      // The distinguishing detail: the partition is gone, not merely empty.
      assert_eq!(dropped_partitions, &["undo_blocks_p1".to_owned()]);
    }
    other => return Err(format!("expected a single hole defect, got {other:?}").into()),
  }

  // The gate acts on it: no facade over a database that cannot roll back.
  let refusal = report.startup_refusal().unwrap_or_default();
  assert!(refusal.contains("500..=999"), "{refusal}");
  drop(db);
  match tmp.open().await {
    Err(e) => {
      let msg = e.to_string();
      assert!(
        msg.contains("undo coverage is not contiguous"),
        "connect must refuse with the coverage message, got {msg}"
      );
      assert!(msg.contains("re-derive"), "must name a remedy, got {msg}");
    }
    Ok(_) => return Err("connect must refuse a database with an undo hole".into()),
  }
  // And the refusal is fatal-classified, so a supervisor does not retry it.
  let err = verify::undo_coverage(&pool)
    .await?
    .startup_refusal()
    .map(HyDbError::Consistency)
    .ok_or("expected a refusal")?;
  assert_eq!(err.classify(), hyphen_db::Outcome::Fatal);

  // ---------------------------------------------------------------------
  // 5. Rows missing individually, inside a partition that is still there —
  //    what an interrupted write leaves, as opposed to a dropped partition.
  //    Same fatal verdict, different operator response, so the report has to
  //    tell them apart.
  // ---------------------------------------------------------------------
  restore_partition(&pool, 1).await?;
  sqlx::query("DELETE FROM undo_blocks WHERE height IN ($1, $2, $3)")
    .bind(span * 2 + 7)
    .bind(span * 2 + 8)
    .bind(span * 2 + 40)
    .execute(&pool)
    .await?;
  let report = verify::undo_coverage(&pool).await?;
  match report.defects.as_slice() {
    [HyDbUndoDefect::Hole {
      missing_heights,
      gaps,
      dropped_partitions,
      ..
    }] => {
      assert_eq!(*missing_heights, 3);
      assert_eq!(
        gaps.len(),
        2,
        "two runs (a pair and a single), got {gaps:?}"
      );
      assert_eq!(gaps[0].heights(), 2);
      assert_eq!(gaps[1].heights(), 1);
      assert!(
        dropped_partitions.is_empty(),
        "the partition is present — only rows are missing, got {dropped_partitions:?}"
      );
    }
    other => return Err(format!("expected a scattered hole, got {other:?}").into()),
  }

  // ---------------------------------------------------------------------
  // 6. The failure this whole check exists for: a canonical tip beyond its
  //    undo coverage, as a crash mid-reindex leaves it. Pruning can never
  //    produce this — it only ever raises the floor.
  // ---------------------------------------------------------------------
  sqlx::query("DELETE FROM undo_blocks WHERE height > $1")
    .bind(tip as i32 - 3)
    .execute(&pool)
    .await?;
  let report = verify::undo_coverage(&pool).await?;
  assert_eq!(report.undo_hi.map(|h| h.get()), Some(tip - 3));
  assert!(
    report
      .defects
      .iter()
      .any(|d| matches!(d, HyDbUndoDefect::TipUncovered { .. })),
    "coverage short of the tip must be reported, got {:?}",
    report.defects
  );
  let refusal = report.startup_refusal().unwrap_or_default();
  assert!(refusal.contains("3 block(s) at the tip"), "{refusal}");

  // ---------------------------------------------------------------------
  // 7. No undo at all under a live projection. Not reachable by pruning
  //    (which cannot touch the tip's own partition), so it is a refusal.
  // ---------------------------------------------------------------------
  sqlx::query("DELETE FROM undo_blocks")
    .execute(&pool)
    .await?;
  let report = verify::undo_coverage(&pool).await?;
  assert!(
    matches!(
      report.defects.as_slice(),
      [HyDbUndoDefect::NoCoverage { .. }]
    ),
    "an empty undo ledger under a live tip must refuse, got {:?}",
    report.defects
  );
  assert!(report.startup_refusal().is_some());

  Ok(())
}

/// A database with no canonical blocks has nothing to cover — the gate must
/// let a fresh daemon through, undo ledger or no undo ledger. Every other
/// integration test in the suite depends on this implicitly; it is asserted
/// here so a tightening of the gate fails *this* test rather than all of them.
#[tokio::test]
async fn an_empty_projection_is_clean() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = empty_case(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn empty_case(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let report = db.status().undo_coverage().await?;
  assert_eq!(report.tip, None);
  assert_eq!(report.undo_lo, None);
  assert_eq!(report.expected_rows, 0);
  assert_eq!(report.prune_cutoff, None);
  assert!(report.is_clean(), "got {:?}", report.defects);
  assert!(report.startup_refusal().is_none());
  Ok(())
}

/// The same check against a large pre-synced seed database, opt-in through
/// `HYPHEN_SEED_DATABASE_URL`.
///
/// An 860,000-block sync is hours of work and cannot be a fixture, but it is
/// the only place the *pruned* state occurs for real: a floor raised by
/// `prune_undo` itself, over a chain long enough for the horizon to have moved
/// a thousand times. Point this at a clone of such a database (never the seed
/// itself — it mutates the undo ledger) and it asserts the two halves that
/// matter together: a genuinely pruned ledger opens, and a hole above its
/// floor does not.
///
/// Set-but-unreachable fails loudly rather than skipping; unset does nothing,
/// because there is nothing to point at.
// Requires a real pruned seed database, which only exists where an 860k-block
// sync has been run. Marked `ignore` rather than skipping silently on a missing
// env var: a test that passes without asserting anything is the idiom that let
// a completely broken search ship green (E2.8). Run it with
//   HYPHEN_SEED_DATABASE_URL=... cargo test -p hyphen-db --test undo_coverage -- --ignored
#[tokio::test]
#[ignore = "needs HYPHEN_SEED_DATABASE_URL pointing at a pruned seed clone"]
async fn a_pruned_seed_opens_and_a_hole_above_its_floor_does_not() -> TestResult {
  let url = std::env::var("HYPHEN_SEED_DATABASE_URL")
    .map_err(|_| "HYPHEN_SEED_DATABASE_URL must be set to run this test")?;
  let pool = PgPool::connect(&url).await?;

  // 1. As synced: pruned to the keep depth, and clean.
  let report = verify::undo_coverage(&pool).await?;
  assert!(
    report.startup_refusal().is_none(),
    "a pruned seed database must open, got {:?}",
    report.defects
  );
  let tip = report.tip.ok_or("seed database has no canonical tip")?;
  let floor = report.undo_lo.ok_or("seed database has no undo rows")?;
  assert_eq!(report.undo_hi, Some(tip), "undo must reach the tip");
  assert_eq!(report.covered_rows, report.expected_rows);
  assert!(
    floor.get() > 0,
    "this fixture is only interesting once pruning has moved the floor"
  );

  // 2. One more prune round, done by hand: dropping the oldest retained
  //    partition raises the floor and must stay forgiven, however far the
  //    retained window falls below the reorg bound.
  let oldest: i32 = sqlx::query_scalar(
    "SELECT MIN(substring(c.relname from '[0-9]+$')::int)
       FROM pg_inherits i
       JOIN pg_class c ON c.oid = i.inhrelid
       JOIN pg_class p ON p.oid = i.inhparent
      WHERE p.relname = 'undo_blocks'",
  )
  .fetch_one(&pool)
  .await?;
  drop_partition(&pool, &format!("undo_blocks_p{oldest}")).await?;
  let pruned = verify::undo_coverage(&pool).await?;
  assert!(
    pruned.startup_refusal().is_none(),
    "a hand-pruned floor must not refuse startup, got {:?}",
    pruned.defects
  );
  assert!(
    pruned.undo_lo > Some(floor),
    "the floor must have risen: {:?} -> {:?}",
    floor,
    pruned.undo_lo
  );

  // 3. A hole above the new floor. Ten heights out of the middle: the bounds
  //    are untouched, so only the count can see it.
  let new_floor = pruned.undo_lo.ok_or("no undo rows left")?;
  let gap_lo = i32::try_from((i64::from(new_floor.get()) + i64::from(tip.get())) / 2)?;
  let gap_hi = gap_lo + 9;
  sqlx::query("DELETE FROM undo_blocks WHERE height BETWEEN $1 AND $2")
    .bind(gap_lo)
    .bind(gap_hi)
    .execute(&pool)
    .await?;

  let injured = verify::undo_coverage(&pool).await?;
  assert_eq!(
    (injured.undo_lo, injured.undo_hi),
    (pruned.undo_lo, pruned.undo_hi),
    "the bounds must be unchanged — that is the point of counting"
  );
  match injured
    .defects
    .iter()
    .find(|d| matches!(d, HyDbUndoDefect::Hole { .. }))
  {
    Some(HyDbUndoDefect::Hole {
      missing_heights,
      gaps,
      dropped_partitions,
      ..
    }) => {
      assert_eq!(*missing_heights, 10);
      assert_eq!(gaps.len(), 1, "one run, got {gaps:?}");
      assert_eq!(gaps[0].lo.get(), gap_lo as u32);
      assert_eq!(gaps[0].hi.get(), gap_hi as u32);
      assert!(
        dropped_partitions.is_empty(),
        "rows are missing, not a partition, got {dropped_partitions:?}"
      );
    }
    _ => {
      return Err(
        format!(
          "a hole above the floor must be detected, got {:?}",
          injured.defects
        )
        .into(),
      )
    }
  }
  assert!(injured.startup_refusal().is_some());
  Ok(())
}
