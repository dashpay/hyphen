//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Partition helpers for large height-ranged tables.

use sqlx::PgPool;

use crate::error::DaemonError;
use super::pg_err;

pub const PARTITION_BLOCK_SPAN: i32 = 100_000;
pub const HISTORY_PARTITION_BLOCK_SPAN: i32 = PARTITION_BLOCK_SPAN;

#[derive(Clone, Copy)]
struct HeightPartition {
  parent_table: &'static str,
  partition_prefix: &'static str,
}

const SCRIPT_OUTPUT_HISTORY: HeightPartition = HeightPartition {
  parent_table: "script_output_history",
  partition_prefix: "script_output_history_p",
};

const SPENT_OUTPUT_HISTORY: HeightPartition = HeightPartition {
  parent_table: "spent_output_history",
  partition_prefix: "spent_output_history_p",
};

const TRANSACTIONS: HeightPartition = HeightPartition {
  parent_table: "transactions",
  partition_prefix: "transactions_p",
};

const TX_OUTPUTS: HeightPartition = HeightPartition {
  parent_table: "tx_outputs",
  partition_prefix: "tx_outputs_p",
};

fn partition_bounds(height: i32) -> (i32, i32) {
  let start = (height / PARTITION_BLOCK_SPAN) * PARTITION_BLOCK_SPAN;
  (start, start + PARTITION_BLOCK_SPAN)
}

fn partition_name(partition: HeightPartition, height: i32) -> String {
  let (start, _) = partition_bounds(height);
  format!("{}{:07}", partition.partition_prefix, start)
}

fn ensure_partition_sql(partition: HeightPartition, height: i32) -> String {
  let (start, end) = partition_bounds(height);
  let child = partition_name(partition, height);
  format!(
    "CREATE TABLE IF NOT EXISTS {child}
       PARTITION OF {parent}
       FOR VALUES FROM ({start}) TO ({end})",
    child = child,
    parent = partition.parent_table,
    start = start,
    end = end,
  )
}

async fn execute_partition_sqls(
  pool: &PgPool,
  sqls: &[String],
  context: &str,
) -> Result<(), DaemonError> {
  for sql in sqls {
    sqlx::query(sql)
      .execute(pool)
      .await
      .map_err(|e| pg_err(&format!("{context}"), e))?;
  }
  Ok(())
}

/// Ensure the base height-partitioned tables exist for the current deployment.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects any partition DDL.
pub async fn ensure_core_history_partitions(pool: &PgPool) -> Result<(), DaemonError> {
  execute_partition_sqls(
    pool,
    &history_partition_sqls(0),
    "ensure core history partitions",
  )
  .await
}

/// Ensure the base canonical partitions exist for the current deployment.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects any partition DDL.
pub async fn ensure_core_canonical_partitions(pool: &PgPool) -> Result<(), DaemonError> {
  execute_partition_sqls(
    pool,
    &canonical_partition_sqls(0),
    "ensure core canonical partitions",
  )
  .await
}

/// Build the history partition DDL statements needed for the block window containing `height`.
pub fn history_partition_sqls(height: i32) -> [String; 2] {
  [
    ensure_partition_sql(SCRIPT_OUTPUT_HISTORY, height),
    ensure_partition_sql(SPENT_OUTPUT_HISTORY, height),
  ]
}

/// Build the canonical partition DDL statements needed for the block window containing `height`.
pub fn canonical_partition_sqls(height: i32) -> [String; 2] {
  [
    ensure_partition_sql(TRANSACTIONS, height),
    ensure_partition_sql(TX_OUTPUTS, height),
  ]
}

/// Return the inclusive start height for the partition window containing `height`.
pub fn history_partition_start(height: i32) -> i32 {
  partition_bounds(height).0
}

/// Return the inclusive start height for the partition window containing `height`.
pub fn canonical_partition_start(height: i32) -> i32 {
  partition_bounds(height).0
}

/// Build the `script_output_history` partition DDL for the block window containing `height`.
pub fn ensure_script_output_history_partition_sql(height: i32) -> String {
  ensure_partition_sql(SCRIPT_OUTPUT_HISTORY, height)
}

/// Build the `transactions` partition DDL for the block window containing `height`.
pub fn ensure_transactions_partition_sql(height: i32) -> String {
  ensure_partition_sql(TRANSACTIONS, height)
}

