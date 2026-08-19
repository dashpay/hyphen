//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Stepped sync against a real database.
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
    // Uniqueness is part of the definition, and for `address_txs` it is the
    // whole reason the index survived: it carries the constraint the dropped
    // primary key used to. A rebuild that quietly drops UNIQUE leaves the
    // table with no key at all.
    assert_eq!(
      def.starts_with("CREATE UNIQUE INDEX"),
      spec.unique,
      "{name} should{} be unique; Postgres reports {def}",
      if spec.unique { "" } else { " not" }
    );
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

/// A name match is not evidence that an index enforces anything.
///
/// `reconcile` decided presence with `to_regclass`, which matches any relation
/// of that name and says nothing about `indisunique`. An index left plain under
/// the unique member's name therefore read as satisfied, and the close was
/// `CREATE UNIQUE INDEX IF NOT EXISTS`, which is a silent no-op against an
/// existing name. Together those left `address_txs` — whose primary key the
/// schema deliberately withholds on the strength of this build — with no
/// uniqueness constraint at all, and nothing to notice it.
///
/// Reachable without an operator: a database synced by a build that carried
/// this index as non-unique keeps the plain twin forever, and reconcile had no
/// way to see the drift.
#[tokio::test]
async fn a_non_unique_index_under_the_unique_members_name_is_rebuilt() -> Res {
  let tmp = common::TempDb::create().await?;
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  create_partitions(&pool, 0).await?;
  create_partitions(&pool, 1).await?;

  let spec = DEFERRED_INDEXES
    .iter()
    .find(|d| d.unique)
    .ok_or("the deferred set must have a unique member")?;
  let name = child_index_name(spec.stem, 0);

  // Stand in for the impostor: the right name and columns, no uniqueness.
  sqlx::query(&format!(
    "CREATE INDEX {name} ON address_txs_p0 {}",
    spec.definition
  ))
  .execute(&pool)
  .await?;

  let unique_now = |pool: PgPool, name: String| async move {
    sqlx::query_scalar::<_, bool>(
      "SELECT i.indisunique FROM pg_class c JOIN pg_index i ON i.indexrelid = c.oid \
       WHERE c.oid = to_regclass($1)",
    )
    .bind(name)
    .fetch_one(&pool)
    .await
  };
  assert!(
    !unique_now(pool.clone(), name.clone()).await?,
    "the impostor must start non-unique, or this test proves nothing"
  );

  // p0 is closed at this tip, so reconcile must repair it rather than accept
  // the name.
  let sync = SteppedSync::with_params(pool.clone(), true, 25_000);
  into_stepped(&sync);
  sync.ensure_settled(150_000).await?;

  assert!(
    unique_now(pool.clone(), name.clone()).await?,
    "a closed partition's unique member must end up UNIQUE, not merely present"
  );

  // ...and it really enforces, not just reports.
  seed(&pool, 10, 0xA1, "XyTestAddrDup").await?;
  let dup = sqlx::query(
    "INSERT INTO address_txs (address, block_height, block_pos, txid) \
     SELECT address, block_height, block_pos, txid FROM address_txs_p0 LIMIT 1",
  )
  .execute(&pool)
  .await;
  assert!(
    dup.is_err(),
    "the rebuilt index must refuse a duplicate row"
  );

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
  // every ON CONFLICT on the write path depends on. `address_txs` is absent
  // deliberately: it has no primary key, that being a redundant key prefix of
  // idx_address_txs_addr_height_desc, so there is no such index to check for,
  // and asserting on one would assert the old schema back into existence.
  let mut partitioned: Vec<String> = Vec::new();
  for partition in 0..=1 {
    for table in ["transactions", "tx_inputs", "tx_outputs"] {
      partitioned.push(format!("{table}_p{partition}_pkey"));
    }
  }
  // And it really is gone, on every partition, rather than merely unlisted.
  let dropped: Vec<String> = (0..=1).map(|p| format!("address_txs_p{p}_pkey")).collect();
  assert!(
    present(&pool, &dropped).await?.is_empty(),
    "address_txs must carry no primary key"
  );

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

/// The guarantee that makes the consolidation safe rather than merely small.
///
/// The schema declares no `address_txs_pkey` — it was a redundant key prefix
/// of `idx_address_txs_addr_height_desc` — and the surviving index is built
/// `UNIQUE`. But that index is *deferred* during stepped sync, so while a
/// partition is open nothing enforces the key at all; the enforcement is the
/// `CREATE UNIQUE INDEX` at partition close, which both builds the index and
/// retroactively proves the partition holds no duplicate.
///
/// That is only a guarantee if the build really does fail. So this injects the
/// duplicate the writer's dedup exists to prevent, straight into the open
/// partition where nothing can stop it, and closes the partition:
///
/// * the duplicate really lands — two rows, which is the exposure the design
///   accepts and the thing that makes the rest of this test non-vacuous;
/// * closing the partition fails, and the failure names the partition;
/// * the index is not left behind in some half-built or non-unique state;
/// * and once the duplicate is removed the close succeeds, so the failure is
///   a stop-and-fix, not a wedge.
#[tokio::test]
async fn closing_a_partition_that_holds_a_duplicate_fails_and_names_it() -> Res {
  let tmp = common::TempDb::create().await?;
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  create_partitions(&pool, 0).await?;
  create_partitions(&pool, 1).await?;

  let sync = SteppedSync::with_params(pool.clone(), true, 25_000);
  into_stepped(&sync);
  sync.ensure_settled(150_000).await?;
  // p1 is open, so it carries no unique index and nothing guards the key.
  assert_deferred_present(&pool, 1, false).await?;

  const ADDRESS: &str = "XjUQQHuVvrXqRHqmWarqmGsd79HbeUAqN7";
  const HEIGHT: i32 = 150_000;
  let txid = vec![0x5au8; 32];
  // A real duplicate is two rows agreeing on all four columns: `block_pos` is
  // a copy of the value `transactions` holds under its own (block_height,
  // txid) primary key, so a duplicated (address, block_height, txid) carries
  // the same position with it.
  for _ in 0..2 {
    sqlx::query(
      "INSERT INTO address_txs (address, block_height, txid, block_pos) VALUES ($1,$2,$3,7)",
    )
    .bind(ADDRESS)
    .bind(HEIGHT)
    .bind(&txid)
    .execute(&pool)
    .await?;
  }
  let (rows,): (i64,) = sqlx::query_as(
    "SELECT COUNT(*) FROM address_txs WHERE address = $1 AND block_height = $2 AND txid = $3",
  )
  .bind(ADDRESS)
  .bind(HEIGHT)
  .bind(&txid)
  .fetch_one(&pool)
  .await?;
  assert_eq!(
    rows, 2,
    "the open partition has to actually accept the duplicate, or this test \
     proves nothing about the build that is supposed to catch it"
  );

  // The tip crosses into p2, which is p1's close and therefore the build.
  create_partitions(&pool, 2).await?;
  let closed = sync.ensure_settled(250_000).await;
  assert!(
    closed.is_err(),
    "closing a partition holding a duplicate key must fail, got {closed:?}"
  );
  let text = closed.err().map_or_else(String::new, |e| e.to_string());
  assert!(
    text.contains("p1"),
    "the failure has to say which partition is bad; got {text}"
  );
  assert!(
    text.contains("idx_address_txs_addr_height_desc_p1"),
    "the failure should name the index being built on that partition; got {text}"
  );
  assert!(
    text.contains("unique") || text.contains("UNIQUE") || text.contains("duplicate"),
    "the failure should say the key is not unique rather than look like a \
     build problem; got {text}"
  );
  // And Postgres' DETAIL, which names the key that is duplicated. `sqlx`'s own
  // Display drops it, so this is the assertion that keeps it plumbed through:
  // without it an operator is told a partition is bad and not which row.
  assert!(
    text.contains(ADDRESS),
    "the failure should carry the duplicated key itself; got {text}"
  );

  // Nothing half-built: a failed CREATE INDEX leaves no index behind, which is
  // exactly why these builds are plain rather than CONCURRENTLY.
  let live = present(
    &pool,
    &[child_index_name("idx_address_txs_addr_height_desc", 1)],
  )
  .await?;
  assert!(
    live.is_empty(),
    "a failed unique build must leave no index at all, got {live:?}"
  );

  // And the failure is recoverable: remove the duplicate and the same close
  // completes, index and all. A duplicate is a bug to fix, not a dead end.
  sqlx::query(
    "DELETE FROM address_txs WHERE ctid IN (
       SELECT ctid FROM address_txs
        WHERE address = $1 AND block_height = $2 AND txid = $3 OFFSET 1)",
  )
  .bind(ADDRESS)
  .bind(HEIGHT)
  .bind(&txid)
  .execute(&pool)
  .await?;
  sync.ensure_settled(250_000).await?;
  assert_deferred_present(&pool, 1, true).await?;
  let unique: bool = sqlx::query_scalar(
    "SELECT indexdef LIKE 'CREATE UNIQUE INDEX%' FROM pg_indexes WHERE indexname = $1",
  )
  .bind(child_index_name("idx_address_txs_addr_height_desc", 1))
  .fetch_one(&pool)
  .await?;
  assert!(
    unique,
    "the index the close builds is what enforces the key; it must be unique"
  );

  drop(db);
  tmp.drop_now().await;
  Ok(())
}

