//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! `txs::get` is bounded in both io dimensions, and everything past the bound
//! is reachable through the paged window.
//!
//! Input and output counts are attacker-chosen up to the block size limit, so
//! an unbounded `get` makes a summary render cost a row per output. The seed
//! here is deliberately wider than [`HYDB_TX_IO_CAP`] on both sides, which is
//! the only way the bound and the continuation are observable at all.
//!
//! Each test provisions its own ephemeral database and fails loudly if no
//! server is reachable — it never returns green without having run.

mod common;

use common::TempDb;
use hyphen_db::query::HYDB_TX_IO_CAP;
use hyphen_db::HyDbHash;
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// Outputs on the seeded transaction: wider than the cap, and not a multiple of
/// it, so the last page is partial.
const OUTPUTS: i32 = 2 * HYDB_TX_IO_CAP as i32 + 37;
/// Inputs on the seeded transaction: also past the cap, by a different amount.
const INPUTS: i32 = HYDB_TX_IO_CAP as i32 + 5;
/// Every 500th output is spent, so spent-status resolution is exercised on more
/// than the first page.
const SPEND_STRIDE: i32 = 500;

/// One block at height 1 holding a single deliberately-wide transaction.
async fn seed_wide_tx(pool: &PgPool, txid: &[u8]) -> TestResult {
  for stmt in [
    "CREATE TABLE transactions_p0 PARTITION OF transactions FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE tx_inputs_p0 PARTITION OF tx_inputs FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE tx_outputs_p0 PARTITION OF tx_outputs FOR VALUES FROM (0) TO (100000)",
    "CREATE TABLE spends_p0 PARTITION OF spends FOR VALUES FROM (0) TO (100000)",
  ] {
    sqlx::query(stmt).execute(pool).await?;
  }

  let block_hash = vec![0x11u8; 32];
  let prev_hash = vec![0x00u8; 32];
  sqlx::query(
    "INSERT INTO blocks (height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count)
     VALUES (1,$1,$2,$3,1600000000,0x1e0fffff,0,1,285,1)",
  )
  .bind(&block_hash)
  .bind(&prev_hash)
  .bind(txid)
  .execute(pool)
  .await?;

  sqlx::query(
    "INSERT INTO transactions (txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee)
     VALUES ($1,1,0,1,'normal',0,200,0,$2,$3,NULL)",
  )
  .bind(txid)
  .bind(INPUTS)
  .bind(OUTPUTS)
  .execute(pool)
  .await?;

  // Values carry the index so a row can be traced back to its position.
  sqlx::query(
    "INSERT INTO tx_outputs (block_height, txid, vout_index, value, script_pubkey, address, address_type)
     SELECT 1, $1, i, 1000 + i, '\\x76a914', 'Xaddr' || i, 'p2pkh'
       FROM generate_series(0, $2) AS i",
  )
  .bind(txid)
  .bind(OUTPUTS - 1)
  .execute(pool)
  .await?;

  sqlx::query(
    "INSERT INTO tx_inputs (block_height, txid, vin_index, prev_txid, prev_vout, script_sig, sequence, prev_address, prev_value)
     SELECT 1, $1, i, $1, i, '\\x00', 4294967295, 'Xprev' || i, 500 + i
       FROM generate_series(0, $2) AS i",
  )
  .bind(txid)
  .bind(INPUTS - 1)
  .execute(pool)
  .await?;

  // Spend every SPEND_STRIDE'th output, keyed by the packed outpoint.
  sqlx::query(
    "INSERT INTO spends (spend_height, spending_txid, spending_vin, outpoint)
     SELECT 1, $1, i, $1 || int4send(i)
       FROM generate_series(0, $2, $3) AS i",
  )
  .bind(txid)
  .bind(OUTPUTS - 1)
  .bind(SPEND_STRIDE)
  .execute(pool)
  .await?;

  Ok(())
}

