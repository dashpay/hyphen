//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! What one block costs the write path, counted rather than asserted.
//!
//! Two numbers from a live mainnet sync motivate this file. `INSERT INTO
//! address_txs` was the active statement in 40 of 40 consecutive
//! `pg_stat_activity` samples, and the two singleton summaries took 112,326
//! updates each against 305,233 and 122,589 *sequential scans* — 2.7 and 1.1
//! scans per update, on tables holding one row. Neither number is visible from
//! a correctness test: the projection is right either way. So this file
//! measures the write path's per-block cost the same way the server reports it,
//! through `pg_stat_all_tables`, over a synthetic chain shaped like the mainnet
//! tail (~140 address touches per block, the p8 partition's 13.9M rows over
//! 100,000 blocks).
//!
//! The assertions are budgets, not timings — timings belong in a benchmark and
//! would be flaky here. A budget catches the regression that actually happened:
//! a summary read spelled as three scalar subqueries scans the singleton three
//! times per block, and nothing about that shows up in a fidelity test.
//!
//! Measured here, 400 blocks of 140.7 address touches each, PostgreSQL 18.3,
//! before → after the A5 and A7 changes. "Accesses" is
//! `pg_stat_all_tables.seq_scan + idx_scan`; "pages" is
//! `pg_statio_user_tables` heap + index buffer accesses per block, which is the
//! currency that decides whether the write path sits in `IO:DataFileRead` on a
//! database far larger than `shared_buffers`:
//!
//! | counter                         | before  | after  |
//! |---------------------------------|---------|--------|
//! | address_txs pages/block         | 1489.57 | 951.18 |
//! | supply_summary pages/block      |   13.95 |   5.00 |
//! | supply_summary accesses/block   |       4 |      1 |
//! | supply_summary updates/block    |       1 |      1 |
//! | transactions_summary accesses   |       2 |      1 |
//! | transactions_summary updates    |       1 |      1 |
//! | statements in the stats stage   |       6 |      2 |
//!
//! `address_txs` lost a third of its pages to dropping `ON CONFLICT DO
//! NOTHING`: an arbiter probe is a second descent of the unique index for every
//! row, and the writer already deduplicates on exactly that key. The update
//! counts are deliberately unchanged — reversibility requires the projection to
//! be complete at every block, so the write cannot be deferred or coalesced
//! across blocks. What went away is the reading and re-reading around it.
//!
//! It then lost a third of what remained to the collapse of its two btrees
//! into one — `address_txs_pkey (address, block_height, txid)` was a
//! strict key prefix of `idx_address_txs_addr_height_desc`, so the second
//! descent it cost per row bought nothing. Same machine, same 400 blocks, the
//! pkey the only difference:
//!
//! | counter                         | two btrees | one btree |
//! |---------------------------------|------------|-----------|
//! | address_txs pages/block         |    1232.00 |    830.37 |
//!
//! Wall clock is flat across that pair (24.4 against
//! 26.6 ms/block) and that is the expected result, not a disappointment: 400
//! blocks fit in `shared_buffers`, `read` is 0, and there is no physical IO for
//! a smaller index to save. The same comparison at one partition's scale — an
//! 8.46M-row `address_txs_p8` stand-in, 881 MB of heap, cold buffers — is where
//! the pages turn into time: 1423.0 pages and 287.2 physical reads per block
//! against 859.0 and 141.7, and 67.9 ms/block against 29.3.
//!
//! The `read` column is zero here and that is expected: 400 blocks fit entirely
//! in `shared_buffers`, so every page touched is a hit. The count of pages
//! touched is what carries over to a database that does not fit — at mainnet
//! scale `address_txs` holds 60.0M rows under what was 17.6 GB of index and is
//! 8.8 GB of it once the primary key went, and a page touched on a btree that
//! read.

mod common;

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Instant;

