//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Reading a transaction whose txid occurs more than once.
//!
//! Pre-BIP30 enforcement a txid can recur across blocks, which `PgTxIdx`
//! already accounts for: it pins the most recent occurrence and serves that
//! one. The outputs were pinned to it and the spent-status was not — the spend
//! was joined on the packed outpoint alone, and an outpoint is unique only
//! *within* an occurrence. So the older occurrence's spend annotated the newer
//! occurrence's output, and where both were spent the join returned the output
//! twice.
//!
//! The state is constructed by direct insert. It cannot be reached through
//! ingest, which is the point: nothing in the write path produces it, so
//! nothing in the write path would have caught the read being wrong.

mod common;

use common::TempDb;
use hyphen_db::HyDbHash;
use sqlx::PgPool;

type Res = Result<(), Box<dyn std::error::Error>>;

const SPAN: i32 = 100_000;

async fn create_partitions(pool: &PgPool, idx: i32) -> Res {
  let lo = idx * SPAN;
  let hi = lo + SPAN;
  for parent in ["transactions", "tx_outputs", "spends"] {
    sqlx::query(&format!(
      "CREATE TABLE IF NOT EXISTS {parent}_p{idx} PARTITION OF {parent} \
       FOR VALUES FROM ({lo}) TO ({hi})"
    ))
    .execute(pool)
    .await?;
  }
  Ok(())
}

/// One occurrence of `txid` at `height`, with a single output of `value`.
async fn put_occurrence(pool: &PgPool, txid: &[u8], height: i32, value: i64) -> Res {
  sqlx::query(
    "INSERT INTO transactions \
       (txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, \
        input_count, output_count) \
     VALUES ($1, $2, 0, 1, 'normal', 0, 100, $3, 0, 1)",
  )
  .bind(txid)
  .bind(height)
  .bind(value)
  .execute(pool)
  .await?;
  sqlx::query(
    "INSERT INTO tx_outputs \
       (block_height, txid, vout_index, value, script_pubkey, address, address_type) \
     VALUES ($1, $2, 0, $3, '\\x51'::bytea, 'XyRecurring', 'p2pkh')",
  )
  .bind(height)
  .bind(txid)
  .bind(value)
  .execute(pool)
  .await?;
  Ok(())
}

/// Spend vout 0 of `txid` at `spend_height`.
async fn put_spend(pool: &PgPool, txid: &[u8], spend_height: i32, spender: u8) -> Res {
  sqlx::query(
    "INSERT INTO spends (spend_height, spending_txid, spending_vin, outpoint) \
     VALUES ($1, $2, 0, $3 || int4send(0))",
  )
  .bind(spend_height)
  .bind(vec![spender; 32])
  .bind(txid)
  .execute(pool)
  .await?;
  Ok(())
}

/// The older occurrence's spend must not mark the newer occurrence's output.
#[tokio::test]
async fn a_spend_of_an_earlier_occurrence_does_not_mark_the_later_one() -> Res {
  let tmp = TempDb::create().await?;
  let result = run_earlier_spend(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_earlier_spend(tmp: &TempDb) -> Res {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;
  create_partitions(&pool, 0).await?;

  let txid = vec![0xAB; 32];
  // Occurrence one at 100, spent at 110. Occurrence two at 200, never spent.
  put_occurrence(&pool, &txid, 100, 500).await?;
  put_spend(&pool, &txid, 110, 0xC1).await?;
  put_occurrence(&pool, &txid, 200, 700).await?;

  let detail = db
    .txs()
    .get(HyDbHash([0xAB; 32]))
    .await?
    .ok_or("the transaction must be found")?;

  assert_eq!(
    detail.block_height.get(),
    200,
    "the most recent occurrence is the one served"
  );
  assert_eq!(
    detail.outputs.len(),
    1,
    "one occurrence has one output; got {:?}",
    detail.outputs
  );
  assert_eq!(detail.outputs[0].value, 700, "and it is the newer one's");
  assert!(
    detail.outputs[0].spent_by.is_none(),
    "the newer occurrence's output is unspent — the spend at 110 belongs to the \
     occurrence at 100, not this one"
  );

  drop(db);
  Ok(())
}

/// With both occurrences spent, the output is returned once, annotated by the
/// spend that belongs to the occurrence being served.
#[tokio::test]
async fn both_occurrences_spent_still_yields_one_output() -> Res {
  let tmp = TempDb::create().await?;
  let result = run_both_spent(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_both_spent(tmp: &TempDb) -> Res {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;
  create_partitions(&pool, 0).await?;

  let txid = vec![0xCD; 32];
  put_occurrence(&pool, &txid, 100, 500).await?;
  put_spend(&pool, &txid, 110, 0xD1).await?;
  put_occurrence(&pool, &txid, 200, 700).await?;
  put_spend(&pool, &txid, 210, 0xD2).await?;

  let detail = db
    .txs()
    .get(HyDbHash([0xCD; 32]))
    .await?
    .ok_or("the transaction must be found")?;

  assert_eq!(
    detail.outputs.len(),
    1,
    "the join must annotate the output, not multiply it; got {:?}",
    detail.outputs
  );
  let spender = detail.outputs[0]
    .spent_by
    .as_ref()
    .ok_or("the newer occurrence's output was spent at 210")?;
  assert_eq!(
    spender.txid.as_bytes()[0],
    0xD2,
    "the spend attributed must be the one at or after this occurrence"
  );

  drop(db);
  Ok(())
}
