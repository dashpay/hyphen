//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Stepped sync (task A3) against a real database.
//!
//! The state machine itself is unit-tested in `stepped.rs`, where it needs no
//! server. What needs one is everything the machine *does*: that the open
//! partition really loses the deferred set and a closed one really gets it
//! back, that the definitions it rebuilds are the ones the schema declared,
//! that nothing the ingest path reads is ever among them — and, the property
//! the whole feature stands or falls on, that a query returns the same rows in
//! either mode. A deferred index is allowed to change what a query costs. It
//! is not allowed to change what a query answers.

mod common;

use hyphen_db::stepped::{
  child_index_name, DEFERRED_INDEXES, INGEST_PATH_INDEXES, STEPPED_HOLD_ADVANCES,
};
use hyphen_db::{HyDbSyncMode, SteppedSync};
use sqlx::PgPool;

type Res = Result<(), Box<dyn std::error::Error>>;

const SPAN: i32 = 100_000;

/// Create the five height partitions covering partition index `idx`.
async fn create_partitions(pool: &PgPool, idx: i32) -> Res {
  let lo = idx * SPAN;
  let hi = lo + SPAN;
  for parent in [
    "transactions",
    "tx_inputs",
    "tx_outputs",
    "address_txs",
    "spends",
  ] {
    sqlx::query(&format!(
      "CREATE TABLE IF NOT EXISTS {parent}_p{idx} PARTITION OF {parent} \
       FOR VALUES FROM ({lo}) TO ({hi})"
    ))
    .execute(pool)
    .await?;
  }
  Ok(())
}

/// Which of `names` exist as relations.
async fn present(pool: &PgPool, names: &[String]) -> Result<Vec<String>, sqlx::Error> {
  sqlx::query_scalar::<_, String>(
    "SELECT n FROM UNNEST($1::text[]) AS n WHERE to_regclass(n) IS NOT NULL",
  )
  .bind(names)
  .fetch_all(pool)
  .await
}

/// The deferred-index names for one partition.
fn deferred_names(partition: i32) -> Vec<String> {
  DEFERRED_INDEXES
    .iter()
    .map(|spec| child_index_name(spec.stem, partition))
    .collect()
}

async fn assert_deferred_present(pool: &PgPool, partition: i32, want: bool) -> Res {
  let names = deferred_names(partition);
  let live = present(pool, &names).await?;
  if want {
    assert_eq!(
      live.len(),
      names.len(),
      "partition p{partition} should carry the whole deferred set; it has {live:?}"
    );
  } else {
    assert!(
      live.is_empty(),
      "partition p{partition} is the open one and should carry none of the \
       deferred set; it has {live:?}"
    );
  }
  Ok(())
}

/// Drive the machine over the entry mark until it commits.
fn into_stepped(sync: &SteppedSync) {
  for _ in 0..STEPPED_HOLD_ADVANCES {
    sync.observe(Some(150_000), 1_000_000);
  }
  assert_eq!(sync.mode(), HyDbSyncMode::Stepped);
}

/// A partition stops being the open one when the tip moves past it, and that
/// is the only moment its deferred indexes are built. Both halves are asserted
/// here: the open partition carries none of them, every closed one carries all
/// of them, and moving the tip across a boundary moves a partition from the
/// first group to the second.
#[tokio::test]
async fn a_partition_gains_the_deferred_set_when_it_closes() -> Res {
  let tmp = common::TempDb::create().await?;
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  create_partitions(&pool, 0).await?;
  create_partitions(&pool, 1).await?;

  let sync = SteppedSync::with_params(pool.clone(), true, 25_000);
  into_stepped(&sync);

  // Tip in p1: p0 is closed and gets built, p1 is open and gets nothing.
  sync.ensure_settled(150_000).await?;
  assert_deferred_present(&pool, 0, true).await?;
  assert_deferred_present(&pool, 1, false).await?;

  // The tip crosses into p2. That is p1's close.
  create_partitions(&pool, 2).await?;
  sync.ensure_settled(250_000).await?;
  assert_deferred_present(&pool, 0, true).await?;
  assert_deferred_present(&pool, 1, true).await?;
  assert_deferred_present(&pool, 2, false).await?;

  // A repeat call is a no-op rather than a rebuild.
  sync.ensure_settled(250_000).await?;
  assert_deferred_present(&pool, 2, false).await?;

  drop(db);
  tmp.drop_now().await;
  Ok(())
}

