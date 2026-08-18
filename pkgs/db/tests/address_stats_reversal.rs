//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Exact reversal of `address_stats`, column by column.
//!
//! The balance stage no longer stores a pre-image of the additive columns. It
//! recomputes the block's per-address delta from the block's own base facts
//! (`tx_outputs` at this height, `spends` at this height joined back to
//! `tx_outputs`) and subtracts it, keeping stored pre-images only for
//! `existed` and `last_seen_height`. That trades a stored fact for a derived
//! one, and a derived fact can be *nearly* right: off by an output paid to the
//! same address twice, by an output created and spent inside one block, by a
//! second transaction touching the same address, by an addressless output
//! counted as if it had one. None of those show up as a crash — they show up
//! as an `address_stats` row that is a few duffs or one `utxo_count` away from
//! where it was, and stay wrong forever.
//!
//! So this file asserts identity, not plausibility: it snapshots every column
//! of every row, reorgs across the blocks that exercise each of those cases,
//! re-connects the original branch, and requires the table back bit for bit —
//! naming the addresses and the columns that drifted when it is not. The same
//! is asserted of the two projections that hang off the same disconnect,
//! `address_rich` and `address_summary.positive_balance_count`, and of a plain
//! `rewind_to` as well as a branch switch, because the two reach the
//! disconnect path by different routes.
//!
//! The chain below is built so that every leg of the recomputation is
//! load-bearing somewhere:
//!
//! * `SPLITTER` is paid by two outputs of one transaction (the `tx_count`
//!   delta must be 1 while the `utxo_count` delta is 2).
//! * `DOUBLE` is touched by two different transactions in one block (2 and 2).
//! * `ROUNDTRIP` spends and is repaid by the same transaction, netting zero.
//! * `EPHEMERAL` receives an output that a later transaction *in the same
//!   block* spends, so the outpoint is both created and consumed at one
//!   height.
//! * An addressless output carries value at a height where addressed outputs
//!   also exist, so a recomputation that forgot `address IS NOT NULL` would
//!   fold it into somebody's balance.
//! * `MINER` is touched at two heights with a gap, so `last_seen_height` has
//!   somewhere to go back to that is neither the current height nor
//!   `first_seen_height`.
//! * `WHALE`, `HEIR`, `DRAINED` and `RICHLING` cross the rich-list floor and
//!   zero in both directions.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use common::TempDb;
use hyphen_db::layout::rich_list_floor;
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHash, HyDbHeader,
  HyDbHeight, HyDbOutpoint, HyDbResult, HyDbTx, HyDbTxIn, HyDbTxOut, HyDbTxType,
};
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const MINER: &str = "yRevMinerA0000000000000000000000001";
const SPLITTER: &str = "yRevSplitA0000000000000000000000002";
const DOUBLE: &str = "yRevDoubleA000000000000000000000003";
const ROUNDTRIP: &str = "yRevRoundA0000000000000000000000004";
const EPHEMERAL: &str = "yRevEphemA0000000000000000000000005";
const WHALE: &str = "yRevWhaleA0000000000000000000000006";
const HEIR: &str = "yRevHeirAA0000000000000000000000007";
const DRAINED: &str = "yRevDrainA0000000000000000000000008";
const RICHLING: &str = "yRevRichlA0000000000000000000000009";
const FORK_A: &str = "yRevForkAA000000000000000000000010";
const FORK_B: &str = "yRevForkBB000000000000000000000011";

/// Every column of `address_stats`. The reversal test compares each one by
/// name, so a column added to the table without being added here would be
/// silently exempt from the comparison; [`assert_columns`] closes that door.
const STATS_COLUMNS: [&str; 7] = [
  "address",
  "balance",
  "total_received",
  "tx_count",
  "utxo_count",
  "first_seen_height",
  "last_seen_height",
];

// ===========================================================================
// A small block builder: blocks of arbitrary transactions
// ===========================================================================

