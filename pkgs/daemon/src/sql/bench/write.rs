//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Benchmarks and seed helpers for write-plane SQL operations.

use crate::error::DaemonError;
use crate::index::Stage;
use hyphend::bench::BenchDataset;
use crate::sql::pg_err;

/// Prime write benchmark baseline state.
///
/// # Errors
///
/// Returns an error when PostgreSQL rejects the seed writes.
pub async fn seed(pool: &crate::sql::SqlPool, dataset: BenchDataset) -> Result<(), DaemonError> {
  let mut tx = pool
    .begin()
    .await
    .map_err(|error| pg_err("begin write seed tx", error))?;
  crate::sql::index::block::insert_block(
    &mut tx,
    1,
    super::synthetic_hash(b'W', 1).as_slice(),
    vec![0_u8; 32].as_slice(),
    super::synthetic_hash(b'M', 1).as_slice(),
    1_700_000_001,
    486_604_799,
    1,
    3,
    1024,
    1,
    format!("{:064x}", 1).as_str(),
    &crate::index::block::util::CbTxFields::default(),
  )
  .await?;
  crate::sql::sync::update_indexed_stage_height_tx(&mut tx, Stage::Block, 100, 1000).await?;
  crate::sql::sync::update_indexed_stage_height_tx(&mut tx, Stage::Transaction, 100, 1000).await?;
  crate::sql::sync::update_indexed_stage_height_tx(&mut tx, Stage::Script, 100, 1000).await?;
  crate::sql::sync::update_indexed_stage_height_tx(&mut tx, Stage::Spent, 100, 1000).await?;
  tx.commit()
    .await
    .map_err(|error| pg_err("commit write seed tx", error))?;
  let seeded_delta_rows = delta_rows(dataset);
  let addresses: Vec<String> = (0..seeded_delta_rows)
    .map(|index| format!("Xdelta{index:05}"))
    .collect();
  let balances: Vec<i64> = vec![1_000; seeded_delta_rows as usize];
  let received: Vec<i64> = vec![2_000; seeded_delta_rows as usize];
  let tx_counts: Vec<i32> = vec![1; seeded_delta_rows as usize];
  let utxo_counts: Vec<i32> = vec![1; seeded_delta_rows as usize];
  let first_seen: Vec<Option<i32>> = vec![Some(1); seeded_delta_rows as usize];
  let last_seen: Vec<Option<i32>> = vec![Some(1); seeded_delta_rows as usize];
  let mut tx = pool
    .begin()
    .await
    .map_err(|error| pg_err("begin write address seed tx", error))?;
  crate::sql::index::spent::apply_address_stats_chunk(
    &mut tx,
    &addresses,
    &balances,
    &received,
    &tx_counts,
    &utxo_counts,
    &first_seen,
    &last_seen,
  )
  .await?;
  tx.commit()
    .await
    .map_err(|error| pg_err("commit write address seed tx", error))?;
  let existing_balance_rows = delta_rows(dataset) / 2;
  let mut tx = pool
    .begin()
    .await
    .map_err(|error| pg_err("begin write balance seed tx", error))?;
  for index in 0..existing_balance_rows {
    sqlx::query(
      "INSERT INTO address_balances (address, balance)
       VALUES ($1, $2)
       ON CONFLICT (address) DO NOTHING",
    )
    .bind(format!("XbalSeed{index:05}"))
    .bind(1_000_i64)
    .execute(&mut *tx)
    .await
    .map_err(|error| pg_err(&format!("seed write address_balances {index}"), error))?;
  }
  tx.commit()
    .await
    .map_err(|error| pg_err("commit write balance seed tx", error))?;
  Ok(())
}

fn delta_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 500,
    BenchDataset::Default => 2_000,
    BenchDataset::Large => 10_000,
  }
}