use bytes::Bytes;
use common::TempDb;
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHash, HyDbHeader,
  HyDbHeight, HyDbOutpoint, HyDbResult, HyDbTipTarget, HyDbTx, HyDbTxIn, HyDbTxOut, HyDbTxType,
};
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Blocks ingested. Enough that a per-block counter is unambiguous, small
/// enough that the whole file runs in a few seconds.
const BLOCKS: u32 = 400;
/// Non-coinbase transactions per block. With two new addresses per spend, one
/// prior address per spend and one coinbase output per spend, this puts
/// `4 * SPENDS + 1 = 141` address touches in a block — the mainnet tail's
/// measured 140 (13,970,758 `address_txs_p8` rows over 100,000 blocks).
const SPENDS: usize = 35;
/// Distinct addresses the chain draws from. Mainnet holds 18.1M addresses
/// across 60.0M `address_txs` rows, so an address recurs 3.3 times; the pool is
/// sized to reproduce that recurrence rather than to make every key unique,
/// because a unique-key chain would understate index depth and overstate the
/// `ON CONFLICT` arbiter's cost.
const ADDR_POOL: u64 = (BLOCKS as u64 * (4 * SPENDS as u64 + 1)) / 3;

const COINBASE_VALUE: i64 = 500_000_000;
/// Value of each of the coinbase's spendable side outputs — the UTXOs the next
/// block's transactions consume.
const SEED_VALUE: i64 = 100_000;
/// Each spend pays two outputs and leaves a fee behind.
const SPEND_OUT: i64 = SEED_VALUE / 2 - 1;

/// A scattered 34-character address, the mainnet average length. The
/// multiplication by a 64-bit odd constant spreads consecutive `k` across the
/// key space, so inserts land on unrelated btree leaves the way real address
/// keys do — an ascending address pool would turn every index insert into a
/// right-edge append and measure nothing.
fn addr(k: u64) -> String {
  format!("y{:033x}", k.wrapping_mul(0x9E37_79B9_7F4A_7C15))
}

/// Pick an address for slot `(height, tx, out)` out of the pool.
fn pick(height: u32, tx: usize, out: usize) -> String {
  let mut x = u64::from(height)
    .wrapping_mul(0x0000_1000_0000_0001)
    .wrapping_add((tx as u64) << 20)
    .wrapping_add(out as u64);
  x ^= x >> 33;
  x = x.wrapping_mul(0xFF51_AFD7_ED55_8CCD);
  x ^= x >> 29;
  addr(x % ADDR_POOL)
}

fn block_hash(height: u32) -> [u8; 32] {
  let mut a = [0u8; 32];
  a[..4].copy_from_slice(&height.to_le_bytes());
  a[4] = 0xB1;
  a
}

fn coinbase_txid(height: u32) -> HyDbHash {
  HyDbHash(block_hash(height))
}

fn spend_txid(height: u32, tx: usize) -> HyDbHash {
  let mut a = [0u8; 32];
  a[..4].copy_from_slice(&height.to_le_bytes());
  a[4..8].copy_from_slice(&(tx as u32).to_le_bytes());
  a[8] = 0x7A;
  HyDbHash(a)
}

fn out_to(vout: u32, value: i64, to: String) -> HyDbTxOut {
  HyDbTxOut {
    vout,
    value: HyDbAmount(value),
    script_pubkey: vec![0x76, 0xa9, 0x14, (vout & 0xff) as u8],
    address: Some(HyDbAddress(to)),
    address_type: HyDbAddrType::P2pkh,
  }
}

/// A chain generated from the height alone: block `h`'s coinbase creates
/// `SPENDS` small outputs, and block `h + 1` spends every one of them. No
/// bookkeeping is needed to keep it valid, so the generator stays a pure
/// function and the writer sees a fresh, scattered address set every block.
struct Chain;

impl Chain {
  fn raw(height: u32) -> Bytes {
    let mut v = Vec::with_capacity(64);
    v.extend_from_slice(&block_hash(height));
    v.extend_from_slice(&if height == 0 {
      [0u8; 32]
    } else {
      block_hash(height - 1)
    });
    Bytes::from(v)
  }

