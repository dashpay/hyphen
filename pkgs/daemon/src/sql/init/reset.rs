//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Schema reset helpers.

use crate::error::DaemonError;
use crate::sql::pg_err;

const LOCK_TIMEOUT: &str = "SET lock_timeout = '5s'";
const SCHEMA_RESET_RETRIES: u32 = 3;

const NUCLEAR_DROP_SQL: &str =
  "DROP TABLE IF EXISTS address_txs CASCADE;
   DROP TABLE IF EXISTS address_stats CASCADE;
   DROP TABLE IF EXISTS address_balances CASCADE;
   DROP TABLE IF EXISTS address_balances_summary CASCADE;
   DROP TABLE IF EXISTS block_undo_address_balances CASCADE;
   DROP TABLE IF EXISTS script_output_history CASCADE;
   DROP TABLE IF EXISTS special_tx_payloads CASCADE;
   DROP TABLE IF EXISTS validator_history CASCADE;
   DROP TABLE IF EXISTS validator_counts CASCADE;
   DROP TABLE IF EXISTS spent_output_history CASCADE;
   DROP TABLE IF EXISTS block_metrics CASCADE;
   DROP TABLE IF EXISTS validators CASCADE;
   DROP TABLE IF EXISTS governance_summary CASCADE;
   DROP TABLE IF EXISTS governance_votes CASCADE;
   DROP TABLE IF EXISTS governance_objects CASCADE;
   DROP TABLE IF EXISTS search_sync_events CASCADE;
   DROP TABLE IF EXISTS network_snapshot CASCADE;
   DROP TABLE IF EXISTS validator_protx_snapshot CASCADE;
   DROP TABLE IF EXISTS chainlock_snapshot CASCADE;
   DROP TABLE IF EXISTS islock_current CASCADE;
   DROP TABLE IF EXISTS mempool_address_txs CASCADE;
   DROP TABLE IF EXISTS mempool_tx_outputs CASCADE;
   DROP TABLE IF EXISTS mempool_tx_inputs CASCADE;
   DROP TABLE IF EXISTS mempool_tx_details CASCADE;
   DROP TABLE IF EXISTS mempool_recent_txs CASCADE;
   DROP TABLE IF EXISTS mempool_summary CASCADE;
   DROP TABLE IF EXISTS tx_inputs CASCADE;
   DROP TABLE IF EXISTS tx_outputs CASCADE;
   DROP TABLE IF EXISTS transactions_summary CASCADE;
   DROP TABLE IF EXISTS transactions CASCADE;
   DROP TABLE IF EXISTS blocks CASCADE;
   DROP TABLE IF EXISTS orphan_blocks CASCADE;
   DROP TABLE IF EXISTS sync_state CASCADE;
   DROP TABLE IF EXISTS chain_progress CASCADE;
   DROP TYPE IF EXISTS tx_type CASCADE;
   DROP TYPE IF EXISTS address_type CASCADE;";

/// Nuclear reset: drop all application tables and re-apply schema.
///
/// The entire operation runs inside a single transaction with retries and
/// exponential backoff, preventing partial schema states on crash.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the lock timeout,
/// drop statements, schema re-application, or final commit.
pub async fn reset(pool: &sqlx::PgPool) -> Result<(), DaemonError> {
  tracing::warn!("nuclear schema reset: dropping all tables");

  let mut last_err: Option<DaemonError> = None;
  for attempt in 1..=SCHEMA_RESET_RETRIES {
    if let Some(ref prev) = last_err {
      tracing::warn!(attempt, "nuclear DROP failed, retrying: {prev}");
      tokio::time::sleep(std::time::Duration::from_secs(attempt as u64 * 2)).await;
    }

    let mut tx = pool
      .begin()
      .await
      .map_err(|e| pg_err("nuclear reset begin", e))?;

    if let Err(e) = sqlx::raw_sql(LOCK_TIMEOUT).execute(&mut *tx).await {
      return Err(pg_err("set lock_timeout", e));
    }

    let result = async {
      sqlx::raw_sql(NUCLEAR_DROP_SQL)
        .execute(&mut *tx)
        .await
        .map_err(|e| pg_err("nuclear drop", e))?;

      tracing::info!("re-applying schema after nuclear reset");
      sqlx::raw_sql(super::SCHEMA)
        .execute(&mut *tx)
        .await
        .map_err(|e| pg_err("re-apply schema", e))?;
      for sql in crate::sql::partition::history_partition_sqls(0) {
        sqlx::query(&sql)
          .execute(&mut *tx)
          .await
          .map_err(|e| pg_err("ensure core history partitions", e))?;
      }
      for sql in crate::sql::partition::canonical_partition_sqls(0) {
        sqlx::query(&sql)
          .execute(&mut *tx)
          .await
          .map_err(|e| pg_err("ensure core canonical partitions", e))?;
      }

      tx.commit()
        .await
        .map_err(|e| pg_err("nuclear reset commit", e))
    }
    .await;

    match result {
      Ok(_) => {
        last_err = None;
        break;
      }
      Err(e) => {
        last_err = Some(e);
      }
    }
  }
  if let Some(e) = last_err {
    return Err(e);
  }

  tracing::info!("nuclear reset complete");
  Ok(())
}

