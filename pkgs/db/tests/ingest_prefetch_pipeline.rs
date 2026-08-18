//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The look-ahead pipeline (task A4) against a real database.
//!
//! The unit tests in `ingest::prefetch` prove the queue's own properties in
//! isolation. These prove the two that only exist once the queue is wired to
//! the writer:
//!
//! * **Order survives the whole path.** Every non-genesis block here spends the
//!   output its parent created, and the deriver resolves prevouts against
//!   `utxo_set` inside the block's own transaction. A block applied before its
//!   parent therefore cannot find its input and fails by name. A chain long
//!   enough to fill the look-ahead several times over is, in this shape, an
//!   assertion that concurrent decoding never reordered the writer.
//!
//! * **A reorg discards the queued branch.** The data source records every hash
//!   it is asked to decode, so the test can assert that the advance which
//!   switches branches decodes *nothing* from the branch it is abandoning — not
//!   merely that the final projection came out right, which a leak followed by
//!   a correct disconnect would also satisfy.

mod common;

use std::collections::HashSet;
use std::sync::{Arc, Mutex};

use bytes::Bytes;
use common::TempDb;
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHash, HyDbHeader,
  HyDbHeight, HyDbOutpoint, HyDbResult, HyDbTx, HyDbTxIn, HyDbTxOut,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const ADDRESS: &str = "yPrefetchAddr0000000000000000000000";
const SUBSIDY: i64 = 50;

/// Branch tag `t`, height `h`. Distinct across both, and recoverable from the
/// bytes so the fake source can report what it decoded.
fn block_hash(tag: u8, height: u32) -> HyDbHash {
  let mut b = [0u8; 32];
  b[..4].copy_from_slice(&height.to_le_bytes());
  b[4] = tag;
  b[5] = 0xb1;
  HyDbHash(b)
}

/// The spending transaction's id, distinct from the block hash the coinbase
/// uses.
fn spend_txid(tag: u8, height: u32) -> HyDbHash {
  let mut b = block_hash(tag, height).0;
  b[5] = 0x5e;
  HyDbHash(b)
}

/// `hash(32) || prev(32) || parent_coinbase_txid(32)`; an all-zero parent txid
/// means "genesis, nothing to spend".
fn raw(tag: u8, height: u32, prev: HyDbHash, parent_cb: Option<HyDbHash>) -> Bytes {
  let mut v = Vec::with_capacity(96);
  v.extend_from_slice(&block_hash(tag, height).0);
  v.extend_from_slice(&prev.0);
  v.extend_from_slice(&parent_cb.unwrap_or(HyDbHash([0u8; 32])).0);
  Bytes::from(v)
}

/// A data source that records what it decoded, so a test can assert about work
/// that should never have happened.
#[derive(Debug, Default)]
struct RecordingSrc {
  decoded: Mutex<Vec<HyDbHash>>,
}

impl RecordingSrc {
  fn decoded(&self) -> Vec<HyDbHash> {
    self
      .decoded
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .clone()
  }

  fn reset(&self) {
    self
      .decoded
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .clear();
  }
}

fn out(value: i64, vout: u32) -> HyDbTxOut {
  HyDbTxOut {
    vout,
    value: HyDbAmount(value),
    script_pubkey: vec![0x76, 0xa9, 0x14],
    address: Some(HyDbAddress(ADDRESS.to_owned())),
    address_type: HyDbAddrType::P2pkh,
  }
}

impl HyDbDataSrc for RecordingSrc {
  fn identify(&self, raw: &[u8]) -> HyDbResult<HyDbBlockId> {
    Ok(HyDbBlockId {
      hash: HyDbHash::from_slice(&raw[..32])?,
      prev_hash: HyDbHash::from_slice(&raw[32..64])?,
    })
  }