  fn height_of(hash: &[u8]) -> u32 {
    u32::from_le_bytes([hash[0], hash[1], hash[2], hash[3]])
  }
}

impl HyDbDataSrc for Chain {
  fn identify(&self, raw: &[u8]) -> HyDbResult<HyDbBlockId> {
    Ok(HyDbBlockId {
      hash: HyDbHash::from_slice(&raw[..32])?,
      prev_hash: HyDbHash::from_slice(&raw[32..64])?,
    })
  }

  fn decode(&self, raw: &[u8]) -> HyDbResult<HyDbBlock> {
    let hash = HyDbHash::from_slice(&raw[..32])?;
    let prev = HyDbHash::from_slice(&raw[32..64])?;
    let height = Chain::height_of(&hash.0);

    // The coinbase: one large output plus one seed output per spend the next
    // block will make.
    let mut cb_outs = vec![out_to(0, COINBASE_VALUE, pick(height, 0, 0))];
    for j in 1..=SPENDS {
      cb_outs.push(out_to(j as u32, SEED_VALUE, pick(height, 0, j)));
    }
    let cb_value = COINBASE_VALUE + SEED_VALUE * SPENDS as i64;
    let mut txs = vec![HyDbTx {
      txid: coinbase_txid(height),
      block_pos: 0,
      version: 1,
      tx_type: HyDbTxType::Coinbase,
      lock_time: 0,
      size: 250,
      value_out: HyDbAmount(cb_value),
      inputs: vec![HyDbTxIn {
        vin_index: 0,
        prev_outpoint: None,
        coinbase: true,
        script_sig: vec![1, 2, 3],
        sequence: 0xffff_ffff,
      }],
      outputs: cb_outs,
      payload: None,
      validator_events: Vec::new(),
    }];

    // One spend per seed output of the previous block's coinbase.
    if height > 0 {
      for i in 1..=SPENDS {
        txs.push(HyDbTx {
          txid: spend_txid(height, i),
          block_pos: i as u32,
          version: 1,
          tx_type: HyDbTxType::Normal,
          lock_time: 0,
          size: 220,
          value_out: HyDbAmount(SPEND_OUT * 2),
          inputs: vec![HyDbTxIn {
            vin_index: 0,
            prev_outpoint: Some(HyDbOutpoint {
              txid: coinbase_txid(height - 1),
              vout: i as u32,
            }),
            coinbase: false,
            script_sig: vec![9, 9],
            sequence: 0xffff_ffff,
          }],
          outputs: vec![
            out_to(0, SPEND_OUT, pick(height, i, 0)),
            out_to(1, SPEND_OUT, pick(height, i, 1)),
          ],
          payload: None,
          validator_events: Vec::new(),
        });
      }
    }

    let tx_count = txs.len() as u32;
    Ok(HyDbBlock {
      header: HyDbHeader {
        hash,
        prev_hash: prev,
        merkle_root: hash,
        time: 1_600_000_000 + height * 150,
        bits: 0x1e0f_ffff,
        difficulty: 1.0,
        nonce: height,
        version: 1,
        size: 1_000,
        tx_count,
        chainwork: None,
        coinbase: None,
      },
      txs,
    })
  }
}

/// `(seq_scan, idx_scan, n_tup_upd, n_tup_ins)` for one table, as the server
/// reports them.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Counters {
  seq_scan: i64,
  idx_scan: i64,
  n_tup_upd: i64,
  n_tup_ins: i64,
}

impl Counters {
  /// Accesses of the table, however the planner chose to reach it. A singleton
  /// is small enough that Postgres may pick either its primary-key index or a
  /// sequential scan for `WHERE singleton`, and which one it picks is not the
  /// point: the count of accesses is.
  fn scans(self) -> i64 {
    self.seq_scan + self.idx_scan
  }
}