/// One output: an optional address (None = addressless, e.g. OP_RETURN) and a
/// value.
type Out = (Option<&'static str>, i64);

#[derive(Clone)]
struct TxSpec {
  /// Distinguishes this transaction's txid from its siblings'.
  tag: u8,
  /// Consumed outpoints; empty means this is the coinbase.
  inputs: Vec<HyDbOutpoint>,
  outputs: Vec<Out>,
}

#[derive(Clone, Default)]
struct BlockSpec {
  txs: Vec<TxSpec>,
}

#[derive(Default)]
struct Src {
  blocks: HashMap<[u8; 32], BlockSpec>,
}

fn h(n: u8) -> [u8; 32] {
  [n; 32]
}

/// A transaction's id: the block hash with its low byte replaced by the tag,
/// so ids are unique across the chain and stable across re-connects.
fn txid_of(block: u8, tag: u8) -> HyDbHash {
  let mut t = [block; 32];
  t[0] = tag;
  HyDbHash(t)
}

/// Outpoint `vout` of the transaction tagged `tag` in block `block`.
fn op(block: u8, tag: u8, vout: u32) -> HyDbOutpoint {
  HyDbOutpoint {
    txid: txid_of(block, tag),
    vout,
  }
}

impl Src {
  fn block(&mut self, hash: u8, txs: Vec<TxSpec>) {
    self.blocks.insert(h(hash), BlockSpec { txs });
  }
}

fn cb(outputs: Vec<Out>) -> TxSpec {
  TxSpec {
    tag: 0,
    inputs: Vec::new(),
    outputs,
  }
}

fn tx(tag: u8, inputs: Vec<HyDbOutpoint>, outputs: Vec<Out>) -> TxSpec {
  TxSpec {
    tag,
    inputs,
    outputs,
  }
}

impl HyDbDataSrc for Src {
  fn identify(&self, raw: &[u8]) -> HyDbResult<HyDbBlockId> {
    Ok(HyDbBlockId {
      hash: HyDbHash::from_slice(&raw[..32])?,
      prev_hash: HyDbHash::from_slice(&raw[32..64])?,
    })
  }

  fn decode(&self, raw: &[u8]) -> HyDbResult<HyDbBlock> {
    let hash = HyDbHash::from_slice(&raw[..32])?;
    let prev = HyDbHash::from_slice(&raw[32..64])?;
    let spec = self
      .blocks
      .get(&hash.0)
      .ok_or_else(|| hyphen_db::HyDbError::Derive("no spec for block".into()))?;

    let mut txs = Vec::with_capacity(spec.txs.len());
    for (pos, t) in spec.txs.iter().enumerate() {
      let value_out: i64 = t.outputs.iter().map(|(_, v)| *v).sum();
      let coinbase = t.inputs.is_empty();
      let inputs = if coinbase {
        vec![HyDbTxIn {
          vin_index: 0,
          prev_outpoint: None,
          coinbase: true,
          script_sig: vec![1],
          sequence: 0xffff_ffff,
        }]
      } else {
        t.inputs
          .iter()
          .enumerate()
          .map(|(i, prev)| HyDbTxIn {
            vin_index: i as u32,
            prev_outpoint: Some(*prev),
            coinbase: false,
            script_sig: vec![2],
            sequence: 0xffff_ffff,
          })
          .collect()
      };
      let outputs = t
        .outputs
        .iter()
        .enumerate()
        .map(|(i, (addr, value))| HyDbTxOut {
          vout: i as u32,
          value: HyDbAmount(*value),
          script_pubkey: vec![0x76, 0xa9, 0x14],
          address: addr.map(|a| HyDbAddress(a.to_owned())),
          address_type: match addr {
            Some(_) => HyDbAddrType::P2pkh,
            None => HyDbAddrType::Unknown,
          },
        })
        .collect();
      txs.push(HyDbTx {
        txid: txid_of(hash.0[31], t.tag),
        block_pos: pos as u32,
        version: 1,
        tx_type: if coinbase {
          HyDbTxType::Coinbase
        } else {
          HyDbTxType::Normal
        },
        lock_time: 0,
        size: 200,
        value_out: HyDbAmount(value_out),
        inputs,
        outputs,
        payload: None,
        validator_events: Vec::new(),
      });
    }
    let tx_count = txs.len() as u32;
    Ok(HyDbBlock {
      header: HyDbHeader {
        hash,
        prev_hash: prev,
        merkle_root: hash,
        time: 1_600_000_000,
        bits: 0x1e0f_ffff,
        difficulty: 1.0,
        nonce: 0,
        version: 1,
        size: 285,
        tx_count,
        chainwork: None,
        coinbase: None,
      },
      txs,
    })
  }
}

fn raw(hash: u8, prev: u8) -> Bytes {
  let mut v = Vec::with_capacity(64);
  v.extend_from_slice(&h(hash));
  v.extend_from_slice(&h(prev));
  Bytes::from(v)
}

// ===========================================================================
// Snapshot and comparison
// ===========================================================================

/// `address_stats` must have exactly the columns [`STATS_COLUMNS`] names.
/// Without this, a column added to the table (and to the ingest path, and to
/// nothing else) would slip past every comparison below unnoticed.
async fn assert_columns(pool: &PgPool) -> TestResult {
  let found: Vec<(String,)> = sqlx::query_as(
    "SELECT column_name FROM information_schema.columns
      WHERE table_name = 'address_stats' ORDER BY ordinal_position",
  )
  .fetch_all(pool)
  .await?;
  let found: Vec<String> = found.into_iter().map(|(c,)| c).collect();
  assert_eq!(
    found,
    STATS_COLUMNS.map(|c| c.to_owned()).to_vec(),
    "address_stats gained or lost a column; the reversal comparison below \
     names its columns explicitly and must be updated with the table"
  );
  Ok(())
}

/// Copy `address_stats` and `address_rich` into snapshot tables tagged `name`,
/// alongside the positive-balance counter.
async fn snapshot(pool: &PgPool, name: &str) -> Result<i64, Box<dyn std::error::Error>> {
  for table in ["address_stats", "address_rich"] {
    sqlx::query(&format!(
      "CREATE TABLE snap_{name}_{table} AS SELECT * FROM {table}"
    ))
    .execute(pool)
    .await?;
  }
  let (n,): (i64,) =
    sqlx::query_as("SELECT positive_balance_count FROM address_summary WHERE singleton")
      .fetch_one(pool)
      .await?;
  Ok(n)
}

/// Every difference between `address_stats` now and the snapshot `name`, one
/// line per address, naming the columns that differ. A FULL JOIN so a row
/// present on one side only is reported as such rather than skipped.
///
/// `ignore` is for addresses that legitimately exist on only one of the two
/// states being compared (the alternative branch's own rows).
async fn stats_drift(
  pool: &PgPool,
  name: &str,
  ignore: &[&str],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
  let ignore: Vec<String> = ignore.iter().map(|a| (*a).to_owned()).collect();
  let rows: Vec<(String,)> = sqlx::query_as(&format!(
    r#"SELECT COALESCE(a.address, b.address)
              || CASE WHEN a.address IS NULL THEN ': missing now, snapshot had '
                      WHEN b.address IS NULL THEN ': present now, absent from the snapshot: '
                      ELSE ': ' END
              || CASE WHEN a.address IS NULL OR b.address IS NULL THEN
                        COALESCE(a::text, b::text)
                 ELSE
                   concat_ws(', ',
                     CASE WHEN a.balance IS DISTINCT FROM b.balance
                          THEN format('balance %s <> %s', a.balance, b.balance) END,
                     CASE WHEN a.total_received IS DISTINCT FROM b.total_received
                          THEN format('total_received %s <> %s',
                                      a.total_received, b.total_received) END,
                     CASE WHEN a.tx_count IS DISTINCT FROM b.tx_count
                          THEN format('tx_count %s <> %s', a.tx_count, b.tx_count) END,
                     CASE WHEN a.utxo_count IS DISTINCT FROM b.utxo_count
                          THEN format('utxo_count %s <> %s', a.utxo_count, b.utxo_count) END,
                     CASE WHEN a.first_seen_height IS DISTINCT FROM b.first_seen_height
                          THEN format('first_seen_height %s <> %s',
                                      a.first_seen_height, b.first_seen_height) END,
                     CASE WHEN a.last_seen_height IS DISTINCT FROM b.last_seen_height
                          THEN format('last_seen_height %s <> %s',
                                      a.last_seen_height, b.last_seen_height) END)
                 END
       FROM address_stats a
       FULL JOIN snap_{name}_address_stats b USING (address)
      WHERE COALESCE(a.address, b.address) <> ALL($1)
        AND (a.address IS NULL OR b.address IS NULL
             OR a.balance           IS DISTINCT FROM b.balance
             OR a.total_received    IS DISTINCT FROM b.total_received
             OR a.tx_count          IS DISTINCT FROM b.tx_count
             OR a.utxo_count        IS DISTINCT FROM b.utxo_count
             OR a.first_seen_height IS DISTINCT FROM b.first_seen_height
             OR a.last_seen_height  IS DISTINCT FROM b.last_seen_height)
      ORDER BY 1"#
  ))
  .bind(&ignore)
  .fetch_all(pool)
  .await?;
  Ok(rows.into_iter().map(|(r,)| r).collect())
}

/// The same for `address_rich`, which has only a balance to drift.
async fn rich_drift(
  pool: &PgPool,
  name: &str,
  ignore: &[&str],
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
  let ignore: Vec<String> = ignore.iter().map(|a| (*a).to_owned()).collect();
  let rows: Vec<(String,)> = sqlx::query_as(&format!(
    r#"SELECT format('%s: now %s, snapshot %s',
                     COALESCE(a.address, b.address), a.balance, b.balance)
         FROM address_rich a
         FULL JOIN snap_{name}_address_rich b USING (address)
        WHERE COALESCE(a.address, b.address) <> ALL($1)
          AND a.balance IS DISTINCT FROM b.balance
        ORDER BY 1"#
  ))
  .bind(&ignore)
  .fetch_all(pool)
  .await?;
  Ok(rows.into_iter().map(|(r,)| r).collect())
}

