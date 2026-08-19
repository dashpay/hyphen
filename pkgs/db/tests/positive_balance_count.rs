//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! `address_summary.positive_balance_count` maintenance across connect and
//! disconnect. `address_stats` carries no index over `balance` — that index
//! made every ingest update non-HOT — so the count of positive-balance
//! addresses cannot be a `COUNT(*)` at read time; it is accumulated inside the
//! block transaction from the same prior/new balance pair the balance stage
//! already holds.
//!
//! An accumulated count has exactly one failure mode worth testing: drift. It
//! is invisible by construction — the counter always returns *a* number — so
//! every checkpoint here re-derives the truth with the very `COUNT(*)` the
//! counter replaces and asserts equality, naming the addresses that make up
//! the true count when it fails. The asymmetric case is the one that matters:
//! a connect path that counts crossings correctly while the disconnect path
//! forgets is green until the first reorg and wrong forever after, so the
//! chain below crosses zero in both directions, reorgs across the crossings,
//! and then re-connects the original branch.
//!
//! Every value here sits far below `layout::rich_list_floor()`, so nothing this
//! test asserts depends on the rich-list side table.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use common::TempDb;
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHash, HyDbHeader,
  HyDbOutpoint, HyDbResult, HyDbTipTarget, HyDbTx, HyDbTxIn, HyDbTxOut, HyDbTxType,
};
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Funded first, spent out entirely later: the downward zero crossing that a
/// disconnect has to undo.
const SPENDER: &str = "yPosSpendAA00000000000000000000000A";
/// Funded once and never touched again: a row that must be counted exactly
/// once, however many blocks come and go.
const HOLDER: &str = "yPosHoldAAA00000000000000000000000B";
/// Funded twice: the second block must not count it a second time.
const TOPUP: &str = "yPosTopUpAA00000000000000000000000C";
/// First seen on the block that also empties the spender.
const HEIR: &str = "yPosHeirAAA00000000000000000000000D";
/// Coinbase recipient of the block that empties the spender.
const MINER: &str = "yPosMinerAA00000000000000000000000E";
/// Only ever on the alternative branch: present after the reorg, gone again
/// when the original branch is re-connected.
const FORK_A: &str = "yPosForkAAA00000000000000000000000F";
const FORK_B: &str = "yPosForkBBB00000000000000000000000G";

fn h(n: u8) -> [u8; 32] {
  [n; 32]
}

#[derive(Clone)]
struct Spend {
  prev: HyDbOutpoint,
  to: &'static str,
  value: i64,
}

#[derive(Clone)]
struct Spec {
  cb_to: &'static str,
  cb_value: i64,
  spend: Option<Spend>,
}

#[derive(Default)]
struct Src {
  specs: HashMap<[u8; 32], Spec>,
}

fn out(to: &str, value: i64) -> HyDbTxOut {
  HyDbTxOut {
    vout: 0,
    value: HyDbAmount(value),
    script_pubkey: vec![0x76, 0xa9, 0x14],
    address: Some(HyDbAddress(to.to_owned())),
    address_type: HyDbAddrType::P2pkh,
  }
}