  fn decode(&self, raw: &[u8]) -> HyDbResult<HyDbBlock> {
    let hash = HyDbHash::from_slice(&raw[..32])?;
    let prev_hash = HyDbHash::from_slice(&raw[32..64])?;
    let parent_cb = HyDbHash::from_slice(&raw[64..96])?;
    self
      .decoded
      .lock()
      .unwrap_or_else(|e| e.into_inner())
      .push(hash);

    let height = u32::from_le_bytes([hash.0[0], hash.0[1], hash.0[2], hash.0[3]]);
    let tag = hash.0[4];

    // Coinbase, always.
    let mut txs = vec![HyDbTx {
      txid: hash,
      block_pos: 0,
      version: 1,
      tx_type: hyphen_db::HyDbTxType::Coinbase,
      lock_time: 0,
      size: 200,
      value_out: HyDbAmount(SUBSIDY),
      inputs: vec![HyDbTxIn {
        vin_index: 0,
        prev_outpoint: None,
        coinbase: true,
        script_sig: vec![1],
        sequence: 0xffff_ffff,
      }],
      outputs: vec![out(SUBSIDY, 0)],
      payload: None,
      validator_events: Vec::new(),
    }];

    // …and, off genesis, a transaction spending the parent's coinbase output.
    // This is what makes the writer's order load-bearing: the input does not
    // exist until the parent block's transaction has committed.
    if parent_cb.0 != [0u8; 32] {
      txs.push(HyDbTx {
        txid: spend_txid(tag, height),
        block_pos: 1,
        version: 1,
        tx_type: hyphen_db::HyDbTxType::Normal,
        lock_time: 0,
        size: 220,
        value_out: HyDbAmount(SUBSIDY),
        inputs: vec![HyDbTxIn {
          vin_index: 0,
          prev_outpoint: Some(HyDbOutpoint {
            txid: parent_cb,
            vout: 0,
          }),
          coinbase: false,
          script_sig: Vec::new(),
          sequence: 0xffff_ffff,
        }],
        outputs: vec![out(SUBSIDY, 0)],
        payload: None,
        validator_events: Vec::new(),
      });
    }

    Ok(HyDbBlock {
      header: HyDbHeader {
        hash,
        prev_hash,
        merkle_root: hash,
        time: 1_600_000_000 + u64::from(height) as u32,
        bits: 0x1e0f_ffff,
        difficulty: 1.0,
        nonce: 0,
        version: 1,
        size: 96,
        tx_count: txs.len() as u32,
        chainwork: None,
        coinbase: None,
      },
      txs,
    })
  }
}

/// Blocks on branch `tag` covering `lo..=hi`, linked to `root` at `lo - 1`.
fn branch(tag: u8, lo: u32, hi: u32, root: Option<(HyDbHash, HyDbHash)>) -> Vec<Bytes> {
  let mut out = Vec::new();
  let (mut prev, mut parent_cb) = match root {
    Some((hash, cb)) => (hash, Some(cb)),
    None => (HyDbHash([0u8; 32]), None),
  };
  for height in lo..=hi {
    out.push(raw(tag, height, prev, parent_cb));
    prev = block_hash(tag, height);
    parent_cb = Some(prev);
  }
  out
}