/// Build the `tx_outputs` partition DDL for the block window containing `height`.
pub fn ensure_tx_outputs_partition_sql(height: i32) -> String {
  ensure_partition_sql(TX_OUTPUTS, height)
}

#[cfg(test)]
/// Build the `spent_output_history` partition DDL for the block window containing `height`.
pub fn ensure_spent_output_history_partition_sql(height: i32) -> String {
  ensure_partition_sql(SPENT_OUTPUT_HISTORY, height)
}

#[cfg(test)]
mod tests {
  use sqlx::postgres::PgPoolOptions;

  use super::{
    canonical_partition_sqls, ensure_core_canonical_partitions, ensure_core_history_partitions,
    ensure_script_output_history_partition_sql, ensure_spent_output_history_partition_sql,
    ensure_transactions_partition_sql, ensure_tx_outputs_partition_sql, history_partition_sqls,
    partition_bounds, partition_name, PARTITION_BLOCK_SPAN, SCRIPT_OUTPUT_HISTORY,
    SPENT_OUTPUT_HISTORY, TRANSACTIONS, TX_OUTPUTS,
  };

  #[test]
  fn partition_bounds_follow_fixed_block_windows() {
    assert_eq!(partition_bounds(0), (0, PARTITION_BLOCK_SPAN));
    assert_eq!(partition_bounds(99_999), (0, PARTITION_BLOCK_SPAN));
    assert_eq!(
      partition_bounds(100_000),
      (100_000, 100_000 + PARTITION_BLOCK_SPAN)
    );
    assert_eq!(
      partition_bounds(100_001),
      (100_000, 100_000 + PARTITION_BLOCK_SPAN)
    );
  }

  #[test]
  fn partition_names_are_deterministic() {
    assert_eq!(
      partition_name(SCRIPT_OUTPUT_HISTORY, 0),
      "script_output_history_p0000000"
    );
    assert_eq!(
      partition_name(SPENT_OUTPUT_HISTORY, 100_001),
      "spent_output_history_p0100000"
    );
    assert_eq!(partition_name(TRANSACTIONS, 0), "transactions_p0000000");
    assert_eq!(partition_name(TX_OUTPUTS, 100_001), "tx_outputs_p0100000");
  }