/// Presence is not correctness. A rebuilt index that indexes the wrong columns,
/// drops the partial predicate, or loses the DESC ordering would satisfy every
/// existence check here and quietly change which plans are available.
#[tokio::test]
async fn a_rebuilt_index_matches_the_definition_the_schema_declared() -> Res {
  let tmp = common::TempDb::create().await?;
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  create_partitions(&pool, 0).await?;
  create_partitions(&pool, 1).await?;
  let sync = SteppedSync::with_params(pool.clone(), true, 25_000);
  into_stepped(&sync);
  sync.ensure_settled(150_000).await?;

  for spec in &DEFERRED_INDEXES {
    let name = child_index_name(spec.stem, 0);
    let def: String = sqlx::query_scalar("SELECT indexdef FROM pg_indexes WHERE indexname = $1")
      .bind(&name)
      .fetch_one(&pool)
      .await?;
    // pg_indexes renders the definition canonically; the interesting part is
    // the key list and predicate, which must be the schema's.
    let tail = def
      .split_once(" USING btree ")
      .map(|(_, tail)| tail.to_owned())
      .unwrap_or_default();
    assert!(!tail.is_empty(), "{name} is not a btree index: {def}");
    for column in spec
      .definition
      .trim_start_matches('(')
      .split(')')
      .next()
      .unwrap_or_default()
      .split(", ")
    {
      assert!(
        tail.contains(column.trim()),
        "{name} should index `{column}`; Postgres reports {def}"
      );
    }
    if let Some(predicate) = spec.definition.split_once("WHERE ") {
      assert!(
        tail.contains(predicate.1),
        "{name} should keep the partial predicate `{}`; Postgres reports {def}",
        predicate.1
      );
    }
  }

  drop(db);
  tmp.drop_now().await;
  Ok(())
}

/// The safety property. Deferring an index the write path probes does not slow
/// a sync down, it stops it — the probe becomes a sequential scan of a table
/// that is still growing. So no run of stepped mode, in either direction, may
/// leave one of them missing.
#[tokio::test]
async fn no_index_the_ingest_path_reads_is_ever_dropped() -> Res {
  let tmp = common::TempDb::create().await?;
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  create_partitions(&pool, 0).await?;
  create_partitions(&pool, 1).await?;

  let unpartitioned: Vec<String> = INGEST_PATH_INDEXES
    .iter()
    .filter(|name| !name.starts_with("transactions_") && !name.starts_with("tx_"))
    .filter(|name| !name.starts_with("address_txs_"))
    .map(|name| (*name).to_owned())
    .collect();
  // The partitioned tables' own primary keys, per partition — the arbiters
  // every ON CONFLICT on the write path depends on.
  let mut partitioned: Vec<String> = Vec::new();
  for partition in 0..=1 {
    for table in ["transactions", "tx_inputs", "tx_outputs", "address_txs"] {
      partitioned.push(format!("{table}_p{partition}_pkey"));
    }
  }

  let baseline_unpartitioned = present(&pool, &unpartitioned).await?.len();
  assert_eq!(
    baseline_unpartitioned,
    unpartitioned.len(),
    "the fixture should start with every ingest-path index present"
  );

  let sync = SteppedSync::with_params(pool.clone(), true, 25_000);
  into_stepped(&sync);
  sync.ensure_settled(150_000).await?;
  create_partitions(&pool, 2).await?;
  sync.ensure_settled(250_000).await?;
  for _ in 0..STEPPED_HOLD_ADVANCES {
    if let Some(transition) = sync.observe(Some(999_000), 1_000_000) {
      sync.apply(transition).await?;
    }
  }
  assert_eq!(sync.mode(), HyDbSyncMode::Realtime);

  assert_eq!(
    present(&pool, &unpartitioned).await?.len(),
    unpartitioned.len(),
    "stepped sync dropped an index the ingest path reads"
  );
  assert_eq!(
    present(&pool, &partitioned).await?.len(),
    partitioned.len(),
    "stepped sync dropped a partition primary key"
  );

  drop(db);
  tmp.drop_now().await;
  Ok(())
}

/// Seed one transaction, its output, and its address edge at `height`.
async fn seed(pool: &PgPool, height: i32, seed: u8, address: &str) -> Res {
  let txid = vec![seed; 32];
  let block_hash = vec![seed ^ 0xff; 32];
  let prev_hash = vec![seed ^ 0xfe; 32];
  sqlx::query(
    "INSERT INTO blocks (height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count)
     VALUES ($1,$2,$3,$4,1600000000,0x1e0fffff,0,1,285,1)",
  )
  .bind(height)
  .bind(&block_hash)
  .bind(&prev_hash)
  .bind(&txid)
  .execute(pool)
  .await?;
  sqlx::query(
    "INSERT INTO transactions (txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee)
     VALUES ($1,$2,0,1,'coinbase',0,200,1000,1,1,NULL)",
  )
  .bind(&txid)
  .bind(height)
  .execute(pool)
  .await?;
  sqlx::query(
    "INSERT INTO tx_outputs (block_height, txid, vout_index, value, script_pubkey, address, address_type)
     VALUES ($1,$2,0,1000,'\\x76a914',$3,'p2pkh')",
  )
  .bind(height)
  .bind(&txid)
  .bind(address)
  .execute(pool)
  .await?;
  sqlx::query(
    "INSERT INTO address_txs (address, block_height, txid, block_pos) VALUES ($1,$2,$3,0)",
  )
  .bind(address)
  .bind(height)
  .bind(&txid)
  .execute(pool)
  .await?;
  sqlx::query(
    "INSERT INTO address_stats (address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height)
     VALUES ($1,1000,1000,1,1,$2,$2)
     ON CONFLICT (address) DO UPDATE SET tx_count = address_stats.tx_count + 1, last_seen_height = EXCLUDED.last_seen_height",
  )
  .bind(address)
  .bind(height)
  .execute(pool)
  .await?;
  Ok(())
}