impl std::ops::Sub for Counters {
  type Output = Self;
  fn sub(self, rhs: Self) -> Self {
    Self {
      seq_scan: self.seq_scan - rhs.seq_scan,
      idx_scan: self.idx_scan - rhs.idx_scan,
      n_tup_upd: self.n_tup_upd - rhs.n_tup_upd,
      n_tup_ins: self.n_tup_ins - rhs.n_tup_ins,
    }
  }
}

/// Make the writer's cumulative statistics readable. Postgres flushes them
/// asynchronously — at most once a second per backend, and a backend that has
/// gone idle can sit on its pending counts for another ten — so a read straight
/// after a fast ingest sees a number that is merely *early*. Ending the
/// backends that did the writing is what makes it exact rather than eventual:
/// exit flushes pending statistics unconditionally. The writer is finished by
/// the time this is called, and the pools reconnect on demand.
async fn flush_writer_stats(pool: &PgPool) -> Result<(), Box<dyn std::error::Error>> {
  sqlx::query(
    r#"SELECT pg_terminate_backend(pid) FROM pg_stat_activity
        WHERE datname = current_database() AND pid <> pg_backend_pid()"#,
  )
  .execute(pool)
  .await?;
  Ok(())
}

/// Read the table counters, polling until two consecutive reads agree so the
/// measurement is repeatable even if a stray backend is still publishing.
async fn counters(pool: &PgPool, table: &str) -> Result<Counters, Box<dyn std::error::Error>> {
  let mut last: Option<Counters> = None;
  for _ in 0..24 {
    let row: (i64, i64, i64, i64) = sqlx::query_as(
      r#"SELECT COALESCE(seq_scan, 0), COALESCE(idx_scan, 0),
                COALESCE(n_tup_upd, 0), COALESCE(n_tup_ins, 0)
           FROM pg_stat_all_tables WHERE relname = $1"#,
    )
    .bind(table)
    .fetch_one(pool)
    .await?;
    let now = Counters {
      seq_scan: row.0,
      idx_scan: row.1,
      n_tup_upd: row.2,
      n_tup_ins: row.3,
    };
    if last == Some(now) {
      return Ok(now);
    }
    last = Some(now);
    sqlx::query("SELECT pg_sleep(0.05)").execute(pool).await?;
  }
  Ok(last.unwrap_or_default())
}

/// Buffer accesses per table, as `pg_statio_user_tables` reports them: `read`
/// is a physical page read (a miss that went to the operating system), `hit` is
/// a page found in `shared_buffers`. Partitions are folded back onto their
/// parent's name, because `address_txs_p0` and `address_txs_p1` are one table
/// as far as the write path is concerned.
///
/// This is the currency the write path is actually spending. A per-block count
/// of pages touched is what decides whether a 63 GB database with 8 GB of
/// `shared_buffers` spends its time in `IO:DataFileRead`, and unlike a wall
/// clock it does not move when the machine is busy with something else.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
struct Io {
  heap_read: i64,
  heap_hit: i64,
  idx_read: i64,
  idx_hit: i64,
}

impl Io {
  fn touched(self) -> i64 {
    self.heap_read + self.heap_hit + self.idx_read + self.idx_hit
  }
  fn read(self) -> i64 {
    self.heap_read + self.idx_read
  }
}

impl std::ops::Sub for Io {
  type Output = Self;
  fn sub(self, rhs: Self) -> Self {
    Self {
      heap_read: self.heap_read - rhs.heap_read,
      heap_hit: self.heap_hit - rhs.heap_hit,
      idx_read: self.idx_read - rhs.idx_read,
      idx_hit: self.idx_hit - rhs.idx_hit,
    }
  }
}

