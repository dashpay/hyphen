//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! End-to-end fidelity tests for the read path. Each test provisions its own
//! ephemeral database (so runs are isolated and parallel-safe), applies the
//! crate's migrations via [`HyphenDb::connect`], seeds a minimal chain with raw
//! SQL, then exercises the public query API.
//!
//! The suite targets `HYPHEN_TEST_DATABASE_URL`, or the default local server
//! when it is unset. An unreachable server fails every test in it: green here
//! means the read path ran, never that it was skipped.

mod common;

use common::TempDb;
use hyphen_db::query::HyDbSearchKind;
use hyphen_db::{HyDbHash, HyDbHeight, HyDbVlType};
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Seed one canonical block at height 1 with a single coinbase-style tx that
/// creates one unspent output to `address`.
async fn seed_minimal_chain(pool: &PgPool, txid: &[u8], address: &str, value: i64) -> TestResult {
  // Partitions for the height-ranged tables (height 1 falls in [0, 100000)).
  for stmt in [
    "CREATE TABLE transactions_p0 PARTITION OF transactions FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE tx_inputs_p0 PARTITION OF tx_inputs FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE tx_outputs_p0 PARTITION OF tx_outputs FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE address_txs_p0 PARTITION OF address_txs FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE spends_p0 PARTITION OF spends FOR VALUES FROM (0) TO (100000)",
  ] {
    sqlx::query(stmt).execute(pool).await?;
  }

  let block_hash = vec![0x11u8; 32];
  let prev_hash = vec![0x00u8; 32];
  let merkle = txid.to_vec();

  sqlx::query("INSERT INTO ser_blocks (hash, prev_hash, height, raw) VALUES ($1,$2,$3,$4)")
    .bind(&block_hash)
    .bind(&prev_hash)
    .bind(1i32)
    .bind(vec![0xdeu8, 0xad, 0xbe, 0xef])
    .execute(pool)
    .await?;

  sqlx::query(
    "INSERT INTO blocks (height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count)
     VALUES (1,$1,$2,$3,1600000000,0x1e0fffff,0,1,285,1)",
  )
  .bind(&block_hash)
  .bind(&prev_hash)
  .bind(&merkle)
  .execute(pool)
  .await?;

  sqlx::query(
    "INSERT INTO transactions (txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee)
     VALUES ($1,1,0,1,'coinbase',0,200,$2,1,1,NULL)",
  )
  .bind(txid)
  .bind(value)
  .execute(pool)
  .await?;

  sqlx::query(
    "INSERT INTO tx_outputs (block_height, txid, vout_index, value, script_pubkey, address, address_type)
     VALUES (1,$1,0,$2,'\\x76a914',$3,'p2pkh')",
  )
  .bind(txid)
  .bind(value)
  .bind(address)
  .execute(pool)
  .await?;

  // outpoint = txid || int4send(0)
  let mut outpoint = txid.to_vec();
  outpoint.extend_from_slice(&0i32.to_be_bytes());
  sqlx::query(
    "INSERT INTO utxo_set (outpoint, address, value, block_height, script_pubkey)
     VALUES ($1,$2,$3,1,'\\x76a914')",
  )
  .bind(&outpoint)
  .bind(address)
  .bind(value)
  .execute(pool)
  .await?;

  sqlx::query(
    "INSERT INTO address_txs (address, block_height, txid, block_pos) VALUES ($1,1,$2,0)",
  )
  .bind(address)
  .bind(txid)
  .execute(pool)
  .await?;

  sqlx::query(
    "INSERT INTO address_stats (address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height)
     VALUES ($1,$2,$2,1,1,1,1)",
  )
  .bind(address)
  .bind(value)
  .execute(pool)
  .await?;

  sqlx::query("UPDATE transactions_summary SET total = 1 WHERE singleton = TRUE")
    .execute(pool)
    .await?;
  sqlx::query("UPDATE supply_summary SET total = $1, included_supply = $1 WHERE singleton = TRUE")
    .bind(value)
    .execute(pool)
    .await?;

  Ok(())
}