/// The property the feature is only acceptable under: deferring an index
/// changes what a query costs and nothing else.
///
/// Rows live in both a closed partition (indexed either way) and the open one
/// (indexed in realtime, not in stepped), and the queries chosen are the ones
/// the deferred set exists to serve — the txid lookup, the newest-first list,
/// the display-hex prefix search, and the address history. Each is asked in
/// stepped mode and again after the mode leaves, and the answers must be
/// identical.
#[tokio::test]
async fn a_query_answers_the_same_in_stepped_mode_as_in_realtime() -> Res {
  let tmp = common::TempDb::create().await?;
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  create_partitions(&pool, 0).await?;
  create_partitions(&pool, 1).await?;

  const ADDRESS: &str = "XjUQQHuVvrXqRHqmWarqmGsd79HbeUAqN7";
  // Two rows in the partition that will be closed, two in the one that will
  // be open — so a wrong answer in stepped mode has somewhere to come from.
  seed(&pool, 10, 0x11, ADDRESS).await?;
  seed(&pool, 20, 0x22, ADDRESS).await?;
  seed(&pool, 100_010, 0x33, ADDRESS).await?;
  seed(&pool, 100_020, 0x44, ADDRESS).await?;

  let sync = SteppedSync::with_params(pool.clone(), true, 25_000);
  into_stepped(&sync);
  sync.ensure_settled(100_020).await?;
  assert_deferred_present(&pool, 0, true).await?;
  assert_deferred_present(&pool, 1, false).await?;

  let stepped_answers = answers(&db, ADDRESS).await?;

  // Leave stepped mode, which rebuilds the open partition's set...
  for _ in 0..STEPPED_HOLD_ADVANCES {
    if let Some(transition) = sync.observe(Some(999_000), 1_000_000) {
      sync.apply(transition).await?;
    }
  }
  assert_eq!(sync.mode(), HyDbSyncMode::Realtime);
  // ...which is what makes the second run a genuinely different physical plan
  // rather than a repeat of the first.
  assert_deferred_present(&pool, 1, true).await?;

  let realtime_answers = answers(&db, ADDRESS).await?;
  assert_eq!(
    stepped_answers, realtime_answers,
    "stepped mode is allowed to change what a query costs, never what it answers"
  );
  // Guard against the vacuous version of this test, where both modes agree
  // because both found nothing.
  assert!(
    stepped_answers[0].starts_with("Some("),
    "the txid lookup should have found the transaction in the open partition, \
     got {}",
    stepped_answers[0]
  );
  for (query, answer) in ["list", "prefix search", "address history"]
    .iter()
    .zip(&stepped_answers[1..])
  {
    assert!(
      !answer.contains("items: []") && *answer != "[]",
      "the {query} should have returned rows, got {answer}"
    );
  }

  drop(db);
  tmp.drop_now().await;
  Ok(())
}

/// Ask each query the deferred set serves, rendered for comparison.
async fn answers(
  db: &hyphen_db::HyphenDb,
  address: &str,
) -> Result<Vec<String>, Box<dyn std::error::Error>> {
  use hyphen_db::types::{HyDbAddress, HyDbHash};

  let mut out = Vec::new();

  // idx_transactions_txid — the point lookup.
  let detail = db.txs().get(HyDbHash([0x33u8; 32])).await?;
  out.push(format!("{detail:?}"));

  // idx_transactions_list — the newest-first page.
  let list = db.txs().list(10, None).await?;
  out.push(format!("{list:?}"));

  // idx_transactions_display_txid — the prefix search.
  let hits = db.txs().search_prefix("33".to_owned(), 10).await?;
  out.push(format!("{hits:?}"));

  // idx_address_txs_addr_height_desc — the address history.
  let history = db
    .addrs()
    .history(HyDbAddress(address.to_owned()), 10, None)
    .await?;
  out.push(format!("{history:?}"));

  Ok(out)
}
