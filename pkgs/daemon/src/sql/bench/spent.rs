//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Benchmarks and seed helpers for address-domain indexed SQL.

use crate::error::DaemonError;
use hyphend::bench::BenchDataset;

const WHALE_ADDRESS: &str = "XbenchWhale1111111111111111111111111";
const PAGE_LIMIT: i64 = 100;

/// Prime the address-domain benchmark data.
///
/// # Errors
///
/// Returns an error when PostgreSQL rejects the seed writes.
pub async fn seed(pool: &crate::sql::SqlPool, dataset: BenchDataset) -> Result<(), DaemonError> {
  let (history_rows, balance_rows) = dataset_sizes(dataset);
  seed_blocks(pool, history_rows + 10).await?;
  seed_transactions_and_address_history(pool, history_rows).await?;
  seed_balances(pool, balance_rows).await?;
  Ok(())
}

fn dataset_sizes(dataset: BenchDataset) -> (i32, i32) {
  (history_rows(dataset), balance_rows(dataset))
}

fn history_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 4_000,
    BenchDataset::Default => 20_000,
    BenchDataset::Large => 60_000,
  }
}

fn balance_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 10_000,
    BenchDataset::Default => 50_000,
    BenchDataset::Large => 100_000,
  }
}

async fn seed_blocks(pool: &crate::sql::SqlPool, count: i32) -> Result<(), DaemonError> {
  for height in 0..count {
    sqlx::query(
      "INSERT INTO blocks (
         height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count, chainwork
       ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
       ON CONFLICT (height) DO NOTHING",
    )
    .bind(height)
    .bind(super::synthetic_hash(b'B', i64::from(height)))
    .bind(if height == 0 {
      vec![0_u8; 32]
    } else {
      super::synthetic_hash(b'B', i64::from(height - 1))
    })
    .bind(super::synthetic_hash(b'M', i64::from(height)))
    .bind(height)
    .bind(486_604_799_i32)
    .bind(i64::from(height))
    .bind(1_i32)
    .bind(1024_i32)
    .bind(4_i32)
    .bind(format!("{height:064x}"))
    .execute(pool)
    .await
    .map_err(|error| DaemonError::Pg(format!("seed spent bench block {height}: {error}")))?;
  }
  Ok(())
}

async fn seed_transactions_and_address_history(
  pool: &crate::sql::SqlPool,
  rows: i32,
) -> Result<(), DaemonError> {
  let mut tx = pool
    .begin()
    .await
    .map_err(|error| DaemonError::Pg(format!("begin spent history seed: {error}")))?;
  for height in 0..rows {
    let txid = super::synthetic_hash(b'T', i64::from(height));
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
       ) VALUES ($1, $2, $3, $4, 'normal', $5, $6, $7, $8, $9, $10)
       ON CONFLICT (txid) DO NOTHING",
    )
    .bind(&txid)
    .bind(height)
    .bind(0_i32)
    .bind(3_i32)
    .bind(0_i32)
    .bind(250_i32)
    .bind(1_000_i64)
    .bind(1_i32)
    .bind(2_i32)
    .bind(Some(10_i64))
    .execute(&mut *tx)
    .await
    .map_err(|error| DaemonError::Pg(format!("seed spent tx {height}: {error}")))?;
    sqlx::query(
      "INSERT INTO address_txs (address, txid, block_height, block_pos)
       VALUES ($1, $2, $3, $4)
       ON CONFLICT DO NOTHING",
    )
    .bind(WHALE_ADDRESS)
    .bind(&txid)
    .bind(height)
    .bind(0_i32)
    .execute(&mut *tx)
    .await
    .map_err(|error| DaemonError::Pg(format!("seed spent address_txs {height}: {error}")))?;
  }
  sqlx::query(
    "INSERT INTO address_stats (
       address, balance, total_received, tx_count, first_seen_height, last_seen_height
     ) VALUES ($1, $2, $3, $4, $5, $6)
     ON CONFLICT (address) DO NOTHING",
  )
  .bind(WHALE_ADDRESS)
  .bind(5_000_000_i64)
  .bind(6_000_000_i64)
  .bind(rows)
  .bind(0_i32)
  .bind(rows - 1)
  .execute(&mut *tx)
  .await
  .map_err(|error| DaemonError::Pg(format!("seed spent whale summary: {error}")))?;
  tx.commit()
    .await
    .map_err(|error| DaemonError::Pg(format!("commit spent history seed: {error}")))?;
  Ok(())
}

async fn seed_balances(pool: &crate::sql::SqlPool, rows: i32) -> Result<(), DaemonError> {
  let mut tx = pool
    .begin()
    .await
    .map_err(|error| DaemonError::Pg(format!("begin spent balances seed: {error}")))?;
  let mut included_supply = 0_i64;
  for index in 0..rows {
    let balance = i64::from(rows - index) * 1_000;
    sqlx::query(
      "INSERT INTO address_stats (
         address, balance, total_received, tx_count, first_seen_height, last_seen_height
       ) VALUES ($1, $2, $3, $4, $5, $6)
       ON CONFLICT (address) DO NOTHING",
    )
    .bind(format!("Xbench{index:05}"))
    .bind(balance)
    .bind(balance + 500)
    .bind(10_i32)
    .bind(0_i32)
    .bind(100_i32)
    .execute(&mut *tx)
    .await
    .map_err(|error| DaemonError::Pg(format!("seed spent balance row {index}: {error}")))?;
    sqlx::query(
      "INSERT INTO address_balances (address, balance)
       VALUES ($1, $2)
       ON CONFLICT (address) DO NOTHING",
    )
    .bind(format!("Xbench{index:05}"))
    .bind(balance)
    .execute(&mut *tx)
    .await
    .map_err(|error| {
      DaemonError::Pg(format!("seed spent address_balances row {index}: {error}"))
    })?;
    included_supply += balance;
  }
  sqlx::query(
    "INSERT INTO address_balances_summary (singleton, total, included_supply)
     VALUES (TRUE, $1, $2)
     ON CONFLICT (singleton) DO UPDATE SET
       total = EXCLUDED.total,
       included_supply = EXCLUDED.included_supply",
  )
  .bind(i64::from(rows))
  .bind(included_supply)
  .execute(&mut *tx)
  .await
  .map_err(|error| DaemonError::Pg(format!("seed spent balances summary: {error}")))?;
  tx.commit()
    .await
    .map_err(|error| DaemonError::Pg(format!("commit spent balances seed: {error}")))?;
  Ok(())
}
