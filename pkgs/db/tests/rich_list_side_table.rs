//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Rich-list side-table (`address_rich`) maintenance across connect and
//! disconnect. `address_stats` carries no balance-ordered index — that index
//! made every ingest update non-HOT — so the ranked read is served from a
//! side table holding only addresses at or above `layout::RICH_LIST_FLOOR`,
//! maintained inside the same block transaction.
//!
//! The failure this file exists to catch is asymmetry: a connect path that
//! adds and removes members correctly while the disconnect path forgets, so
//! the side table drifts from `address_stats` across reorgs with nothing
//! detecting it. Every checkpoint therefore asserts the *whole* membership set
//! and the invariant `address_rich == {a : address_stats.balance >= floor}`.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use common::TempDb;
use hyphen_db::layout::RICH_LIST_FLOOR;
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHash, HyDbHeader,
  HyDbOutpoint, HyDbResult, HyDbTx, HyDbTxIn, HyDbTxOut, HyDbTxType,
};
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Above the floor from its first block: the member that a later spend must
/// evict, and that a disconnect must bring back.
const WHALE: &str = "yWhaleAAA00000000000000000000000000";
/// Never anywhere near the floor: the dust majority, which must never appear.
const DUST: &str = "yDustAAAA00000000000000000000000000";
/// Accumulates across two blocks, crossing the floor upward on the second.
const CROSSER: &str = "yCrossAAA00000000000000000000000000";
/// Receives the whale's spend: a brand-new row that a disconnect must delete
/// from both tables.
const HEIR: &str = "yHeirAAAA00000000000000000000000000";

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

/// `address_rich`, newest ranking first, as (address, balance).
async fn members(pool: &PgPool) -> Result<Vec<(String, i64)>, Box<dyn std::error::Error>> {
  Ok(
    sqlx::query_as("SELECT address, balance FROM address_rich ORDER BY balance DESC, address ASC")
      .fetch_all(pool)
      .await?,
  )
}

/// The invariant the whole design rests on: the side table is exactly the
/// above-floor slice of `address_stats`, balances included. Returns the
/// offending rows so a failure names them.
async fn assert_consistent(pool: &PgPool, at: &str) -> TestResult {
  let drift: Vec<(String, Option<i64>, Option<i64>)> = sqlx::query_as(
    r#"SELECT COALESCE(r.address, a.address), r.balance, a.balance
         FROM address_rich AS r
         FULL JOIN (SELECT address, balance FROM address_stats WHERE balance >= $1) AS a
           ON a.address = r.address
        WHERE r.address IS NULL OR a.address IS NULL OR r.balance <> a.balance"#,
  )
  .bind(RICH_LIST_FLOOR)
  .fetch_all(pool)
  .await?;
  assert!(
    drift.is_empty(),
    "address_rich drifted from address_stats at {at}: {drift:?}"
  );
  Ok(())
}

#[tokio::test]
async fn rich_list_side_table_tracks_the_floor_in_both_directions() -> TestResult {
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
  let f = RICH_LIST_FLOOR;

  let mut src = Src::default();
  // h0: the whale is born above the floor. Its coinbase output is the one the
  // later spend consumes.
  src.add(1, WHALE, 2 * f, None);
  // h1: dust — must never reach the side table.
  src.add(2, DUST, 100, None);
  // h2: the crosser lands one duff short of the floor.
  src.add(3, CROSSER, f - 1, None);
  // h3: one more duff takes it over. Crossing upward.
  src.add(4, CROSSER, 2, None);
  // h4: the whale spends everything to a brand-new address. Crossing downward
  // for the whale (0), and a first-ever row above the floor for the heir.
  src.add_spend(
    5,
    DUST,
    100,
    Spend {
      prev: HyDbOutpoint {
        txid: HyDbHash(h(1)),
        vout: 0,
      },
      to: HEIR,
      value: 2 * f,
    },
  );
  // The alternative branch off h2: two blocks of pure dust, so a reorg to it
  // must unwind both the crossing and the spend.
  src.add(0x14, DUST, 100, None);
  src.add(0x15, DUST, 100, None);
  let ingest = db.ingest(Arc::new(src)).await?;

  for (hash, prev) in [(1u8, 0u8), (2, 1), (3, 2)] {
    ingest.on_block(raw(hash, prev)).await?;
  }
  ingest.on_tip(HyDbHash(h(3))).await?;

  // Only the whale qualifies; dust and the one-duff-short crosser do not.
  assert_eq!(members(&pool).await?, vec![(WHALE.to_owned(), 2 * f)]);
  assert_consistent(&pool, "h2").await?;

  // Crossing upward.
  ingest.on_block(raw(4, 3)).await?;
  ingest.on_tip(HyDbHash(h(4))).await?;
  assert_eq!(
    members(&pool).await?,
    vec![(WHALE.to_owned(), 2 * f), (CROSSER.to_owned(), f + 1)],
    "the crosser joins on the block that lifts it over the floor"
  );
  assert_consistent(&pool, "h3").await?;

  // Crossing downward, plus a new above-floor row in the same block.
  ingest.on_block(raw(5, 4)).await?;
  ingest.on_tip(HyDbHash(h(5))).await?;
  assert_eq!(
    members(&pool).await?,
    vec![(HEIR.to_owned(), 2 * f), (CROSSER.to_owned(), f + 1)],
    "the spent-out whale leaves and its heir arrives"
  );
  assert_consistent(&pool, "h4").await?;

  // The API ranks within the side table and agrees with it.
  let page = db.addrs().rich_list(10, None).await?;
  let ranked: Vec<(String, i64)> = page
    .items
    .iter()
    .map(|e| (e.address.0.clone(), e.balance))
    .collect();
  assert_eq!(
    ranked,
    vec![(HEIR.to_owned(), 2 * f), (CROSSER.to_owned(), f + 1)]
  );
  assert_eq!(page.items[0].total_received, 2 * f);

  // Two-block reorg back to h2: the crossing and the spend both unwind.
  ingest.on_block(raw(0x14, 3)).await?;
  ingest.on_block(raw(0x15, 0x14)).await?;
  let report = ingest.on_tip(HyDbHash(h(0x15))).await?;
  let reorg = report
    .reorg
    .ok_or("switching branches must report a reorg")?;
  assert_eq!(reorg.disconnected.len(), 2, "heights 3 and 4 disconnected");

  assert_eq!(
    members(&pool).await?,
    vec![(WHALE.to_owned(), 2 * f)],
    "disconnect restores the whale, drops the crosser back below the floor, \
     and removes the heir whose row no longer exists"
  );
  assert_consistent(&pool, "after reorg").await?;

  // The heir's `address_stats` row is gone entirely, not merely zeroed — the
  // side table's delete must key off the row's disappearance, not its balance.
  let heir: Option<(i64,)> = sqlx::query_as("SELECT balance FROM address_stats WHERE address = $1")
    .bind(HEIR)
    .fetch_optional(&pool)
    .await?;
  assert!(
    heir.is_none(),
    "the heir was registered by a disconnected block"
  );

  // Re-connecting the original branch reproduces the same membership: the two
  // directions are inverses, not merely each individually plausible.
  ingest.on_tip(HyDbHash(h(5))).await?;
  assert_eq!(
    members(&pool).await?,
    vec![(HEIR.to_owned(), 2 * f), (CROSSER.to_owned(), f + 1)]
  );
  assert_consistent(&pool, "after re-connect").await?;

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