#[tokio::test]
async fn read_path_round_trips_a_seeded_chain() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_read_path(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_read_path(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  let txid = [0xabu8; 32];
  let address = "yTestAddr0000000000000000000000000";
  let value = 500_000_000i64;
  seed_minimal_chain(&pool, &txid, address, value).await?;

  // Blocks
  let detail = db.blocks().by_height(HyDbHeight::new(1)).await?;
  assert!(detail.is_some(), "block 1 should exist");
  let list = db.blocks().list(10, None).await?;
  assert_eq!(list.items.len(), 1, "one block listed");
  assert!(list.next_cursor.is_none(), "single page");
  let tip = db.status().tip().await?;
  assert_eq!(tip.map(|t| t.height), Some(HyDbHeight::new(1)));

  // Transactions
  let tx = db.txs().get(HyDbHash(txid)).await?;
  let tx = tx.ok_or("tx should exist")?;
  assert_eq!(tx.outputs.len(), 1);
  assert_eq!(tx.outputs[0].value, value);
  assert!(tx.outputs[0].spent_by.is_none(), "output is unspent");
  assert_eq!(db.txs().total().await?, 1);

  // Addresses
  let summary = db
    .addrs()
    .summary(hyphen_db::HyDbAddress(address.to_owned()))
    .await?;
  let summary = summary.ok_or("address should exist")?;
  assert_eq!(summary.balance, value);
  assert_eq!(summary.utxo_count, 1);

  let utxos = db
    .addrs()
    .utxos(hyphen_db::HyDbAddress(address.to_owned()), 10, None)
    .await?;
  assert_eq!(utxos.items.len(), 1);
  assert_eq!(utxos.items[0].outpoint.txid, HyDbHash(txid));
  assert_eq!(utxos.items[0].value, value);

  let history = db
    .addrs()
    .history(hyphen_db::HyDbAddress(address.to_owned()), 10, None)
    .await?;
  assert_eq!(history.items.len(), 1);
  assert_eq!(history.items[0].txid, HyDbHash(txid));

  let supply = db.addrs().supply().await?;
  assert_eq!(supply.total, value);

  // Search finds the address by prefix.
  let hits = db.search().prefix(address[..6].to_owned(), 10).await?;
  assert!(
    hits
      .iter()
      .any(|h| h.kind == HyDbSearchKind::Address && h.id == address),
    "search should surface the seeded address"
  );

  Ok(())
}

#[tokio::test]
async fn empty_database_reports_no_tip() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = async {
    let db = tmp.open().await?;
    assert!(db.status().tip().await?.is_none(), "empty db has no tip");
    assert!(db.blocks().list(10, None).await?.items.is_empty());
    assert_eq!(db.txs().total().await?, 0);
    Ok::<(), Box<dyn std::error::Error>>(())
  }
  .await;
  tmp.drop_now().await;
  result
}

/// The validator `list`/`get` path decodes with a runtime `FromRow`, so this
/// exercises that column-name → field mapping end-to-end (a positional mismap
/// would surface as a swapped/mistyped field here rather than at compile time).
#[tokio::test]
async fn validator_round_trips_through_list_and_get() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run_validators(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_validators(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  let protx = [0x22u8; 32];
  let collateral = [0x33u8; 32];
  sqlx::query(
    "INSERT INTO validators
       (protx_hash, collateral_hash, collateral_index, owner_address,
        addr_core_p2p, validator_type, status, registered_height, last_updated_height)
     VALUES ($1, $2, 3, 'yOwnerAddr', ARRAY['1.2.3.4:9999'], 'evonode', 'registered', 5, 7)",
  )
  .bind(&protx[..])
  .bind(&collateral[..])
  .execute(&pool)
  .await?;

  // get: every field lands in the right place (protx_hash vs collateral_hash
  // are both bytea, so a positional swap would pass types but fail here).
  let v = db
    .validators()
    .get(HyDbHash(protx))
    .await?
    .ok_or("validator should exist")?;
  assert_eq!(v.protx_hash, HyDbHash(protx));
  assert_eq!(v.collateral_hash, HyDbHash(collateral));
  assert_eq!(v.collateral_index, 3);
  assert_eq!(v.owner_address.map(|a| a.0), Some("yOwnerAddr".to_owned()));
  assert_eq!(v.addr_core_p2p, vec!["1.2.3.4:9999".to_owned()]);
  assert_eq!(v.validator_type, HyDbVlType::Evonode);
  assert_eq!(v.registered_height, HyDbHeight::new(5));
  assert_eq!(v.last_updated_height, HyDbHeight::new(7));

  // list: unfiltered returns it; the type filter includes/excludes correctly.
  let all = db.validators().list(None, None, 10, None).await?;
  assert_eq!(all.items.len(), 1);
  assert_eq!(all.items[0].protx_hash, HyDbHash(protx));

  let evo = db
    .validators()
    .list(None, Some(HyDbVlType::Evonode), 10, None)
    .await?;
  assert_eq!(evo.items.len(), 1);
  let regular = db
    .validators()
    .list(None, Some(HyDbVlType::Regular), 10, None)
    .await?;
  assert!(
    regular.items.is_empty(),
    "the seeded validator is an evonode"
  );

  Ok(())
}
