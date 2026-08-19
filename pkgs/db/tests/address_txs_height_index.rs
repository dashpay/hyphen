//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The reversal delete on `address_txs` has an index to stand on (task C2).
//!
//! `DELETE FROM address_txs WHERE block_height = $1` runs once per reversed
//! block. Both pre-existing indexes on the table lead with `address`, so the
//! delete had nothing but partition pruning and scanned the whole partition.
//! Migration 0004 adds `idx_address_txs_block_height`.
//!
//! What is asserted here is presence and reachability, not latency: on a seeded
//! test partition a sequential scan is genuinely the cheaper plan, so the plan
//! is checked with `enable_seqscan` off — that answers "can the planner use it
//! for this predicate", which is the property the migration exists to provide.
//! The latency claim (337 ms of sequential scan against 0.03-0.07 ms of index
//! scan on an 860,000-block database) is recorded in the migration.

mod common;

use common::TempDb;
use sqlx::{PgPool, Row as _};

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// The parent index the migration creates.
const INDEX: &str = "idx_address_txs_block_height";

#[tokio::test]
async fn the_reversal_delete_has_a_height_index_on_every_partition() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run(tmp: &TempDb) -> TestResult {
  // `open()` applies the migration set, 0004 included.
  let _db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;

  // The parent index exists, is a btree, and leads with block_height alone.
  let parent = sqlx::query(
    "SELECT pg_get_indexdef(i.indexrelid) AS def
       FROM pg_index i JOIN pg_class c ON c.oid = i.indexrelid
      WHERE c.relname = $1",
  )
  .bind(INDEX)
  .fetch_one(&pool)
  .await?;
  let def: String = parent.try_get("def")?;
  assert!(
    def.contains("USING btree (block_height)"),
    "{INDEX} must be a btree on block_height alone, got {def}"
  );

  // A partition created after the migration inherits it: partitioned indexes
  // are re-created for every partition added, which is what makes the parent
  // index the right place for this (see 0003 for the same mechanism used in
  // reverse).
  assert_eq!(child_count(&pool).await?, 0, "no partitions yet");
  sqlx::query(
    "CREATE TABLE address_txs_p0 PARTITION OF address_txs FOR VALUES FROM (0) TO (100000)",
  )
  .execute(&pool)
  .await?;
  sqlx::query(
    "CREATE TABLE address_txs_p1 PARTITION OF address_txs FOR VALUES FROM (100000) TO (200000)",
  )
  .execute(&pool)
  .await?;
  assert_eq!(
    child_count(&pool).await?,
    2,
    "every partition must carry the index the disconnect delete probes"
  );

  // The planner can reach it for the delete's predicate. A seeded partition is
  // small enough that a sequential scan wins on cost, so the sequential
  // alternative is taken away rather than competed with.
  sqlx::query(
    "INSERT INTO address_txs (address, block_height, txid, block_pos)
     SELECT 'Xaddr' || (i % 50), i / 5, int4send(i) || int4send(i) || int4send(i)
            || int4send(i) || int4send(i) || int4send(i) || int4send(i) || int4send(i),
            i % 5
       FROM generate_series(1, 5000) AS i",
  )
  .execute(&pool)
  .await?;
  sqlx::query("ANALYZE address_txs").execute(&pool).await?;

  sqlx::query("SET enable_seqscan = off")
    .execute(&pool)
    .await?;
  let plan: String =
    sqlx::query_scalar::<_, String>("EXPLAIN DELETE FROM address_txs WHERE block_height = 700")
      .fetch_all(&pool)
      .await?
      .join("\n");
  // The plan names the partition's child index, which Postgres auto-names; ask
  // the catalog which one belongs to the parent rather than hard-coding it.
  let child: String = sqlx::query_scalar(
    "SELECT child.relname FROM pg_inherits h
       JOIN pg_class parent ON parent.oid = h.inhparent
       JOIN pg_class child ON child.oid = h.inhrelid
      WHERE parent.relname = $1 AND child.relname LIKE 'address\\_txs\\_p0%'",
  )
  .bind(INDEX)
  .fetch_one(&pool)
  .await?;
  assert!(
    plan.contains(&child) && plan.contains("Index Scan"),
    "the delete's predicate must be servable by {INDEX} (child {child}); plan was:\n{plan}"
  );
  // Pruning still applies on top of it: one partition, not both.
  assert!(
    plan.contains("address_txs_p0") && !plan.contains("address_txs_p1"),
    "the delete must still prune to the one partition; plan was:\n{plan}"
  );

  Ok(())
}

/// How many partitions carry a child of the parent index.
async fn child_count(pool: &PgPool) -> Result<i64, Box<dyn std::error::Error>> {
  Ok(
    sqlx::query_scalar(
      "SELECT count(*) FROM pg_inherits h
         JOIN pg_class parent ON parent.oid = h.inhparent
        WHERE parent.relname = $1",
    )
    .bind(INDEX)
    .fetch_one(pool)
    .await?,
  )
}