/// Targeted per-stage reset: drop specified tables, reset sync_state,
/// re-apply schema (IF NOT EXISTS guards recreate only dropped ones).
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the lock timeout,
/// stage reset, schema re-application, or final commit.
pub async fn targeted_reset(
  pool: &sqlx::PgPool,
  tables: &[&str],
  stages: &[&str],
) -> Result<(), DaemonError> {
  tracing::warn!(?tables, ?stages, "targeted reset: dropping tables");

  let drops: Vec<String> = tables
    .iter()
    .map(|t| {
      assert!(
        t.chars().all(|c| c.is_ascii_lowercase() || c == '_'),
        "table name must be [a-z_]+: {t}"
      );
      format!("DROP TABLE IF EXISTS {t} CASCADE")
    })
    .collect();
  let drop_sql = drops.join("; ");

  let mut last_err: Option<DaemonError> = None;
  for attempt in 1..=SCHEMA_RESET_RETRIES {
    if let Some(ref prev) = last_err {
      tracing::warn!(attempt, "targeted DROP failed, retrying: {prev}");
      tokio::time::sleep(std::time::Duration::from_secs(attempt as u64 * 2)).await;
    }

    let mut tx = pool
      .begin()
      .await
      .map_err(|e| pg_err("targeted reset begin", e))?;

    if let Err(e) = sqlx::raw_sql(LOCK_TIMEOUT).execute(&mut *tx).await {
      return Err(pg_err("set lock_timeout", e));
    }

    let result = async {
      sqlx::raw_sql(&drop_sql)
        .execute(&mut *tx)
        .await
        .map_err(|e| pg_err("targeted drop", e))?;

      for stage in stages {
        sqlx::query("UPDATE sync_state SET indexed_height = -1, updated_at = 0 WHERE stage = $1")
          .bind(stage)
          .execute(&mut *tx)
          .await
          .map_err(|e| pg_err(&format!("reset sync_state for {stage}"), e))?;
      }

      tracing::info!("re-applying schema after targeted reset");
      sqlx::raw_sql(super::SCHEMA)
        .execute(&mut *tx)
        .await
        .map_err(|e| pg_err("re-apply schema", e))?;
      for sql in crate::sql::partition::history_partition_sqls(0) {
        sqlx::query(&sql)
          .execute(&mut *tx)
          .await
          .map_err(|e| pg_err("ensure core history partitions", e))?;
      }
      for sql in crate::sql::partition::canonical_partition_sqls(0) {
        sqlx::query(&sql)
          .execute(&mut *tx)
          .await
          .map_err(|e| pg_err("ensure core canonical partitions", e))?;
      }

      tx.commit()
        .await
        .map_err(|e| pg_err("targeted reset commit", e))
    }
    .await;

    match result {
      Ok(_) => {
        last_err = None;
        break;
      }
      Err(e) => {
        last_err = Some(e);
      }
    }
  }
  if let Some(e) = last_err {
    return Err(e);
  }

  tracing::info!("targeted reset complete");
  Ok(())
}

#[cfg(test)]
mod tests {
  use sqlx::postgres::PgPoolOptions;

  use super::{reset, targeted_reset};