/// `address_rich` is exactly the set of addresses at or above the floor, and
/// the maintained positive-balance count is exactly the number of positive
/// rows. Both are re-derived from `address_stats`, the table the reversal has
/// just rewritten, so they catch a projection that survived a reorg its source
/// did not.
async fn assert_projections_agree(pool: &PgPool, at: &str) -> TestResult {
  let orphans: Vec<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
    "SELECT COALESCE(r.address, s.address), r.balance, s.balance
       FROM address_rich r
       FULL JOIN (SELECT address, balance FROM address_stats WHERE balance >= $1) s
         USING (address)
      WHERE r.balance IS DISTINCT FROM s.balance
      ORDER BY 1",
  )
  .bind(rich_list_floor())
  .fetch_all(pool)
  .await?;
  assert!(
    orphans.is_empty(),
    "address_rich disagrees with address_stats at {at}: {orphans:?}"
  );

  let (counter,): (i64,) =
    sqlx::query_as("SELECT positive_balance_count FROM address_summary WHERE singleton")
      .fetch_one(pool)
      .await?;
  let (truth,): (i64,) = sqlx::query_as("SELECT COUNT(*) FROM address_stats WHERE balance > 0")
    .fetch_one(pool)
    .await?;
  assert_eq!(
    counter, truth,
    "positive_balance_count drifted at {at}: counter {counter}, table {truth}"
  );
  Ok(())
}

