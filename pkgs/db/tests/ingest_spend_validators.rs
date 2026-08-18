//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! End-to-end ingest coverage for the spend + validator diagonal that the
//! coinbase-only `ingest_fidelity` chain never touches: non-coinbase inputs
//! (spend resolution, inlined provenance, the `spends`/`utxo_set` ledger) and
//! the full validator lifecycle (register → update → revoke, plus collateral
//! reap when a registered validator's collateral outpoint is spent). A final
//! reorg disconnects the spend/validator blocks to exercise the stage inverses
//! (`ValidatorsStage::restore`, spend undo).
//!
//! Skips (returns `Ok`) when `HYPHEN_TEST_DATABASE_URL` is unset.
//!
//! A block is `hash(32) || prev_hash(32)`; the data source looks the rest up in
//! a per-block spec it was constructed with, so a block's coinbase, optional
//! spend, and validator events are all deterministic in the raw bytes.

mod common;

use std::collections::HashMap;
use std::sync::Arc;

use bytes::Bytes;
use common::{TempDb, ENV};
use hyphen_db::{
  HyDbAddrType, HyDbAddress, HyDbAmount, HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHash, HyDbHeader,
  HyDbHeight, HyDbOutpoint, HyDbResult, HyDbTx, HyDbTxIn, HyDbTxOut, HyDbTxType, HyDbVlEvent,
  HyDbVlRegister, HyDbVlRevoke, HyDbVlType, HyDbVlUpdRegistrar, HyDbVlUpdService,
};

type TestResult = Result<(), Box<dyn std::error::Error>>;

const A: &str = "yAddrAAAA00000000000000000000000000";
const B: &str = "yAddrBBBB00000000000000000000000000";

fn h(n: u8) -> [u8; 32] {
  [n; 32]
}

/// A non-coinbase spend: consume `prev`, create one output to `to`.
#[derive(Clone)]
struct Spend {
  prev: HyDbOutpoint,
  to: &'static str,
  value: i64,
}

/// Everything needed to decode one block deterministically.
#[derive(Clone)]
struct Spec {
  cb_to: &'static str,
  cb_value: i64,
  spend: Option<Spend>,
  events: Vec<HyDbVlEvent>,
}

#[derive(Default)]
struct Src {
  specs: HashMap<[u8; 32], Spec>,
}

impl Src {
  fn add(&mut self, hash: u8, spec: Spec) {
    self.specs.insert(h(hash), spec);
  }
}

/// The spend tx's txid: the block hash with its first byte flipped, so it never
/// collides with the coinbase txid (== block hash).
fn spend_txid(block: [u8; 32]) -> HyDbHash {
  let mut t = block;
  t[0] ^= 0xff;
  HyDbHash(t)
}

fn coinbase_tx(hash: [u8; 32], to: &str, value: i64, events: Vec<HyDbVlEvent>) -> HyDbTx {
  HyDbTx {
    txid: HyDbHash(hash),
    block_pos: 0,
    version: 1,
    tx_type: HyDbTxType::Coinbase,
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
      address: Some(HyDbAddress(to.to_owned())),
      address_type: HyDbAddrType::P2pkh,
    }],
    payload: None,
    validator_events: events,
  }
}