  #[tokio::test]
  async fn targeted_reset_preserves_transaction_tables_for_validator_stage(
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
    reset(&pool).await?;

    seed_sync_state(&pool).await?;
    sqlx::query(
      "INSERT INTO blocks (
         height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count
       ) VALUES (0, $1, $2, $3, 0, 0, 0, 1, 80, 1)",
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
       ) VALUES ($1, 0, 0, 1, 'normal', 0, 100, 0, 1, 1)",
    )
    .bind(vec![1_u8; 32])
    .execute(&pool)
    .await?;
    sqlx::query(
      "INSERT INTO special_tx_payloads (txid, tx_type, payload)
       VALUES ($1, 'provider_register', '{}'::jsonb)",
    )
    .bind(vec![2_u8; 32])
    .execute(&pool)
    .await?;
    sqlx::query(
      "INSERT INTO validators (
         pro_tx_hash, collateral_hash, collateral_index, operator_pubkey,
         operator_reward, owner_address, voting_address, validator_type,
         status, registered_height, last_updated_height
       ) VALUES ($1, $2, 0, $3, 0, 'owner', 'voter', 'Regular', 'registered', 0, 0)",
    )
    .bind(vec![3_u8; 32])
    .bind(vec![4_u8; 32])
    .bind(vec![5_u8; 48])
    .execute(&pool)
    .await?;

    let stages = crate::index::Stage::Validator.reset_stages();
    let tables: Vec<&str> = stages
      .iter()
      .flat_map(|stage| stage.tables().iter().copied())
      .collect();
    let stage_names: Vec<&str> = stages
      .iter()
      .copied()
      .map(crate::index::Stage::name)
      .collect();
    targeted_reset(&pool, &tables, &stage_names).await?;

    let validator_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM validators")
      .fetch_one(&pool)
      .await?;
    let special_payload_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM special_tx_payloads")
      .fetch_one(&pool)
      .await?;
    let transaction_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM transactions")
      .fetch_one(&pool)
      .await?;

    assert_eq!(validator_count, 0);
    assert_eq!(special_payload_count, 1);
    assert_eq!(transaction_count, 1);
    assert_eq!(
      indexed_height(&pool, crate::index::Stage::Validator.name()).await?,
      -1
    );
    assert_eq!(
      indexed_height(&pool, crate::index::Stage::Transaction.name()).await?,
      12
    );

    Ok(())
  }

  #[tokio::test]
  async fn targeted_reset_resets_script_and_spent_together(
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
    reset(&pool).await?;

    seed_sync_state(&pool).await?;
    sqlx::query(
      "INSERT INTO address_stats (
         address, balance, total_received, tx_count, first_seen_height, last_seen_height
       ) VALUES ('Xtest', 1, 1, 1, 0, 0)",
    )
    .execute(&pool)
    .await?;
    sqlx::query(
      "INSERT INTO address_txs (address, txid, block_height, block_pos)
       VALUES ('Xtest', $1, 0, 0)",
    )
    .bind(vec![7_u8; 32])
    .execute(&pool)
    .await?;
    sqlx::query(
      "INSERT INTO script_output_history (
         height, txid, vout_index, prev_address, prev_address_type
       ) VALUES (0, $1, 0, NULL, NULL)",
    )
    .bind(vec![8_u8; 32])
    .execute(&pool)
    .await?;

    let stages = crate::index::Stage::Script.reset_stages();
    let tables: Vec<&str> = stages
      .iter()
      .flat_map(|stage| stage.tables().iter().copied())
      .collect();
    let stage_names: Vec<&str> = stages
      .iter()
      .copied()
      .map(crate::index::Stage::name)
      .collect();
    targeted_reset(&pool, &tables, &stage_names).await?;

    let address_stats_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM address_stats")
      .fetch_one(&pool)
      .await?;
    let script_history_count: i64 =
      sqlx::query_scalar("SELECT COUNT(*) FROM script_output_history")
        .fetch_one(&pool)
        .await?;

    assert_eq!(address_stats_count, 0);
    assert_eq!(script_history_count, 0);
    assert_eq!(
      indexed_height(&pool, crate::index::Stage::Script.name()).await?,
      -1
    );
    assert_eq!(
      indexed_height(&pool, crate::index::Stage::Spent.name()).await?,
      -1
    );

    Ok(())
  }

  #[tokio::test]
  async fn nuclear_reset_drops_address_balances_undo_table(
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
    reset(&pool).await?;

    sqlx::query(
      "INSERT INTO block_undo_address_balances (
         height, address, existed_before, prev_balance
       ) VALUES (11, 'Xundo', TRUE, 42)",
    )
    .execute(&pool)
    .await?;

    reset(&pool).await?;

    let row_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM block_undo_address_balances")
      .fetch_one(&pool)
      .await?;
    assert_eq!(row_count, 0);
    Ok(())
  }

  async fn seed_sync_state(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    for stage in [
      "block",
      "transaction",
      "script",
      "spent",
      "metrics",
      "validator",
    ] {
      sqlx::query("UPDATE sync_state SET indexed_height = 12, updated_at = 34 WHERE stage = $1")
        .bind(stage)
        .execute(pool)
        .await?;
    }
    Ok(())
  }

  async fn indexed_height(
    pool: &sqlx::PgPool,
    stage: &str,
  ) -> Result<i32, Box<dyn std::error::Error>> {
    Ok(
      sqlx::query_scalar("SELECT indexed_height FROM sync_state WHERE stage = $1")
        .bind(stage)
        .fetch_one(pool)
        .await?,
    )
  }
}