/// Assert `address_stats`, `address_rich` and the counter all match snapshot
/// `name` exactly.
async fn assert_identical(
  pool: &PgPool,
  name: &str,
  counter_then: i64,
  ignore: &[&str],
  at: &str,
) -> TestResult {
  let drift = stats_drift(pool, name, ignore).await?;
  assert!(
    drift.is_empty(),
    "address_stats is not bit-for-bit back to the {name} snapshot at {at}:\n  {}",
    drift.join("\n  ")
  );
  let drift = rich_drift(pool, name, ignore).await?;
  assert!(
    drift.is_empty(),
    "address_rich is not back to the {name} snapshot at {at}:\n  {}",
    drift.join("\n  ")
  );
  if ignore.is_empty() {
    let (counter,): (i64,) =
      sqlx::query_as("SELECT positive_balance_count FROM address_summary WHERE singleton")
        .fetch_one(pool)
        .await?;
    assert_eq!(
      counter, counter_then,
      "positive_balance_count is not back to its {name} value at {at}"
    );
  }
  assert_projections_agree(pool, at).await
}

// ===========================================================================
// The chain
// ===========================================================================

fn chain() -> Src {
  let f = rich_list_floor();
  let mut src = Src::default();

  // h0: the miner is paid, and the whale is funded above the floor.
  src.block(
    1,
    vec![cb(vec![(Some(MINER), 5_000), (Some(WHALE), f + 500)])],
  );

  // h1: two addresses funded, one of them by two outputs of a single
  // transaction (tx_count +1, utxo_count +2).
  src.block(
    2,
    vec![
      cb(vec![(Some(DRAINED), 900)]),
      tx(
        1,
        vec![op(1, 0, 0)],
        vec![
          (Some(SPLITTER), 2_000),
          (Some(SPLITTER), 3_000),
          (Some(ROUNDTRIP), 0),
        ],
      ),
    ],
  );
  // Note: MINER's 5,000 pays the two SPLITTER outputs; ROUNDTRIP's zero-value
  // output exists only so ROUNDTRIP has an outpoint to spend later.

  // h2: nothing this test's addresses touch, so `last_seen_height` has a gap
  // to fall back across.
  src.block(3, vec![cb(vec![(Some(RICHLING), 100)])]);

  // h3: the block every recomputation leg has to survive.
  //   tx1  DOUBLE is paid (first of two transactions touching it here)
  //   tx2  DOUBLE is paid again by a different transaction (tx_count +2)
  //   tx3  ROUNDTRIP spends its own output and is repaid by the same tx
  //   tx4  the whale's coinbase output is split: part stays, part goes to the
  //        heir (a new row above the floor), and part goes nowhere at all —
  //        an addressless output carrying value.
  src.block(
    4,
    vec![
      cb(vec![(Some(MINER), 4_000)]),
      tx(1, vec![op(2, 1, 0)], vec![(Some(DOUBLE), 2_000)]),
      tx(2, vec![op(2, 1, 1)], vec![(Some(DOUBLE), 3_000)]),
      tx(3, vec![op(2, 1, 2)], vec![(Some(ROUNDTRIP), 0)]),
      tx(
        4,
        vec![op(1, 0, 1)],
        vec![
          (Some(WHALE), 200),
          (Some(HEIR), f),
          (None, 300),
          (Some(EPHEMERAL), 0),
        ],
      ),
    ],
  );

  // h4: the drained address goes to exactly zero, the richling crosses the
  // floor upward, and an output created in this block is spent in this block.
  src.block(
    5,
    vec![
      cb(vec![(Some(RICHLING), f)]),
      tx(1, vec![op(2, 0, 0)], vec![(Some(EPHEMERAL), 900)]),
      tx(2, vec![op(5, 1, 0)], vec![(Some(MINER), 900)]),
    ],
  );

  // The alternative branch off h2, two blocks long.
  src.block(0x14, vec![cb(vec![(Some(FORK_A), 7_000)])]);
  src.block(0x15, vec![cb(vec![(Some(FORK_B), 8_000)])]);
  src
}