/// Every table's buffer accesses, keyed by parent name.
async fn io(pool: &PgPool) -> Result<BTreeMap<String, Io>, Box<dyn std::error::Error>> {
  let rows: Vec<(String, i64, i64, i64, i64)> = sqlx::query_as(
    r#"SELECT regexp_replace(relname, '_p[0-9]+$', '') AS parent,
              SUM(COALESCE(heap_blks_read, 0))::int8,
              SUM(COALESCE(heap_blks_hit, 0))::int8,
              SUM(COALESCE(idx_blks_read, 0))::int8,
              SUM(COALESCE(idx_blks_hit, 0))::int8
         FROM pg_statio_user_tables
        GROUP BY 1"#,
  )
  .fetch_all(pool)
  .await?;
  Ok(
    rows
      .into_iter()
      .map(|(t, hr, hh, ir, ih)| {
        (
          t,
          Io {
            heap_read: hr,
            heap_hit: hh,
            idx_read: ir,
            idx_hit: ih,
          },
        )
      })
      .collect(),
  )
}

#[tokio::test]
async fn a_block_costs_one_write_per_singleton_and_no_read() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  // The baseline is taken after the migrator's own writes have been published,
  // or the schema's `INSERT INTO supply_summary VALUES (TRUE)` lands inside the
  // measured window instead of before it.
  flush_writer_stats(&pool).await?;
  let before_supply = counters(&pool, "supply_summary").await?;
  let before_txs = counters(&pool, "transactions_summary").await?;
  let before_addr = counters(&pool, "address_summary").await?;
  let before_metrics = counters(&pool, "block_metrics").await?;
  let before_io = io(&pool).await?;

  let ingest = db.ingest(Arc::new(Chain)).await?;
  let started = Instant::now();
  for h in 0..BLOCKS {
    ingest.on_block(Chain::raw(h)).await?;
  }
  ingest
    .on_tip(HyDbTipTarget::AtNode(HyDbHash(block_hash(BLOCKS - 1))))
    .await?;
  let elapsed = started.elapsed();

  // The chain really was written, in full, before any budget is read.
  let (tip,): (Option<i32>,) = sqlx::query_as("SELECT MAX(height) FROM blocks")
    .fetch_one(&pool)
    .await?;
  assert_eq!(
    tip,
    Some(BLOCKS as i32 - 1),
    "the whole synthetic chain has to be connected before its cost means anything"
  );
  let (touches,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM address_txs")
    .fetch_one(&pool)
    .await?;
  flush_writer_stats(&pool).await?;

  // Pages touched per block, per table, biggest first. The write path's cost is
  // this list: `address_txs` leads it, and it leads it because it is the widest
  // fan-out in the database and carries two btrees over the same leading
  // column.
  let after_io = io(&pool).await?;
  let mut ranked: Vec<(String, Io)> = after_io
    .iter()
    .map(|(t, a)| {
      (
        t.clone(),
        *a - before_io.get(t).copied().unwrap_or_default(),
      )
    })
    .filter(|(_, d)| d.touched() > 0)
    .collect();
  ranked.sort_by_key(|(_, d)| -d.touched());
  eprintln!("  pages touched per block (read = physical, hit = shared_buffers):");
  for (table, d) in ranked.iter().take(16) {
    eprintln!(
      "    {table:24} {:8.2} touched  {:7.2} read  ({} heap_read, {} idx_read)",
      d.touched() as f64 / f64::from(BLOCKS),
      d.read() as f64 / f64::from(BLOCKS),
      d.heap_read,
      d.idx_read,
    );
  }

  let supply = counters(&pool, "supply_summary").await? - before_supply;
  let txs = counters(&pool, "transactions_summary").await? - before_txs;
  let addr = counters(&pool, "address_summary").await? - before_addr;
  let blocks = i64::from(BLOCKS);

  eprintln!(
    "{BLOCKS} blocks in {:.2}s ({:.2} ms/block), {touches} address_txs rows \
     ({:.1}/block)\n  supply_summary       {supply:?}\n  transactions_summary {txs:?}\n  \
     address_summary      {addr:?}",
    elapsed.as_secs_f64(),
    elapsed.as_secs_f64() * 1000.0 / f64::from(BLOCKS),
    touches as f64 / blocks as f64,
  );

  // Every block moves both singletons: a block always carries a coinbase, so
  // the transaction count and the subsidy both move. One update each is the
  // floor reversibility allows — the projection must be complete at every
  // height — and this asserts the write path is at that floor and not above it.
  assert_eq!(
    txs.n_tup_upd, blocks,
    "transactions_summary should take exactly one update per block"
  );
  assert_eq!(
    supply.n_tup_upd, blocks,
    "supply_summary should take exactly one update per block"
  );

  // And the accesses: one per block per singleton, which is the update's own.
  // Anything above that is a read the update could have returned itself. Before
  // A7 this was 4 accesses per block on supply_summary (three scalar subqueries
  // plus the update) and 2 on transactions_summary.
  assert_eq!(
    supply.scans(),
    blocks,
    "supply_summary should be touched once per block — the access its own UPDATE does"
  );
  assert_eq!(
    txs.scans(),
    blocks,
    "transactions_summary should be touched once per block"
  );

  // C1's singleton is already at the floor and must stay there: it is written
  // only when an address crosses zero. Every block here creates first-seen
  // addresses, so it moves every block, but it must never be *read* separately.
  assert!(
    addr.scans() <= blocks,
    "address_summary should not be read separately from its own update ({addr:?})"
  );

  // block_metrics accumulates onto the prior height without a separate read:
  // the fold is an INSERT ... SELECT, so the only access is the insert's own
  // index lookup of height - 1. One insert per block, no extra statement.
  let metrics = counters(&pool, "block_metrics").await? - before_metrics;
  assert_eq!(metrics.n_tup_ins, blocks, "one block_metrics row per block");
  assert!(
    metrics.scans() <= blocks,
    "the cumulative fold should touch the prior row once, inside the insert ({metrics:?})"
  );

  // The fan-out is the shape this file claims to measure. If a change to the
  // generator quietly stopped producing mainnet-tail-sized address batches, the
  // numbers above would still pass while measuring nothing.
  let per_block = touches as f64 / blocks as f64;
  assert!(
    per_block > 100.0,
    "the synthetic chain should touch mainnet-tail-many addresses per block, got {per_block:.1}"
  );
  Ok(())
}