fn spend_tx(hash: [u8; 32], sp: &Spend) -> HyDbTx {
  HyDbTx {
    txid: spend_txid(hash),
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
    outputs: vec![HyDbTxOut {
      vout: 0,
      value: HyDbAmount(sp.value),
      script_pubkey: vec![0x76, 0xa9, 0x14],
      address: Some(HyDbAddress(sp.to.to_owned())),
      address_type: HyDbAddrType::P2pkh,
    }],
    payload: None,
    validator_events: Vec::new(),
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
      .specs
      .get(&hash.0)
      .ok_or_else(|| hyphen_db::HyDbError::Derive("no spec for block".into()))?;

    let mut txs = vec![coinbase_tx(
      hash.0,
      spec.cb_to,
      spec.cb_value,
      spec.events.clone(),
    )];
    if let Some(sp) = &spec.spend {
      txs.push(spend_tx(hash.0, sp));
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

fn outpoint(txid: [u8; 32], vout: u32) -> HyDbOutpoint {
  HyDbOutpoint {
    txid: HyDbHash(txid),
    vout,
  }
}

// ---- validator-event builders ---------------------------------------------

fn register(protx: u8, collateral: HyDbOutpoint) -> HyDbVlEvent {
  HyDbVlEvent::Register(HyDbVlRegister {
    protx_hash: HyDbHash(h(protx)),
    collateral_hash: collateral.txid,
    collateral_index: collateral.vout as i32,
    collateral_address: Some(HyDbAddress(A.to_owned())),
    owner_address: Some(HyDbAddress("yOwner".to_owned())),
    operator_pubkey: Some(vec![0xaa; 48]),
    voting_address: Some(HyDbAddress("yVoting".to_owned())),
    addr_core_p2p: vec!["1.2.3.4:9999".to_owned()],
    addr_platform_p2p: Vec::new(),
    addr_platform_https: Vec::new(),
    payout_address: Some(HyDbAddress("yPayout".to_owned())),
    operator_payout_address: None,
    operator_reward: 0,
    validator_type: HyDbVlType::Regular,
  })
}

fn upd_service(protx: u8) -> HyDbVlEvent {
  HyDbVlEvent::UpdateService(HyDbVlUpdService {
    protx_hash: HyDbHash(h(protx)),
    addr_core_p2p: vec!["5.6.7.8:9999".to_owned()],
    addr_platform_p2p: Vec::new(),
    addr_platform_https: Vec::new(),
    operator_payout_address: Some(HyDbAddress("yOpPayout".to_owned())),
  })
}

fn upd_registrar(protx: u8) -> HyDbVlEvent {
  HyDbVlEvent::UpdateRegistrar(HyDbVlUpdRegistrar {
    protx_hash: HyDbHash(h(protx)),
    operator_pubkey: vec![0xbb; 48],
    payout_address: Some(HyDbAddress("yPayout2".to_owned())),
    voting_address: Some(HyDbAddress("yVoting2".to_owned())),
  })
}

fn revoke(protx: u8) -> HyDbVlEvent {
  HyDbVlEvent::Revoke(HyDbVlRevoke {
    protx_hash: HyDbHash(h(protx)),
  })
}

const V: u8 = 0x51; // reaped via collateral spend
const W: u8 = 0x52; // revoked

fn build_src() -> Src {
  let mut src = Src::default();
  // B1 h0: coinbase -> A. Creates the outpoint used as V's collateral.
  src.add(
    1,
    Spec {
      cb_to: A,
      cb_value: 100,
      spend: None,
      events: vec![],
    },
  );
  // B2 h1: coinbase -> A. Register V (collateral (h1,0)) and W (collateral (h2,0)).
  src.add(
    2,
    Spec {
      cb_to: A,
      cb_value: 100,
      spend: None,
      events: vec![
        register(V, outpoint(h(1), 0)),
        register(W, outpoint(h(2), 0)),
      ],
    },
  );
  // B3 h2: coinbase -> B. Update V twice (service, then registrar).
  src.add(
    3,
    Spec {
      cb_to: B,
      cb_value: 100,
      spend: None,
      events: vec![upd_service(V), upd_registrar(V)],
    },
  );
  // B4 h3: coinbase -> B. Revoke W. Spend B3's output (h3,0) [B] -> A.
  src.add(
    4,
    Spec {
      cb_to: B,
      cb_value: 100,
      spend: Some(Spend {
        prev: outpoint(h(3), 0),
        to: A,
        value: 100,
      }),
      events: vec![revoke(W)],
    },
  );
  // B5 h4: coinbase -> A. Spend (h1,0) == V's collateral -> reaps V.
  src.add(
    5,
    Spec {
      cb_to: A,
      cb_value: 100,
      spend: Some(Spend {
        prev: outpoint(h(1), 0),
        to: A,
        value: 100,
      }),
      events: vec![],
    },
  );
  // Alt branch off B3 (h2) for the reorg: two plain coinbase blocks.
  src.add(
    6,
    Spec {
      cb_to: A,
      cb_value: 100,
      spend: None,
      events: vec![],
    },
  );
  src.add(
    7,
    Spec {
      cb_to: A,
      cb_value: 100,
      spend: None,
      events: vec![],
    },
  );
  src
}

#[tokio::test]
async fn ingest_spends_validators_and_reorgs() -> TestResult {
  let Some(tmp) = TempDb::create().await? else {
    eprintln!("{ENV} unset; skipping spend/validator ingest test");
    return Ok(());
  };
  let result = run(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let ingest = db.ingest(Arc::new(build_src())).await?;

  // Absorb the main chain (genesis first so parents resolve).
  for (hash, prev) in [(1u8, 0u8), (2, 1), (3, 2), (4, 3), (5, 4)] {
    assert!(
      matches!(
        ingest.on_block(raw(hash, prev)).await?,
        hyphen_db::HyDbBlockSeen::New
      ),
      "block {hash} newly absorbed"
    );
  }
  let report = ingest.on_tip(HyDbHash(h(5))).await?;
  assert_eq!(report.to, Some(HyDbHeight(4)));
  assert!(report.reorg.is_none(), "linear connect is not a reorg");

  // --- Spend resolution + balances -----------------------------------------
  // A: created (h1,0),(h2,0),(h5,0),B4-spend-out,B5-spend-out = 5; spent (h1,0) = 1.
  let a = db
    .addrs()
    .summary(HyDbAddress(A.to_owned()))
    .await?
    .ok_or("address A exists")?;
  assert_eq!(a.balance, 400, "A: 5 outs of 100 minus one 100 spent");
  assert_eq!(a.utxo_count, 4);

  // B: created (h3,0) and (h4,0); (h3,0) is spent in B4, leaving (h4,0).
  let b = db
    .addrs()
    .summary(HyDbAddress(B.to_owned()))
    .await?
    .ok_or("address B exists")?;
  assert_eq!(
    b.balance, 100,
    "B keeps the unspent B4 coinbase; (h3,0) was spent"
  );
  assert_eq!(b.utxo_count, 1);

  assert_eq!(db.txs().total().await?, 7, "5 coinbases + 2 spends");

  // The B4 spend tx records inlined provenance for the output it consumed (B's).
  let spend = db
    .txs()
    .get(spend_txid(h(4)))
    .await?
    .ok_or("B4 spend tx exists")?;
  assert_eq!(spend.inputs.len(), 1);
  assert!(!spend.inputs[0].coinbase);
  assert_eq!(
    spend.inputs[0].prev_address.as_ref().map(|a| a.0.as_str()),
    Some(B)
  );
  assert_eq!(spend.inputs[0].prev_value, Some(100));

  // --- Validator lifecycle -------------------------------------------------
  // W was revoked; V was reaped (collateral spent), so it is gone.
  let w = db
    .validators()
    .get(HyDbHash(h(W)))
    .await?
    .ok_or("W still registered-row exists")?;
  assert_eq!(w.status, "revoked");
  // V picked up its two updates before being reaped: confirm it is now absent.
  assert!(
    db.validators().get(HyDbHash(h(V))).await?.is_none(),
    "V reaped by collateral spend"
  );

  let counts = db.validators().counts().await?;
  assert!(
    counts.iter().any(|c| c.status == "revoked" && c.total >= 1),
    "counts reflect the revoked validator, got {counts:?}"
  );

  // --- Reorg: switch to the alt branch off B3, dropping B4 (revoke+spend) and
  //     B5 (reap+spend). Drives ValidatorsStage::restore and spend undo. -----
  for (hash, prev) in [(6u8, 3u8), (7, 6)] {
    ingest.on_block(raw(hash, prev)).await?;
  }
  let report = ingest.on_tip(HyDbHash(h(7))).await?;
  let reorg = report.reorg.ok_or("switching branches reports a reorg")?;
  assert_eq!(
    reorg.disconnected.len(),
    2,
    "old heights 3 and 4 disconnected"
  );
  assert_eq!(report.to, Some(HyDbHeight(4)));

  let tip = db.status().tip().await?.ok_or("tip after reorg")?;
  assert_eq!(tip.hash, HyDbHash(h(7)));

  // W's revoke (B4) undone → back to registered; V's reap (B5) undone → restored.
  let w = db
    .validators()
    .get(HyDbHash(h(W)))
    .await?
    .ok_or("W after reorg")?;
  assert_eq!(w.status, "registered", "revoke was disconnected");
  let v = db
    .validators()
    .get(HyDbHash(h(V)))
    .await?
    .ok_or("V restored after un-reap")?;
  assert_eq!(v.status, "registered");
  // V retains its pre-reorg updated service address (from B3's UpdateService).
  assert_eq!(v.addr_core_p2p, vec!["5.6.7.8:9999".to_owned()]);

  // The old spend/validator blocks survive as orphans in the raw store.
  assert!(db.blocks().raw_bytes(HyDbHash(h(5))).await?.is_some());
  assert!(
    db.blocks().by_hash(HyDbHash(h(5))).await?.is_none(),
    "B5 no longer canonical"
  );

  Ok(())
}
