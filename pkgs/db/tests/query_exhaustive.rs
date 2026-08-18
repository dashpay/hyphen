//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Exhaustive read-path coverage: seed a small multi-block, multi-address chain
//! (with a spent output, two validators, validator history, and per-block
//! metrics) via raw SQL, then drive every query method — including BOTH keyset
//! pagination arms (a small `limit` forces a `next_cursor`, which is fed back)
//! and the malformed-cursor rejection paths.
//!
//! Skips (returns `Ok`) when `HYPHEN_TEST_DATABASE_URL` is unset.

mod common;

use common::{TempDb, ENV};
use hyphen_db::query::HyDbSearchKind;
use hyphen_db::{HyDbAddress, HyDbCursor, HyDbError, HyDbHash, HyDbHeight, HyDbVlType, HyphenDb};
use rstest::rstest;
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

const A: &str = "yAddrAAAA00000000000000000000000000";
const B: &str = "yAddrBBBB00000000000000000000000000";

fn b32(n: u8) -> Vec<u8> {
  vec![n; 32]
}

/// Packed 36-byte outpoint key: txid(32) || vout(4 BE).
fn outpoint(txid: u8, vout: i32) -> Vec<u8> {
  let mut v = b32(txid);
  v.extend_from_slice(&vout.to_be_bytes());
  v
}