// ---------------------------------------------------------------------------
// The arm that skips the write
// ---------------------------------------------------------------------------

/// A chain whose last block carries no transactions at all, so nothing moves
/// either singleton. Unreachable on a chain with a coinbase, and reachable here
/// on purpose: the stats stage now has an arm that writes no `UPDATE` when a
/// block moves nothing, and an arm no test enters is an arm that can rot. It
/// still has to write the `undo_blocks` pre-image — a height with no pre-image
/// is a height that cannot be disconnected — so this asserts both halves: the
/// singletons do not move, and the block reverses.
struct EmptyTail;

impl EmptyTail {
  /// Heights 0..=1 carry a coinbase; height 2 carries nothing.
  fn empty_at() -> u32 {
    2
  }
}

impl HyDbDataSrc for EmptyTail {
  fn identify(&self, raw: &[u8]) -> HyDbResult<HyDbBlockId> {
    Ok(HyDbBlockId {
      hash: HyDbHash::from_slice(&raw[..32])?,
      prev_hash: HyDbHash::from_slice(&raw[32..64])?,
    })
  }

  fn decode(&self, raw: &[u8]) -> HyDbResult<HyDbBlock> {
    let hash = HyDbHash::from_slice(&raw[..32])?;
    let prev = HyDbHash::from_slice(&raw[32..64])?;
    let height = Chain::height_of(&hash.0);
    let txs = if height >= Self::empty_at() {
      Vec::new()
    } else {
      vec![HyDbTx {
        txid: coinbase_txid(height),
        block_pos: 0,
        version: 1,
        tx_type: HyDbTxType::Coinbase,
        lock_time: 0,
        size: 250,
        value_out: HyDbAmount(COINBASE_VALUE),
        inputs: vec![HyDbTxIn {
          vin_index: 0,
          prev_outpoint: None,
          coinbase: true,
          script_sig: vec![1],
          sequence: 0xffff_ffff,
        }],
        outputs: vec![out_to(0, COINBASE_VALUE, pick(height, 0, 0))],
        payload: None,
        validator_events: Vec::new(),
      }]
    };
    let tx_count = txs.len() as u32;
    Ok(HyDbBlock {
      header: HyDbHeader {
        hash,
        prev_hash: prev,
        merkle_root: hash,
        time: 1_600_000_000 + height * 150,
        bits: 0x1e0f_ffff,
        difficulty: 1.0,
        nonce: height,
        version: 1,
        size: 500,
        tx_count,
        chainwork: None,
        coinbase: None,
      },
      txs,
    })
  }
}