#[tokio::test]
async fn the_look_ahead_keeps_order_and_a_reorg_discards_it() -> TestResult {
  let Some(tmp) = TempDb::create().await? else {
    return Err("no test database".into());
  };
  let result = run(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run(tmp: &TempDb) -> TestResult {
  // The default look-ahead is 64 deep; 300 blocks fills and drains it several
  // times, with decoding genuinely concurrent throughout.
  const TIP: u32 = 299;
  const FORK: u32 = 250;
  const ALT_TIP: u32 = 320;

  let db = tmp.open().await?;
  let src = Arc::new(RecordingSrc::default());
  let ingest = db.ingest(Arc::clone(&src) as Arc<dyn HyDbDataSrc>).await?;

  for block in branch(0xaa, 0, TIP, None) {
    ingest.on_block(block).await?;
  }
  let report = ingest.on_tip(block_hash(0xaa, TIP)).await?;
  assert_eq!(
    report.to,
    Some(HyDbHeight(TIP)),
    "the whole chain must connect; a block applied before its parent would \
     have failed to resolve its input instead"
  );
  assert!(report.reorg.is_none());

  // Every height is present exactly once, and each carries both of its
  // transactions — so no block was skipped by the queue either.
  assert_eq!(
    db.status().tip().await?.map(|t| t.height),
    Some(HyDbHeight(TIP))
  );
  assert_eq!(
    db.txs().total().await?,
    u64::from(TIP) * 2 + 1,
    "one coinbase per block plus one spend per non-genesis block"
  );
  for height in [0u32, 1, FORK, TIP] {
    let block = db
      .blocks()
      .by_height(HyDbHeight(height))
      .await?
      .ok_or_else(|| format!("height {height} missing after a look-ahead connect"))?;
    assert_eq!(block.hash, block_hash(0xaa, height));
  }

  // ---- the reorg ---------------------------------------------------------
  // A heavier branch forking at FORK. Everything above it on branch aa is
  // abandoned; the look-ahead built for branch aa must contribute nothing.
  let root = (block_hash(0xaa, FORK), block_hash(0xaa, FORK));
  for block in branch(0xbb, FORK + 1, ALT_TIP, Some(root)) {
    ingest.on_block(block).await?;
  }

  src.reset();
  let report = ingest.on_tip(block_hash(0xbb, ALT_TIP)).await?;
  let reorg = report
    .reorg
    .ok_or("switching branches must report a reorg")?;
  assert_eq!(reorg.fork_height, HyDbHeight(FORK));
  assert_eq!(
    reorg.disconnected.len(),
    (TIP - FORK) as usize,
    "every branch-aa block above the fork is disconnected"
  );
  assert_eq!(report.to, Some(HyDbHeight(ALT_TIP)));

  // The discard, asserted directly: the advance that abandoned branch aa did
  // not decode a single one of its blocks. A queue that leaked would have
  // decoded them ahead of the writer before the plan chose otherwise.
  let decoded: HashSet<HyDbHash> = src.decoded().into_iter().collect();
  let stale: Vec<u32> = ((FORK + 1)..=TIP)
    .filter(|h| decoded.contains(&block_hash(0xaa, *h)))
    .collect();
  assert!(
    stale.is_empty(),
    "the reorg decoded {} blocks from the branch it was abandoning: {:?}",
    stale.len(),
    &stale[..stale.len().min(8)]
  );
  assert!(
    decoded.contains(&block_hash(0xbb, ALT_TIP)),
    "the replacement branch must have been decoded"
  );

  // …and the projection is the new branch, with the old one's blocks gone.
  let tip = db.status().tip().await?.ok_or("tip after reorg")?;
  assert_eq!(tip.hash, block_hash(0xbb, ALT_TIP));
  for height in [FORK + 1, TIP, ALT_TIP] {
    let block = db
      .blocks()
      .by_height(HyDbHeight(height))
      .await?
      .ok_or_else(|| format!("height {height} missing after the reorg"))?;
    let expected = if height <= FORK { 0xaa } else { 0xbb };
    assert_eq!(
      block.hash,
      block_hash(expected, height),
      "height {height} should be on branch {expected:#x}"
    );
  }

  // A second advance to the same tip is a no-op, and re-reading it does not
  // resurrect anything from the discarded queue.
  src.reset();
  let again = ingest.on_tip(block_hash(0xbb, ALT_TIP)).await?;
  assert_eq!(again.to, Some(HyDbHeight(ALT_TIP)));
  assert!(again.reorg.is_none());
  assert!(
    src.decoded().is_empty(),
    "an advance with nothing to do must not decode anything"
  );

  Ok(())
}