/// Seed three canonical blocks (heights 1–3), five transactions (one spend, one
/// provider-register with a payload), two addresses, two validators, counts,
/// and per-block metrics.
async fn seed(pool: &PgPool) -> TestResult {
  for stmt in [
    "CREATE TABLE transactions_p0 PARTITION OF transactions FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE tx_inputs_p0 PARTITION OF tx_inputs FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE tx_outputs_p0 PARTITION OF tx_outputs FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE address_txs_p0 PARTITION OF address_txs FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE spends_p0 PARTITION OF spends FOR VALUES FROM (0) TO (100000)",
  ] {
    sqlx::query(stmt).execute(pool).await?;
  }

  // Blocks 1..=3 (hashes 0x11/0x22/0x33), linked by prev_hash.
  let prevs = [b32(0x00), b32(0x11), b32(0x22)];
  let hashes = [b32(0x11), b32(0x22), b32(0x33)];
  for (i, (hash, prev)) in hashes.iter().zip(prevs.iter()).enumerate() {
    let height = (i + 1) as i32;
    sqlx::query("INSERT INTO ser_blocks (hash, prev_hash, height, raw) VALUES ($1,$2,$3,$4)")
      .bind(hash)
      .bind(prev)
      .bind(height)
      .bind(vec![0xde_u8, 0xad])
      .execute(pool)
      .await?;
    sqlx::query(
      "INSERT INTO blocks (height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count)
       VALUES ($1,$2,$3,$2,1600000000,0x1e0fffff,0,1,285,2)",
    )
    .bind(height)
    .bind(hash)
    .bind(prev)
    .execute(pool)
    .await?;
    // Per-block metrics row.
    sqlx::query(
      "INSERT INTO block_metrics
         (height, tx_count, utxos_spendable, utxos_unspendable, blockchain_size,
          credit_pool_balance, circulating_supply, difficulty)
       VALUES ($1, $1, $1, 0, 285, 0, $2, 1.5)",
    )
    .bind(height)
    .bind((height as i64) * 1000)
    .execute(pool)
    .await?;
  }

  // Transactions: coinbases (a1,a2,a3), a spend (b2) of a1's output, and a
  // provider-register (c3) carrying a payload.
  type SeedTx<'a> = (&'a [u8], i32, i32, &'a str, i64, i32, i32);
  let txs: &[SeedTx] = &[
    // (txid, height, block_pos, tx_type, value_out, input_count, output_count)
    (&[0xa1; 32], 1, 0, "coinbase", 1000, 1, 1),
    (&[0xa2; 32], 2, 0, "coinbase", 1000, 1, 1),
    (&[0xb2; 32], 2, 1, "normal", 1000, 1, 1),
    (&[0xa3; 32], 3, 0, "coinbase", 1000, 1, 1),
    (&[0xc3; 32], 3, 1, "provider_register", 0, 0, 0),
  ];
  for (txid, height, pos, ty, value, ins, outs) in txs {
    sqlx::query(
      "INSERT INTO transactions (txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee)
       VALUES ($1,$2,$3,1,$4::tx_type,0,200,$5,$6,$7,NULL)",
    )
    .bind(&txid[..])
    .bind(height)
    .bind(pos)
    .bind(ty)
    .bind(value)
    .bind(ins)
    .bind(outs)
    .execute(pool)
    .await?;
  }

  // Outputs: a1→A (spent), a2→A, a3→A, b2→B. utxo_set holds the unspent three.
  let outs: &[(u8, &str)] = &[(0xa1, A), (0xa2, A), (0xa3, A), (0xb2, B)];
  for (txid, addr) in outs {
    let height = match txid {
      0xa1 => 1,
      _ => {
        if *txid == 0xa3 {
          3
        } else {
          2
        }
      }
    };
    sqlx::query(
      "INSERT INTO tx_outputs (block_height, txid, vout_index, value, script_pubkey, address, address_type)
       VALUES ($1,$2,0,1000,'\\x76a914',$3,'p2pkh')",
    )
    .bind(height)
    .bind(b32(*txid))
    .bind(*addr)
    .execute(pool)
    .await?;
  }
  // utxo_set: a2, a3, b2 unspent (a1 was spent by b2).
  for (txid, addr, height) in [(0xa2u8, A, 2i32), (0xa3, A, 3), (0xb2, B, 2)] {
    sqlx::query(
      "INSERT INTO utxo_set (outpoint, address, value, block_height, script_pubkey)
       VALUES ($1,$2,1000,$3,'\\x76a914')",
    )
    .bind(outpoint(txid, 0))
    .bind(addr)
    .bind(height)
    .execute(pool)
    .await?;
  }

  // The spend: b2 consumes a1:0 (A's output), recording inlined provenance.
  sqlx::query(
    "INSERT INTO tx_inputs (block_height, txid, vin_index, prev_txid, prev_vout, script_sig, sequence, prev_address, prev_value)
     VALUES (2,$1,0,$2,0,'\\x02',4294967295,$3,1000)",
  )
  .bind(b32(0xb2))
  .bind(b32(0xa1))
  .bind(A)
  .execute(pool)
  .await?;
  sqlx::query(
    "INSERT INTO spends (spend_height, spending_txid, spending_vin, outpoint) VALUES (2,$1,0,$2)",
  )
  .bind(b32(0xb2))
  .bind(outpoint(0xa1, 0))
  .execute(pool)
  .await?;

  // address_txs: A appears in four txs (incl. the spend), B in one.
  let atxs: &[(&str, i32, u8, i32)] = &[
    (A, 1, 0xa1, 0),
    (A, 2, 0xa2, 0),
    (A, 2, 0xb2, 1),
    (A, 3, 0xa3, 0),
    (B, 2, 0xb2, 1),
  ];
  for (addr, height, txid, pos) in atxs {
    sqlx::query(
      "INSERT INTO address_txs (address, block_height, txid, block_pos) VALUES ($1,$2,$3,$4)",
    )
    .bind(*addr)
    .bind(height)
    .bind(b32(*txid))
    .bind(pos)
    .execute(pool)
    .await?;
  }

  // address_stats: A holds a2+a3 (2000), B holds b2 (1000).
  sqlx::query(
    "INSERT INTO address_stats (address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height)
     VALUES ($1,2000,3000,4,2,1,3), ($2,1000,1000,1,1,2,2)",
  )
  .bind(A)
  .bind(B)
  .execute(pool)
  .await?;

  // Two validators: an evonode (registered) and a regular (revoked).
  sqlx::query(
    "INSERT INTO validators
       (protx_hash, collateral_hash, collateral_index, owner_address, addr_core_p2p,
        validator_type, status, registered_height, last_updated_height)
     VALUES ($1,$2,0,'yOwner1',ARRAY['1.1.1.1:1'],'evonode','registered',3,3),
            ($3,$4,1,'yOwner2',ARRAY['2.2.2.2:2'],'regular','revoked',1,2)",
  )
  .bind(b32(0xd1))
  .bind(b32(0xe1))
  .bind(b32(0xd2))
  .bind(b32(0xe2))
  .execute(pool)
  .await?;
  sqlx::query(
    "INSERT INTO validator_counts (status, validator_type, total)
     VALUES ('registered','evonode',1), ('revoked','regular',1)",
  )
  .execute(pool)
  .await?;
  // History for the evonode: the provider-register special tx c3.
  sqlx::query(
    "INSERT INTO sptx_payloads (block_height, txid, tx_type, protx_hash, payload)
     VALUES (3,$1,'provider_register',$2,'{\"kind\":\"register\"}'::jsonb)",
  )
  .bind(b32(0xc3))
  .bind(b32(0xd1))
  .execute(pool)
  .await?;

  // Summaries.
  sqlx::query("UPDATE transactions_summary SET total = 5 WHERE singleton = TRUE")
    .execute(pool)
    .await?;
  sqlx::query(
    "UPDATE supply_summary SET total = 3000, included_supply = 3000 WHERE singleton = TRUE",
  )
  .execute(pool)
  .await?;
  Ok(())
}