/// The two singletons, as one tuple.
async fn summaries(pool: &PgPool) -> Result<(i64, i64, i64, i64), Box<dyn std::error::Error>> {
  Ok(
    sqlx::query_as(
      r#"SELECT (SELECT total FROM transactions_summary WHERE singleton),
                (SELECT total FROM supply_summary WHERE singleton),
                (SELECT included_supply FROM supply_summary WHERE singleton),
                (SELECT unspendable_value FROM supply_summary WHERE singleton)"#,
    )
    .fetch_one(pool)
    .await?,
  )
}

#[tokio::test]
async fn a_block_that_moves_nothing_writes_no_update_and_still_reverses() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = empty_tail(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn empty_tail(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;
  let ingest = db.ingest(Arc::new(EmptyTail)).await?;

  for h in 0..EmptyTail::empty_at() {
    ingest.on_block(Chain::raw(h)).await?;
  }
  ingest
    .on_tip(HyDbTipTarget::AtNode(HyDbHash(block_hash(
      EmptyTail::empty_at() - 1,
    ))))
    .await?;
  let before = summaries(&pool).await?;
  flush_writer_stats(&pool).await?;
  let upd_before = (
    counters(&pool, "transactions_summary").await?.n_tup_upd,
    counters(&pool, "supply_summary").await?.n_tup_upd,
  );

  // The block that moves nothing.
  let empty = EmptyTail::empty_at();
  ingest.on_block(Chain::raw(empty)).await?;
  ingest
    .on_tip(HyDbTipTarget::AtNode(HyDbHash(block_hash(empty))))
    .await?;
  let (tip,): (Option<i32>,) = sqlx::query_as("SELECT MAX(height) FROM blocks")
    .fetch_one(&pool)
    .await?;
  assert_eq!(
    tip,
    Some(empty as i32),
    "a transactionless block still has to connect"
  );
  assert_eq!(
    summaries(&pool).await?,
    before,
    "a block that moves nothing must leave both singletons exactly as they were"
  );

  flush_writer_stats(&pool).await?;
  let upd_after = (
    counters(&pool, "transactions_summary").await?.n_tup_upd,
    counters(&pool, "supply_summary").await?.n_tup_upd,
  );
  assert_eq!(
    upd_after, upd_before,
    "and it must leave no dead tuple behind either: no UPDATE at all"
  );

  // The pre-image is still recorded, and it is what a disconnect restores.
  let (undo,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM undo_blocks WHERE height = $1")
    .bind(empty as i32)
    .fetch_one(&pool)
    .await?;
  assert_eq!(
    undo, 1,
    "a height with no undo row is a height that cannot be disconnected"
  );

  ingest.rewind_to(HyDbHeight::new(empty - 1)).await?;
  assert_eq!(
    summaries(&pool).await?,
    before,
    "and the rewind restores the same values it captured"
  );
  let (tip,): (Option<i32>,) = sqlx::query_as("SELECT MAX(height) FROM blocks")
    .fetch_one(&pool)
    .await?;
  assert_eq!(
    tip,
    Some(empty as i32 - 1),
    "the rewind took the block away"
  );
  Ok(())
}