#[tokio::test]
async fn address_stats_reverses_bit_for_bit() -> TestResult {
  let Some(tmp) = TempDb::create().await? else {
    return Ok(());
  };
  let result = run(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;
  assert_columns(&pool).await?;

  let ingest = db.ingest(Arc::new(chain())).await?;
  for (hash, prev) in [(1u8, 0u8), (2, 1), (3, 2)] {
    ingest.on_block(raw(hash, prev)).await?;
  }
  ingest.on_tip(HyDbHash(h(3))).await?;
  assert_projections_agree(&pool, "h2").await?;
  let counter_h2 = snapshot(&pool, "h2").await?;

  // The two blocks that exercise every leg of the recomputation.
  ingest.on_block(raw(4, 3)).await?;
  ingest.on_block(raw(5, 4)).await?;
  ingest.on_tip(HyDbHash(h(5))).await?;
  assert_projections_agree(&pool, "h4").await?;
  let counter_h4 = snapshot(&pool, "h4").await?;

  // The state at h4 is only worth comparing against if the blocks actually
  // moved these rows the way the header claims. Spot-check the derived
  // columns that a naive recomputation would get wrong.
  let stats = |addr: &'static str| {
    let pool = pool.clone();
    async move {
      sqlx::query_as::<_, (i64, i64, i32, i32, Option<i32>, Option<i32>)>(
        "SELECT balance, total_received, tx_count, utxo_count, first_seen_height,
                last_seen_height FROM address_stats WHERE address = $1",
      )
      .bind(addr)
      .fetch_one(&pool)
      .await
    }
  };
  assert_eq!(
    stats(SPLITTER).await?,
    (0, 5_000, 3, 0, Some(1), Some(3)),
    "two outputs of one transaction (one tx, two utxos), spent by two"
  );
  assert_eq!(
    stats(DOUBLE).await?,
    (5_000, 5_000, 2, 2, Some(3), Some(3)),
    "two transactions in one block: two txs"
  );
  assert_eq!(
    stats(ROUNDTRIP).await?,
    (0, 0, 2, 1, Some(1), Some(3)),
    "spent and repaid by one transaction: net zero, one tx at that height"
  );
  assert_eq!(
    stats(EPHEMERAL).await?,
    (0, 900, 3, 1, Some(3), Some(4)),
    "an output created and spent in the same block nets out"
  );
  assert_eq!(
    stats(MINER).await?,
    (4_900, 9_900, 4, 2, Some(0), Some(4)),
    "touched at heights 0, 1, 3 and 4"
  );
  assert_eq!(
    stats(DRAINED).await?,
    (0, 900, 2, 0, Some(1), Some(4)),
    "emptied to exactly zero"
  );

  // ---- Route one: a plain rewind and replay ------------------------------
  ingest.rewind_to(HyDbHeight(2)).await?;
  assert_identical(&pool, "h2", counter_h2, &[], "after rewind_to(2)").await?;
  ingest.on_tip(HyDbHash(h(5))).await?;
  assert_identical(&pool, "h4", counter_h4, &[], "after replaying to h4").await?;

  // ---- Route two: a branch switch and back -------------------------------
  ingest.on_block(raw(0x14, 3)).await?;
  ingest.on_block(raw(0x15, 0x14)).await?;
  let report = ingest.on_tip(HyDbHash(h(0x15))).await?;
  let reorg = report
    .reorg
    .ok_or("switching branches must report a reorg")?;
  assert_eq!(reorg.disconnected.len(), 2, "heights 3 and 4 disconnected");
  // Everything except the branch's own two addresses must be exactly the h2
  // state again — the disconnect ran inside the same transaction that
  // connected the replacement blocks.
  assert_identical(
    &pool,
    "h2",
    counter_h2,
    &[FORK_A, FORK_B],
    "after switching branches",
  )
  .await?;
  let survivors: i64 =
    sqlx::query_scalar("SELECT COUNT(*) FROM address_stats WHERE address = ANY($1)")
      .bind(vec![HEIR.to_owned(), DOUBLE.to_owned()])
      .fetch_one(&pool)
      .await?;
  assert_eq!(
    survivors, 0,
    "rows the disconnected blocks created must be gone, not zeroed"
  );

  // And back to the original branch: this is the assertion the whole file is
  // for — every column of every row identical to before the reorg.
  ingest.on_tip(HyDbHash(h(5))).await?;
  assert_identical(&pool, "h4", counter_h4, &[], "after re-connecting h3/h4").await?;

  // ---- The one thing reversal cannot recover from -----------------------
  // Undo rows below the retention window are gone, and a block whose undo
  // rows are gone cannot be reversed: the deltas are recomputable but
  // `existed` and `last_seen_height` are not. The reorg bound is supposed to
  // make that unreachable; simulate it reaching anyway and require a refusal
  // rather than a disconnect that quietly leaves the block's balances in
  // place. (Destructive, so it runs last.)
  sqlx::query("DELETE FROM undo_address_stats WHERE height = 4")
    .execute(&pool)
    .await?;
  let refusal = ingest
    .rewind_to(HyDbHeight(3))
    .await
    .err()
    .map(|e| e.to_string());
  assert!(
    refusal
      .as_deref()
      .is_some_and(|e| e.contains("cannot be reversed")),
    "a block with no undo rows must be refused, naming the cause; got {refusal:?}"
  );

  Ok(())
}
