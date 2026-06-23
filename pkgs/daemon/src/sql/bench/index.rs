//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Benchmarks and seed helpers for indexed block, transaction, validator, and metrics SQL.

use crate::error::DaemonError;
use hyphend::bench::BenchDataset;
use crate::sql::pg_err;

const PAGE_LIMIT: i64 = 100;
const CHART_TARGET_POINTS: i32 = 144;
const VALIDATOR_HISTORY_ROWS_PER_VALIDATOR: i32 = 3;

/// Prime indexed benchmark data.
///
/// # Errors
///
/// Returns an error when PostgreSQL rejects the seed writes.
pub async fn seed(pool: &crate::sql::SqlPool, dataset: BenchDataset) -> Result<(), DaemonError> {
  seed_blocks_transactions_metrics(pool, block_rows(dataset)).await?;
  seed_validators(pool, dataset).await?;
  Ok(())
}

fn block_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 12_000,
    BenchDataset::Default => 60_000,
    BenchDataset::Large => 120_000,
  }
}

fn validator_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 3_000,
    BenchDataset::Default => 12_000,
    BenchDataset::Large => 24_000,
  }
}

async fn seed_blocks_transactions_metrics(
  pool: &crate::sql::SqlPool,
  rows: i32,
) -> Result<(), DaemonError> {
  let mut tx = pool
    .begin()
    .await
    .map_err(|error| pg_err("begin index seed tx", error))?;
  for height in 0..rows {
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
    .bind(1_700_000_000_i32 + height)
    .bind(486_604_799_i32)
    .bind(i64::from(height))
    .bind(3_i32)
    .bind(2048_i32)
    .bind(3_i32)
    .bind(format!("{height:064x}"))
    .execute(&mut *tx)
    .await
    .map_err(|error| pg_err(&format!("seed index block {height}"), error))?;
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
    .bind(220_i32)
    .bind(2_000_i64)
    .bind(1_i32)
    .bind(2_i32)
    .bind(Some(10_i64))
    .execute(&mut *tx)
    .await
    .map_err(|error| pg_err(&format!("seed index tx {height}"), error))?;
    sqlx::query(
      "INSERT INTO tx_inputs (
         txid, vin_index, prev_txid, prev_vout, script_sig, sequence, prev_address, prev_value
       ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8)",
    )
    .bind(&txid)
    .bind(0_i32)
    .bind(if height == 0 {
      None::<Vec<u8>>
    } else {
      Some(super::synthetic_hash(b'T', i64::from(height - 1)))
    })
    .bind(if height == 0 {
      None::<i32>
    } else {
      Some(0_i32)
    })
    .bind(vec![0x51_u8])
    .bind(0_i64)
    .bind(if height == 0 {
      None::<String>
    } else {
      Some(format!("Xfrom{height:05}"))
    })
    .bind(if height == 0 {
      None::<i64>
    } else {
      Some(1_000_i64)
    })
    .execute(&mut *tx)
    .await
    .map_err(|error| pg_err(&format!("seed index input {height}"), error))?;
    sqlx::query(
      "INSERT INTO tx_outputs (
         txid, vout_index, block_height, value, script_pubkey, address, address_type, spent_txid, spent_vin, spent_height
       ) VALUES ($1, $2, $3, $4, $5, $6, 'p2pkh', NULL, NULL, NULL)",
    )
    .bind(&txid)
    .bind(0_i32)
    .bind(height)
    .bind(2_000_i64)
    .bind(vec![0x76_u8, 0xa9, 0x14])
    .bind(format!("Xto{height:05}"))
    .execute(&mut *tx)
    .await
    .map_err(|error| pg_err(&format!("seed index output {height}"), error))?;
    sqlx::query(
      "INSERT INTO block_metrics (
         height, tx_count, utxos_spendable, utxos_unspendable, blockchain_size, credit_pool_balance,
         circulating_supply, difficulty, hashrate
       ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
       ON CONFLICT (height) DO NOTHING",
    )
    .bind(height)
    .bind(i64::from(height) + 1)
    .bind(i64::from(height) * 2)
    .bind(i64::from(height))
    .bind(i64::from(height) * 4_096)
    .bind(Some(i64::from(height) * 10))
    .bind(18_000_000_000_000_i64 + i64::from(height) * 5_000)
    .bind(1234.5_f64)
    .bind(987_654.0_f64)
    .execute(&mut *tx)
    .await
    .map_err(|error| pg_err(&format!("seed index metrics {height}"), error))?;
  }
  tx.commit()
    .await
    .map_err(|error| pg_err("commit index seed tx", error))?;
  Ok(())
}