#[tokio::test]
async fn a_wide_transaction_is_bounded_and_fully_pageable() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;
  let txid = vec![0x77u8; 32];
  seed_wide_tx(&pool, &txid).await?;
  let hash = HyDbHash::from_slice(&txid)?;
  let cap = HYDB_TX_IO_CAP as usize;

  // The bound itself: the wide sides come back at exactly the cap, lowest
  // index first, not at OUTPUTS/INPUTS.
  let Some(detail) = db.txs().get(hash).await? else {
    return Err("the seeded transaction must resolve".into());
  };
  assert_eq!(
    detail.outputs.len(),
    cap,
    "get must stop at the cap, not return all {OUTPUTS} outputs"
  );
  assert_eq!(
    detail.inputs.len(),
    cap,
    "get must stop at the cap, not return all {INPUTS} inputs"
  );
  assert_eq!(detail.outputs[0].vout_index, 0);
  assert_eq!(detail.outputs[cap - 1].vout_index, cap as i32 - 1);
  assert_eq!(detail.inputs[0].vin_index, 0);
  assert_eq!(detail.inputs[cap - 1].vin_index, cap as i32 - 1);

  // The window agrees with `get` on the overlapping range, so the two are one
  // ordering and not two.
  let first = db.txs().outputs(hash, 0, HYDB_TX_IO_CAP).await?;
  assert_eq!(first.items.len(), cap);
  for (a, b) in detail.outputs.iter().zip(&first.items) {
    assert_eq!(a.vout_index, b.vout_index);
    assert_eq!(a.value, b.value);
    assert_eq!(a.spent_by.is_some(), b.spent_by.is_some());
  }

  // Walk the outputs with a small page size. Every index appears exactly once,
  // in order, and the walk ends on its own.
  let mut seen: Vec<i32> = Vec::new();
  let mut cursor = Some(0u32);
  let mut pages = 0;
  while let Some(from) = cursor {
    let page = db.txs().outputs(hash, from, 250).await?;
    pages += 1;
    assert!(pages < 100, "the walk must terminate");
    assert_eq!(
      page.items.first().map(|o| o.vout_index),
      Some(from as i32),
      "a page starts at the index its cursor names"
    );
    for out in &page.items {
      assert_eq!(out.value, 1000 + i64::from(out.vout_index), "row identity");
      assert_eq!(
        out.spent_by.is_some(),
        out.vout_index % SPEND_STRIDE == 0,
        "spent-status resolves inside the window (vout {})",
        out.vout_index
      );
      seen.push(out.vout_index);
    }
    cursor = match page.next_cursor {
      None => None,
      Some(c) => Some(c.0.parse()?),
    };
  }
  assert_eq!(
    seen,
    (0..OUTPUTS).collect::<Vec<_>>(),
    "paging must cover every output exactly once, in order"
  );

  // Same for the inputs.
  let mut seen: Vec<i32> = Vec::new();
  let mut cursor = Some(0u32);
  while let Some(from) = cursor {
    let page = db.txs().inputs(hash, from, 400).await?;
    for input in &page.items {
      assert_eq!(input.prev_value, Some(500 + i64::from(input.vin_index)));
      seen.push(input.vin_index);
    }
    cursor = match page.next_cursor {
      None => None,
      Some(c) => Some(c.0.parse()?),
    };
  }
  assert_eq!(seen, (0..INPUTS).collect::<Vec<_>>());

  // The tail page is partial and terminal.
  let tail = db.txs().outputs(hash, OUTPUTS as u32 - 5, 250).await?;
  assert_eq!(tail.items.len(), 5);
  assert!(tail.next_cursor.is_none(), "the last page has no cursor");

  // Past the end, and an unknown txid, are both empty and terminal rather than
  // an error — the same "nothing here" `get` reports as `None`.
  let past = db.txs().outputs(hash, OUTPUTS as u32, 10).await?;
  assert!(past.items.is_empty() && past.next_cursor.is_none());
  let missing = db.txs().inputs(HyDbHash([0x99u8; 32]), 0, 10).await?;
  assert!(missing.items.is_empty() && missing.next_cursor.is_none());

  // A caller asking for more than the cap gets the cap, not a bigger page.
  let over = db.txs().outputs(hash, 0, HYDB_TX_IO_CAP * 10).await?;
  assert_eq!(over.items.len(), cap);
  assert_eq!(over.next_cursor.map(|c| c.0), Some(cap.to_string()));

  Ok(())
}