/// The other half of the same guarantee: near the tip the index exists, so the
/// duplicate is refused per row at insert time rather than at a partition
/// boundary. Enforcement granularity changes with the mode; enforcement does
/// not disappear.
#[tokio::test]
async fn a_duplicate_is_refused_per_row_once_the_index_is_back() -> Res {
  let tmp = common::TempDb::create().await?;
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  create_partitions(&pool, 0).await?;
  let sync = SteppedSync::with_params(pool.clone(), true, 25_000);
  // Realtime from the start: p0 is the open partition and still gets the set.
  sync.ensure_settled(10).await?;
  assert_deferred_present(&pool, 0, true).await?;

  const ADDRESS: &str = "XjUQQHuVvrXqRHqmWarqmGsd79HbeUAqN7";
  let txid = vec![0x5au8; 32];
  let insert = |pool: PgPool, txid: Vec<u8>| async move {
    sqlx::query(
      "INSERT INTO address_txs (address, block_height, txid, block_pos) VALUES ($1,10,$2,7)",
    )
    .bind(ADDRESS)
    .bind(txid)
    .execute(&pool)
    .await
  };
  insert(pool.clone(), txid.clone()).await?;
  let second = insert(pool.clone(), txid.clone()).await;
  let code = second
    .as_ref()
    .err()
    .and_then(sqlx::Error::as_database_error)
    .and_then(|e| e.code())
    .map(|c| c.to_string());
  assert_eq!(
    code.as_deref(),
    Some("23505"),
    "a duplicate key should be refused as a unique violation, got {second:?}"
  );

  drop(db);
  tmp.drop_now().await;
  Ok(())
}