async fn seed_validators(
  pool: &crate::sql::SqlPool,
  dataset: BenchDataset,
) -> Result<(), DaemonError> {
  let mut tx = pool
    .begin()
    .await
    .map_err(|error| pg_err("begin validator seed tx", error))?;
  for index in 0..validator_rows(dataset) {
    let pro_tx_hash = super::synthetic_hash(b'V', i64::from(index));
    sqlx::query(
      "INSERT INTO validators (
         pro_tx_hash, collateral_hash, collateral_index, collateral_address, owner_address, operator_pubkey,
         voting_address, addr_core_p2p, addr_platform_p2p, addr_platform_https, payout_address,
         operator_reward, validator_type, status, registered_height, last_updated_height
       ) VALUES (
         $1, $2, $3, $4, $5, $6,
         $7, $8, $9, $10, $11,
         $12, $13, $14, $15, $16
       )
       ON CONFLICT (pro_tx_hash) DO NOTHING",
    )
    .bind(&pro_tx_hash)
    .bind(super::synthetic_hash(b'C', i64::from(index)))
    .bind(0_i32)
    .bind(format!("Xcollateral{index:05}"))
    .bind(format!("Xowner{index:05}"))
    .bind(super::synthetic_hash(b'O', i64::from(index)))
    .bind(format!("Xvote{index:05}"))
    .bind(vec![format!("127.0.0.1:{}", 20_000 + index)])
    .bind(vec![format!("10.0.0.{}:{}", index % 255, 30_000 + index)])
    .bind(vec![format!("https://node{index:05}.dash.test")])
    .bind(format!("Xpayout{index:05}"))
    .bind(0_i32)
    .bind(if index % 4 == 0 { "Evo" } else { "Regular" })
    .bind(if index % 3 == 0 {
      "registered"
    } else {
      "confirmed"
    })
    .bind(index)
    .bind(index + 100)
    .execute(&mut *tx)
    .await
    .map_err(|error| pg_err(&format!("seed validator {index}"), error))?;
    for history_step in 0..VALIDATOR_HISTORY_ROWS_PER_VALIDATOR {
      let txid = super::synthetic_hash(b'P', i64::from(index * 10 + history_step));
      let block_height = block_rows(dataset)
        - (validator_rows(dataset) * VALIDATOR_HISTORY_ROWS_PER_VALIDATOR)
        + index * VALIDATOR_HISTORY_ROWS_PER_VALIDATOR
        + history_step;
      sqlx::query(
        "INSERT INTO transactions (
           txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
         ) VALUES ($1, $2, $3, $4, 'provider_register', $5, $6, $7, $8, $9, $10)
         ON CONFLICT (txid) DO NOTHING",
      )
      .bind(&txid)
      .bind(block_height)
      .bind(history_step)
      .bind(3_i32)
      .bind(0_i32)
      .bind(256_i32)
      .bind(1_000_i64)
      .bind(1_i32)
      .bind(2_i32)
      .bind(Some(10_i64))
      .execute(&mut *tx)
      .await
      .map_err(|error| pg_err(&format!("seed validator tx {index}:{history_step}"), error))?;
      sqlx::query(
        "INSERT INTO special_tx_payloads (txid, tx_type, pro_tx_hash, payload)
         VALUES ($1, 'provider_register', $2, $3)
         ON CONFLICT (txid) DO NOTHING",
      )
      .bind(&txid)
      .bind(&pro_tx_hash)
      .bind(serde_json::json!({
        "proTxHash": hex::encode(&pro_tx_hash),
        "step": history_step,
      }))
      .execute(&mut *tx)
      .await
      .map_err(|error| pg_err(&format!("seed validator payload {index}:{history_step}"), error))?;
    }
  }
  tx.commit()
    .await
    .map_err(|error| pg_err("commit validator seed tx", error))?;
  Ok(())
}