  #[tokio::test]
  async fn ensures_history_partitions_idempotently_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };
    let pool = PgPoolOptions::new()
      .max_connections(1)
      .connect(
        &database_url
          .into_string()
          .map_err(|_| "invalid database url")?,
      )
      .await?;
    crate::sql::bootstrap::reset(&pool).await?;
    ensure_core_history_partitions(&pool).await?;

    for sql in history_partition_sqls(0) {
      sqlx::query(&sql).execute(&pool).await?;
    }

    sqlx::query(&ensure_script_output_history_partition_sql(100_000))
      .execute(&pool)
      .await?;
    sqlx::query(&ensure_script_output_history_partition_sql(100_000))
      .execute(&pool)
      .await?;
    sqlx::query(&ensure_spent_output_history_partition_sql(100_001))
      .execute(&pool)
      .await?;
    let count: i64 = sqlx::query_scalar(
      "SELECT COUNT(*)
         FROM pg_class
        WHERE relname IN (
          'script_output_history_p0000000',
          'script_output_history_p0100000',
          'spent_output_history_p0100000'
        )",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 3);

    Ok(())
  }

  #[tokio::test]
  async fn ensures_canonical_partitions_idempotently_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };
    let pool = PgPoolOptions::new()
      .max_connections(1)
      .connect(
        &database_url
          .into_string()
          .map_err(|_| "invalid database url")?,
      )
      .await?;
    crate::sql::bootstrap::reset(&pool).await?;
    ensure_core_canonical_partitions(&pool).await?;

    for sql in canonical_partition_sqls(0) {
      sqlx::query(&sql).execute(&pool).await?;
    }

    sqlx::query(&ensure_transactions_partition_sql(100_000))
      .execute(&pool)
      .await?;
    sqlx::query(&ensure_transactions_partition_sql(100_000))
      .execute(&pool)
      .await?;
    sqlx::query(&ensure_tx_outputs_partition_sql(100_001))
      .execute(&pool)
      .await?;
    let count: i64 = sqlx::query_scalar(
      "SELECT COUNT(*)
         FROM pg_class
        WHERE relname IN (
          'transactions_p0000000',
          'transactions_p0100000',
          'tx_outputs_p0100000'
        )",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(count, 3);

    Ok(())
  }

  #[tokio::test]
  async fn reset_recreates_partitioned_history_tables_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };
    let pool = PgPoolOptions::new()
      .max_connections(1)
      .connect(
        &database_url
          .into_string()
          .map_err(|_| "invalid database url")?,
      )
      .await?;
    crate::sql::bootstrap::reset(&pool).await?;
    ensure_core_history_partitions(&pool).await?;

    sqlx::query(&ensure_script_output_history_partition_sql(100_000))
      .execute(&pool)
      .await?;
    sqlx::query(&ensure_spent_output_history_partition_sql(100_000))
      .execute(&pool)
      .await?;
    sqlx::query(
      "INSERT INTO script_output_history (
         height, txid, vout_index, prev_address, prev_address_type
       ) VALUES (100000, $1, 0, NULL, NULL)",
    )
    .bind(vec![1_u8; 32])
    .execute(&pool)
    .await?;
    sqlx::query(
      "INSERT INTO spent_output_history (
         height, spending_txid, spending_vin, prev_txid, prev_vout,
         prev_spent_txid, prev_spent_vin, prev_spent_height
       ) VALUES (100000, $1, 0, $2, 1, NULL, NULL, NULL)",
    )
    .bind(vec![2_u8; 32])
    .bind(vec![3_u8; 32])
    .execute(&pool)
    .await?;
    let script_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM script_output_history")
      .fetch_one(&pool)
      .await?;
    let spent_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM spent_output_history")
      .fetch_one(&pool)
      .await?;
    assert_eq!(script_count, 1);
    assert_eq!(spent_count, 1);

    crate::sql::bootstrap::reset(&pool).await?;

    let public_tables: i64 = sqlx::query_scalar(
      "SELECT COUNT(*)
         FROM information_schema.tables
        WHERE table_schema = 'public'
          AND table_name IN (
            'script_output_history',
            'spent_output_history'
          )",
    )
    .fetch_one(&pool)
    .await?;
    let child_tables: i64 = sqlx::query_scalar(
      "SELECT COUNT(*)
         FROM pg_class
        WHERE relname IN (
          'script_output_history_p0100000',
          'spent_output_history_p0100000'
        )",
    )
    .fetch_one(&pool)
    .await?;

    assert_eq!(public_tables, 2);
    assert_eq!(child_tables, 0);

    Ok(())
  }

  #[tokio::test]
  async fn reset_recreates_partitioned_canonical_tables_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };
    let pool = PgPoolOptions::new()
      .max_connections(1)
      .connect(
        &database_url
          .into_string()
          .map_err(|_| "invalid database url")?,
      )
      .await?;
    crate::sql::bootstrap::reset(&pool).await?;
    ensure_core_canonical_partitions(&pool).await?;

    sqlx::query(&ensure_transactions_partition_sql(100_000))
      .execute(&pool)
      .await?;
    sqlx::query(&ensure_tx_outputs_partition_sql(100_000))
      .execute(&pool)
      .await?;
    sqlx::query(
      "INSERT INTO blocks (
         height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count
       ) VALUES (100000, $1, $2, $3, 0, 0, 0, 1, 80, 1)",
    )
    .bind(vec![10_u8; 32])
    .bind(vec![11_u8; 32])
    .bind(vec![12_u8; 32])
    .execute(&pool)
    .await?;
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size,
         value_out, input_count, output_count
       ) VALUES ($1, 100000, 0, 1, 'normal', 0, 100, 0, 1, 1)",
    )
    .bind(vec![13_u8; 32])
    .execute(&pool)
    .await?;
    sqlx::query(
      "INSERT INTO tx_outputs (
         txid, vout_index, block_height, value, script_pubkey
       ) VALUES ($1, 0, 100000, 1, $2)",
    )
    .bind(vec![13_u8; 32])
    .bind(vec![0x51_u8])
    .execute(&pool)
    .await?;

    crate::sql::bootstrap::reset(&pool).await?;

    let child_tables: i64 = sqlx::query_scalar(
      "SELECT COUNT(*)
         FROM pg_class
        WHERE relname IN (
          'transactions_p0100000',
          'tx_outputs_p0100000'
        )",
    )
    .fetch_one(&pool)
    .await?;
    assert_eq!(child_tables, 0);

    Ok(())
  }
}