#[tokio::test]
async fn read_path_exhaustive() -> TestResult {
  let Some(tmp) = TempDb::create().await? else {
    eprintln!("{ENV} unset; skipping exhaustive read test");
    return Ok(());
  };
  let result = run(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;
  seed(&pool).await?;

  blocks(&db).await?;
  transactions(&db).await?;
  addresses(&db).await?;
  validators(&db).await?;
  metrics(&db).await?;
  search_and_status(&db).await?;
  Ok(())
}

async fn blocks(db: &HyphenDb) -> TestResult {
  // list: both pagination arms (limit 2 over 3 blocks).
  let p1 = db.blocks().list(2, None).await?;
  assert_eq!(p1.items.len(), 2);
  let cur = p1.next_cursor.clone().ok_or("blocks page 1 has a cursor")?;
  let p2 = db.blocks().list(2, Some(cur)).await?;
  assert_eq!(p2.items.len(), 1, "second page holds the remaining block");
  assert!(p2.next_cursor.is_none());

  // by_height / by_hash / exists / height_for_hash / hash_for_height.
  assert!(db.blocks().by_height(HyDbHeight(2)).await?.is_some());
  assert!(db.blocks().by_hash(HyDbHash([0x22; 32])).await?.is_some());
  assert!(db.blocks().by_hash(HyDbHash([0x99; 32])).await?.is_none());
  assert!(db.blocks().exists(HyDbHeight(1)).await?);
  assert!(!db.blocks().exists(HyDbHeight(999)).await?);
  assert_eq!(
    db.blocks().height_for_hash(HyDbHash([0x11; 32])).await?,
    Some(HyDbHeight(1))
  );
  assert_eq!(
    db.blocks().hash_for_height(HyDbHeight(3)).await?,
    Some(HyDbHash([0x33; 32]))
  );

  // txs of a block: both arms (block 2 has two txs).
  let t1 = db.blocks().txs(HyDbHeight(2), 1, None).await?;
  assert_eq!(t1.items.len(), 1);
  let cur = t1
    .next_cursor
    .clone()
    .ok_or("block-txs page 1 has a cursor")?;
  let t2 = db.blocks().txs(HyDbHeight(2), 1, Some(cur)).await?;
  assert_eq!(t2.items.len(), 1);

  // raw_bytes / topology / search_prefix.
  assert!(db.blocks().raw_bytes(HyDbHash([0x11; 32])).await?.is_some());
  assert!(db.blocks().topology(HyDbHash([0x11; 32])).await?.is_some());
  assert!(db.blocks().topology(HyDbHash([0x99; 32])).await?.is_none());
  let hits = db.blocks().search_prefix("11".to_owned(), 10).await?;
  assert!(
    hits.iter().any(|r| r.hash == HyDbHash([0x11; 32])),
    "block search by display prefix"
  );
  Ok(())
}

async fn transactions(db: &HyphenDb) -> TestResult {
  // list: both arms (limit 2 over 5 txs).
  let p1 = db.txs().list(2, None).await?;
  assert_eq!(p1.items.len(), 2);
  let cur = p1.next_cursor.clone().ok_or("tx page 1 has a cursor")?;
  let p2 = db.txs().list(2, Some(cur)).await?;
  assert_eq!(p2.items.len(), 2);
  assert!(p2.next_cursor.is_some(), "still more txs after page 2");

  assert_eq!(db.txs().total().await?, 5);

  // get: the spend tx carries inlined provenance; the coinbase output is spent.
  let spend = db
    .txs()
    .get(HyDbHash([0xb2; 32]))
    .await?
    .ok_or("spend tx exists")?;
  assert_eq!(
    spend.inputs[0].prev_address.as_ref().map(|a| a.0.as_str()),
    Some(A)
  );
  assert_eq!(spend.inputs[0].prev_value, Some(1000));
  let cb = db
    .txs()
    .get(HyDbHash([0xa1; 32]))
    .await?
    .ok_or("coinbase tx exists")?;
  assert!(cb.outputs[0].spent_by.is_some(), "a1:0 was spent by b2");

  // search_prefix: exact prefix (one hit) and truncation (three 'a' txids, limit 2).
  assert_eq!(db.txs().search_prefix("a1".to_owned(), 10).await?.len(), 1);
  assert_eq!(
    db.txs().search_prefix("a".to_owned(), 2).await?.len(),
    2,
    "truncated at limit"
  );
  Ok(())
}

async fn addresses(db: &HyphenDb) -> TestResult {
  let a = || HyDbAddress(A.to_owned());
  // summary / exists.
  let s = db.addrs().summary(a()).await?.ok_or("A summary")?;
  assert_eq!(s.balance, 2000);
  assert!(db.addrs().exists(a()).await?);
  assert!(!db.addrs().exists(HyDbAddress("yNope".to_owned())).await?);

  // history: both arms (A has 4 address_txs, limit 2).
  let h1 = db.addrs().history(a(), 2, None).await?;
  assert_eq!(h1.items.len(), 2);
  let cur = h1.next_cursor.clone().ok_or("history page 1 cursor")?;
  let h2 = db.addrs().history(a(), 2, Some(cur)).await?;
  assert_eq!(h2.items.len(), 2);

  // utxos: both arms (A has 2 utxos, limit 1).
  let u1 = db.addrs().utxos(a(), 1, None).await?;
  assert_eq!(u1.items.len(), 1);
  let cur = u1.next_cursor.clone().ok_or("utxos page 1 cursor")?;
  let u2 = db.addrs().utxos(a(), 1, Some(cur)).await?;
  assert_eq!(u2.items.len(), 1);

  // rich_list: both arms (2 addresses, limit 1) — highest balance first.
  let r1 = db.addrs().rich_list(1, None).await?;
  assert_eq!(r1.items.len(), 1);
  assert_eq!(r1.items[0].address.0, A, "A (2000) ranks above B (1000)");
  let cur = r1.next_cursor.clone().ok_or("rich_list page 1 cursor")?;
  let r2 = db.addrs().rich_list(1, Some(cur)).await?;
  assert_eq!(r2.items.len(), 1);
  assert_eq!(r2.items[0].address.0, B);

  // bal_count / supply / search_prefix.
  assert_eq!(
    db.addrs().bal_count().await?,
    2,
    "two positive-balance addresses"
  );
  assert_eq!(db.addrs().supply().await?.total, 3000);
  let hits = db.addrs().search_prefix("yAddrA".to_owned(), 10).await?;
  assert!(hits.iter().any(|h| h.0 == A));
  Ok(())
}

async fn validators(db: &HyphenDb) -> TestResult {
  // list: both arms (2 validators, limit 1).
  let p1 = db.validators().list(None, None, 1, None).await?;
  assert_eq!(p1.items.len(), 1);
  let cur = p1.next_cursor.clone().ok_or("validator page 1 cursor")?;
  let p2 = db.validators().list(None, None, 1, Some(cur)).await?;
  assert_eq!(p2.items.len(), 1);

  // status + type filters.
  let revoked = db
    .validators()
    .list(Some("revoked".to_owned()), None, 10, None)
    .await?;
  assert_eq!(revoked.items.len(), 1);
  assert_eq!(revoked.items[0].status, "revoked");
  let evo = db
    .validators()
    .list(None, Some(HyDbVlType::Evonode), 10, None)
    .await?;
  assert_eq!(evo.items.len(), 1);
  assert_eq!(evo.items[0].validator_type, HyDbVlType::Evonode);

  // get / history / counts / search_prefix.
  assert!(db.validators().get(HyDbHash([0xd1; 32])).await?.is_some());
  let hist = db.validators().history(HyDbHash([0xd1; 32])).await?;
  assert_eq!(hist.len(), 1, "one provider-register payload in history");
  let counts = db.validators().counts().await?;
  assert_eq!(counts.iter().map(|c| c.total).sum::<i64>(), 2);
  assert_eq!(
    db.validators()
      .search_prefix("d1".to_owned(), 10)
      .await?
      .len(),
    1
  );
  Ok(())
}

async fn metrics(db: &HyphenDb) -> TestResult {
  let latest = db.metrics().latest().await?.ok_or("latest metric")?;
  assert_eq!(latest.height, HyDbHeight(3), "latest is the highest block");

  let all = db.metrics().range(HyDbHeight(1), HyDbHeight(3), 1).await?;
  assert_eq!(all.len(), 3);

  // Step 2 keeps heights divisible by 2 within [1,3] → just height 2.
  let stepped = db.metrics().range(HyDbHeight(1), HyDbHeight(3), 2).await?;
  assert_eq!(
    stepped.iter().map(|m| m.height.0).collect::<Vec<_>>(),
    vec![2]
  );
  Ok(())
}

async fn search_and_status(db: &HyphenDb) -> TestResult {
  // A single prefix call runs all four kind-arms; assert the matching kinds.
  let tx_hits = db.search().prefix("a1".to_owned(), 10).await?;
  assert!(tx_hits.iter().any(|h| h.kind == HyDbSearchKind::Tx));
  let block_hits = db.search().prefix("11".to_owned(), 10).await?;
  assert!(block_hits.iter().any(|h| h.kind == HyDbSearchKind::Block));
  let addr_hits = db.search().prefix("yAddrA".to_owned(), 10).await?;
  assert!(addr_hits.iter().any(|h| h.kind == HyDbSearchKind::Address));
  // Validator arm (protx_hash [0xd1; 32] → hex "d1…").
  let vl_hits = db.search().prefix("d1".to_owned(), 10).await?;
  assert!(vl_hits.iter().any(|h| h.kind == HyDbSearchKind::Validator));
  // Truncation: a limit of 1 caps the merged hit list regardless of arm count.
  assert_eq!(db.search().prefix("d1".to_owned(), 1).await?.len(), 1);

  // status: health + tip + sizes.
  assert!(
    db.status().health().await?.tip.is_some(),
    "health reports the tip"
  );
  assert_eq!(
    db.status().tip().await?.map(|t| t.height),
    Some(HyDbHeight(3))
  );
  assert!(
    !db.status().sizes().await?.is_empty(),
    "size diagnostics list the tables"
  );
  Ok(())
}

/// Malformed keyset cursors are rejected with `Decode` before any SQL runs. One
/// connected-but-otherwise-untouched handle drives each cursor format. Async
/// `#[rstest]` cases compose with `#[tokio::test]`.
#[rstest]
#[case::not_an_int("not-an-int")]
#[case::wrong_shape("1:2:zz")]
#[tokio::test]
async fn rejects_malformed_cursor(#[case] bad: &str) -> TestResult {
  let Some(tmp) = TempDb::create().await? else {
    return Ok(());
  };
  let outcome = async {
    let db = tmp.open().await?;
    let blk = db.blocks().list(10, Some(HyDbCursor(bad.to_owned()))).await;
    assert!(
      matches!(blk, Err(HyDbError::Decode(_))),
      "block cursor rejected: {blk:?}"
    );
    let tx = db.txs().list(10, Some(HyDbCursor(bad.to_owned()))).await;
    assert!(
      matches!(tx, Err(HyDbError::Decode(_))),
      "tx cursor rejected: {tx:?}"
    );
    Ok::<(), Box<dyn std::error::Error>>(())
  }
  .await;
  tmp.drop_now().await;
  outcome
}