/// The spend tx's txid: the block hash with its first byte flipped, so it
/// never collides with the coinbase txid (== block hash).
fn spend_txid(block: [u8; 32]) -> HyDbHash {
  let mut t = block;
  t[0] ^= 0xff;
  HyDbHash(t)
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
      .specs
      .get(&hash.0)
      .ok_or_else(|| hyphen_db::HyDbError::Derive("no spec for block".into()))?;

    let mut txs = vec![HyDbTx {
      txid: hash,
      block_pos: 0,
      version: 1,
      tx_type: HyDbTxType::Coinbase,
      lock_time: 0,
      size: 200,
      value_out: HyDbAmount(spec.cb_value),
      inputs: vec![HyDbTxIn {
        vin_index: 0,
        prev_outpoint: None,
        coinbase: true,
        script_sig: vec![1],
        sequence: 0xffff_ffff,
      }],
      outputs: vec![out(spec.cb_to, spec.cb_value)],
      payload: None,
      validator_events: Vec::new(),
    }];
    if let Some(sp) = &spec.spend {
      txs.push(HyDbTx {
        txid: spend_txid(hash.0),
        block_pos: 1,
        version: 1,
        tx_type: HyDbTxType::Normal,
        lock_time: 0,
        size: 200,
        value_out: HyDbAmount(sp.value),
        inputs: vec![HyDbTxIn {
          vin_index: 0,
          prev_outpoint: Some(sp.prev),
          coinbase: false,
          script_sig: vec![2],
          sequence: 0xffff_ffff,
        }],
        outputs: vec![out(sp.to, sp.value)],
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

/// The maintained counter.
async fn counter(pool: &PgPool) -> Result<i64, Box<dyn std::error::Error>> {
  let (n,): (i64,) =
    sqlx::query_as("SELECT positive_balance_count FROM address_summary WHERE singleton = TRUE")
      .fetch_one(pool)
      .await?;
  Ok(n)
}

/// The truth the counter stands in for, as the set of addresses that make it
/// up, so a mismatch reports *which* rows the counter disagrees about rather
/// than just two integers.
async fn truth(pool: &PgPool) -> Result<Vec<(String, i64)>, Box<dyn std::error::Error>> {
  Ok(
    sqlx::query_as("SELECT address, balance FROM address_stats WHERE balance > 0 ORDER BY address")
      .fetch_all(pool)
      .await?,
  )
}

/// The invariant: `address_summary.positive_balance_count` equals
/// `COUNT(*) FROM address_stats WHERE balance > 0`, exactly, at this block
/// boundary.
async fn assert_exact(pool: &PgPool, at: &str) -> TestResult {
  let maintained = counter(pool).await?;
  let rows = truth(pool).await?;
  assert_eq!(
    maintained,
    rows.len() as i64,
    "positive_balance_count drifted at {at}: counter says {maintained}, \
     address_stats holds {} positive rows {rows:?}",
    rows.len()
  );
  Ok(())
}

/// The addresses currently holding a positive balance.
async fn positive(pool: &PgPool) -> Result<Vec<String>, Box<dyn std::error::Error>> {
  Ok(truth(pool).await?.into_iter().map(|(a, _)| a).collect())
}

#[tokio::test]
async fn positive_balance_count_is_exact_in_both_directions() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  // A fresh database starts at zero, and the count is the count of rows, not
  // of blocks: an empty chain must not need a scan to say "none".
  assert_eq!(counter(&pool).await?, 0, "a fresh database counts nothing");
  assert_exact(&pool, "genesis").await?;

  let mut src = Src::default();
  // h0: the spender is funded. Its coinbase output is what the later spend
  // consumes, taking it back to exactly zero.
  src.add(1, SPENDER, 500, None);
  // h1: a second address crosses zero upward.
  src.add(2, HOLDER, 300, None);
  // h2: a third.
  src.add(3, TOPUP, 100, None);
  // h3: a top-up of an already-positive address. Nothing crosses zero, so the
  // counter must not move — the naive "one per touched address" bug dies here.
  src.add(4, TOPUP, 50, None);
  // h4: one block, three crossings — the spender empties out (downward), and
  // both the heir and the block's miner are first-ever rows (upward).
  src.add_spend(
    5,
    MINER,
    10,
    Spend {
      prev: HyDbOutpoint {
        txid: HyDbHash(h(1)),
        vout: 0,
      },
      to: HEIR,
      value: 500,
    },
  );
  // The alternative branch off h2: two blocks funding two addresses that exist
  // on no other branch.
  src.add(0x14, FORK_A, 700, None);
  src.add(0x15, FORK_B, 800, None);
  let ingest = db.ingest(Arc::new(src)).await?;

  for (hash, prev) in [(1u8, 0u8), (2, 1), (3, 2)] {
    ingest.on_block(raw(hash, prev)).await?;
  }
  ingest.on_tip(HyDbTipTarget::AtNode(HyDbHash(h(3)))).await?;
  assert_exact(&pool, "h2").await?;
  assert_eq!(counter(&pool).await?, 3, "three upward crossings, one each");

  // A block that moves a balance without crossing zero.
  ingest.on_block(raw(4, 3)).await?;
  ingest.on_tip(HyDbTipTarget::AtNode(HyDbHash(h(4)))).await?;
  assert_exact(&pool, "h3").await?;
  assert_eq!(
    counter(&pool).await?,
    3,
    "topping up an already-positive address is not a crossing"
  );

  // One downward crossing and two upward ones in a single block.
  ingest.on_block(raw(5, 4)).await?;
  ingest.on_tip(HyDbTipTarget::AtNode(HyDbHash(h(5)))).await?;
  assert_eq!(
    positive(&pool).await?,
    vec![
      HEIR.to_owned(),
      HOLDER.to_owned(),
      MINER.to_owned(),
      TOPUP.to_owned()
    ],
    "the spender is emptied; the heir and the miner arrive"
  );
  assert_exact(&pool, "h4").await?;
  assert_eq!(counter(&pool).await?, 4);

  // The public read path serves the counter, not a scan.
  assert_eq!(db.addrs().bal_count().await?, 4);

  // Two-block reorg back to h2. The disconnect must give the spender its
  // balance back (upward) and take the heir and the miner away (downward,
  // and by row deletion rather than by zeroing), then the two branch blocks
  // add two more.
  ingest.on_block(raw(0x14, 3)).await?;
  ingest.on_block(raw(0x15, 0x14)).await?;
  let report = ingest
    .on_tip(HyDbTipTarget::AtNode(HyDbHash(h(0x15))))
    .await?;
  let reorg = report
    .reorg
    .ok_or("switching branches must report a reorg")?;
  assert_eq!(reorg.disconnected.len(), 2, "heights 3 and 4 disconnected");
  assert_eq!(
    positive(&pool).await?,
    vec![
      FORK_A.to_owned(),
      FORK_B.to_owned(),
      HOLDER.to_owned(),
      SPENDER.to_owned(),
      TOPUP.to_owned()
    ],
    "the spender is whole again and the branch's own addresses are here"
  );
  assert_exact(&pool, "after reorg").await?;
  assert_eq!(counter(&pool).await?, 5);

  // The heir's row is gone entirely rather than zeroed: the disconnect side of
  // the counter has to treat a vanished row as non-positive, not skip it.
  let heir: Option<(i64,)> = sqlx::query_as("SELECT balance FROM address_stats WHERE address = $1")
    .bind(HEIR)
    .fetch_optional(&pool)
    .await?;
  assert!(heir.is_none(), "the heir's block was disconnected");

  // Re-connecting the original branch returns the counter to its earlier
  // value: the two directions are inverses of one another, not merely each
  // individually plausible.
  ingest.on_tip(HyDbTipTarget::AtNode(HyDbHash(h(5)))).await?;
  assert_eq!(
    positive(&pool).await?,
    vec![
      HEIR.to_owned(),
      HOLDER.to_owned(),
      MINER.to_owned(),
      TOPUP.to_owned()
    ]
  );
  assert_exact(&pool, "after re-connect").await?;
  assert_eq!(counter(&pool).await?, 4, "back to the h4 count exactly");

  Ok(())
}

impl Src {
  fn add(&mut self, hash: u8, cb_to: &'static str, cb_value: i64, spend: Option<Spend>) {
    self.specs.insert(
      h(hash),
      Spec {
        cb_to,
        cb_value,
        spend,
      },
    );
  }

  fn add_spend(&mut self, hash: u8, cb_to: &'static str, cb_value: i64, spend: Spend) {
    self.add(hash, cb_to, cb_value, Some(spend));
  }
}
