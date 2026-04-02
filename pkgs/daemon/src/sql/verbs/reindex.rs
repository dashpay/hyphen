//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Force-reindex helpers.

use std::sync::Arc;

use crate::error::DaemonError;
use crate::index::Stage;
use crate::sql;

/// Force a full reindex of `stage` and all downstream dependents.
///
/// # Errors
///
/// Returns `DaemonError::Index` for unknown stage names or
/// `DaemonError::Pg` when the required schema reset fails.
pub async fn force_reindex_stage(
  pool: &sqlx::PgPool,
  _db: &rocksdb::DB,
  stage: &str,
) -> Result<String, DaemonError> {
  let Some(stage) = Stage::from_name(stage) else {
    return Err(DaemonError::Index(format!("unknown stage: {stage}")));
  };
  let stages = stage.reset_stages();

  if stages.contains(&Stage::Block) {
    sql::bootstrap::reset(pool).await?;
  } else {
    let tables: Vec<&str> = stages
      .iter()
      .flat_map(|stage| stage.tables().iter().copied())
      .collect();
    let stage_names: Vec<&str> = stages.iter().copied().map(Stage::name).collect();
    sql::bootstrap::targeted_reset(pool, &tables, &stage_names).await?;
  }

  let names: Vec<&str> = stages.iter().copied().map(Stage::name).collect();
  Ok(format!("reindex started for: {}", names.join(", ")))
}

/// Force a full reindex of `stage` using owned shared handles.
///
/// # Errors
///
/// Returns the same errors as [`force_reindex_stage`].
pub async fn force_reindex_stage_shared(
  pool: sqlx::PgPool,
  _db: Arc<rocksdb::DB>,
  stage: String,
) -> Result<String, DaemonError> {
  let Some(stage) = Stage::from_name(&stage) else {
    return Err(DaemonError::Index(format!("unknown stage: {stage}")));
  };
  let stages = stage.reset_stages();

  if stages.contains(&Stage::Block) {
    sql::bootstrap::reset(&pool).await?;
  } else {
    let tables: Vec<&str> = stages
      .iter()
      .flat_map(|stage| stage.tables().iter().copied())
      .collect();
    let stage_names: Vec<&str> = stages.iter().copied().map(Stage::name).collect();
    sql::bootstrap::targeted_reset(&pool, &tables, &stage_names).await?;
  }

  let names: Vec<&str> = stages.iter().copied().map(Stage::name).collect();
  Ok(format!("reindex started for: {}", names.join(", ")))
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;
  use std::sync::atomic::{AtomicU64, Ordering};
  use std::time::{SystemTime, UNIX_EPOCH};

  use crate::index::Stage;
  use crate::sql;

  use super::force_reindex_stage;

  /// Seed one block row so public reindex tests can observe block-owned resets.
  async fn seed_block(
    pool: &sqlx::PgPool,
    height: i32,
    hash: Vec<u8>,
    prev_hash: Vec<u8>,
    merkle_root: Vec<u8>,
  ) -> Result<(), sqlx::Error> {
    sqlx::query(
      "INSERT INTO blocks (
         height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count
       ) VALUES ($1, $2, $3, $4, $5, 0, 0, 1, 80, 1)",
    )
    .bind(height)
    .bind(hash)
    .bind(prev_hash)
    .bind(merkle_root)
    .bind(i64::from(height))
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one transaction row so reindex tests can verify stage-owned resets.
  async fn seed_transaction(
    pool: &sqlx::PgPool,
    txid: Vec<u8>,
    tx_type: &str,
    value_out: i64,
  ) -> Result<(), sqlx::Error> {
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count
       ) VALUES ($1, 0, 0, 3, $2, 0, 100, $3, 1, 1)",
    )
    .bind(txid)
    .bind(tx_type)
    .bind(value_out)
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one sync-state row so reset tests can observe public stage rewrites.
  async fn seed_sync_height(
    pool: &sqlx::PgPool,
    stage: &str,
    height: i32,
  ) -> Result<(), sqlx::Error> {
    sqlx::query("UPDATE sync_state SET indexed_height = $1, updated_at = 0 WHERE stage = $2")
      .bind(height)
      .bind(stage)
      .execute(pool)
      .await?;
    Ok(())
  }

  /// Seed one validator payload row because validator resets depend on transaction-owned payload state.
  async fn seed_validator_payload(
    pool: &sqlx::PgPool,
    txid: Vec<u8>,
    pro_tx_hash: Vec<u8>,
  ) -> Result<(), sqlx::Error> {
    sqlx::query(
      "INSERT INTO special_tx_payloads (txid, tx_type, pro_tx_hash, payload)
       VALUES ($1, 'provider_register', $2, '{}'::jsonb)",
    )
    .bind(txid)
    .bind(pro_tx_hash)
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one validator row because validator resets must clear validator-owned state only.
  async fn seed_validator_row(
    pool: &sqlx::PgPool,
    pro_tx_hash: Vec<u8>,
    collateral_hash: Vec<u8>,
    operator_pubkey: Vec<u8>,
  ) -> Result<(), sqlx::Error> {
    sqlx::query(
      "INSERT INTO validators (
         pro_tx_hash, collateral_hash, collateral_index, owner_address, operator_pubkey,
         voting_address, operator_reward, validator_type, status, registered_height, last_updated_height
       ) VALUES ($1, $2, 0, 'Xowner', $3, 'Xvote', 0, 'regular', 'registered', 0, 0)",
    )
    .bind(pro_tx_hash)
    .bind(collateral_hash)
    .bind(operator_pubkey)
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one script history row because script resets should clear script-owned history.
  async fn seed_script_history(pool: &sqlx::PgPool, txid: Vec<u8>) -> Result<(), sqlx::Error> {
    sqlx::query(
      "INSERT INTO script_output_history (height, txid, vout_index, prev_address, prev_address_type)
       VALUES (0, $1, 0, 'Xscript', 'p2pkh')",
    )
    .bind(txid)
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one address balance row because script resets should clear spent-derived balance state.
  async fn seed_address_balance(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query("INSERT INTO address_balances (address, balance) VALUES ('Xscript', 10)")
      .execute(pool)
      .await?;
    Ok(())
  }

  /// Seed one spent history row because spent resets should clear spent-owned history.
  async fn seed_spent_history(
    pool: &sqlx::PgPool,
    spending_txid: Vec<u8>,
    prev_txid: Vec<u8>,
  ) -> Result<(), sqlx::Error> {
    sqlx::query(
      "INSERT INTO spent_output_history (height, spending_txid, spending_vin, prev_txid, prev_vout)
       VALUES (0, $1, 0, $2, 0)",
    )
    .bind(spending_txid)
    .bind(prev_txid)
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one output row because block resets should clear transaction-owned output state.
  async fn seed_output(
    pool: &sqlx::PgPool,
    txid: Vec<u8>,
    script_pubkey: Vec<u8>,
  ) -> Result<(), sqlx::Error> {
    sqlx::query(
      "INSERT INTO tx_outputs (
         txid, vout_index, block_height, value, script_pubkey, address, address_type
       ) VALUES ($1, 0, 0, 50, $2, 'Xblock', 'p2pkh')",
    )
    .bind(txid)
    .bind(script_pubkey)
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one metrics row because block resets should clear metrics-owned state.
  async fn seed_metrics_row(pool: &sqlx::PgPool) -> Result<(), sqlx::Error> {
    sqlx::query(
      "INSERT INTO block_metrics (
         height, tx_count, utxos_spendable, utxos_unspendable, blockchain_size, circulating_supply, difficulty, hashrate
       ) VALUES (0, 1, 1, 0, 80, 50, 1, 1)",
    )
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Count rows in one application table because reset tests assert owned-table clearing.
  async fn count_rows(pool: &sqlx::PgPool, table_name: &'static str) -> Result<i64, sqlx::Error> {
    sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table_name}"))
      .fetch_one(pool)
      .await
  }

  #[tokio::test]
  async fn force_reindex_rejects_unknown_stage() -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    sql::bootstrap::reset(&pool).await?;
    let db = open_test_db()?;

    let err = force_reindex_stage(&pool, &db, "unknown_stage")
      .await
      .expect_err("unknown stage should fail");
    assert!(
      matches!(err, crate::error::DaemonError::Index(message) if message.contains("unknown stage"))
    );

    Ok(())
  }

  #[tokio::test]
  async fn force_reindex_validator_preserves_transaction_owned_tables(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    sql::bootstrap::reset(&pool).await?;

    seed_block(&pool, 0, vec![1_u8; 32], vec![2_u8; 32], vec![3_u8; 32]).await?;
    seed_transaction(&pool, vec![4_u8; 32], "provider_register", 50).await?;
    seed_validator_payload(&pool, vec![4_u8; 32], vec![5_u8; 32]).await?;
    seed_validator_row(&pool, vec![5_u8; 32], vec![6_u8; 32], vec![7_u8; 48]).await?;
    seed_sync_height(&pool, "transaction", 0).await?;
    seed_sync_height(&pool, "validator", 0).await?;

    let db = open_test_db()?;
    let message = force_reindex_stage(&pool, &db, "validator").await?;

    assert_eq!(message, "reindex started for: validator");
    assert_eq!(count_rows(&pool, "transactions").await?, 1);
    assert_eq!(count_rows(&pool, "special_tx_payloads").await?, 1);
    assert_eq!(count_rows(&pool, "validators").await?, 0);
    assert_eq!(
      sql::query::indexed_stage_height(&pool, Stage::Transaction).await?,
      0
    );
    assert_eq!(
      sql::query::indexed_stage_height(&pool, Stage::Validator).await?,
      -1
    );

    Ok(())
  }

  #[tokio::test]
  async fn force_reindex_script_resets_spent_stage_from_public_entrypoint(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    sql::bootstrap::reset(&pool).await?;

    seed_address_balance(&pool).await?;
    seed_script_history(&pool, vec![9_u8; 32]).await?;
    seed_spent_history(&pool, vec![10_u8; 32], vec![11_u8; 32]).await?;
    seed_sync_height(&pool, "script", 0).await?;
    seed_sync_height(&pool, "spent", 0).await?;

    let db = open_test_db()?;
    let message = force_reindex_stage(&pool, &db, "script").await?;

    assert_eq!(message, "reindex started for: spent, script");
    assert_eq!(count_rows(&pool, "script_output_history").await?, 0);
    assert_eq!(count_rows(&pool, "address_balances").await?, 0);
    assert_eq!(count_rows(&pool, "spent_output_history").await?, 0);
    assert_eq!(
      sql::query::indexed_stage_height(&pool, Stage::Script).await?,
      -1
    );
    assert_eq!(
      sql::query::indexed_stage_height(&pool, Stage::Spent).await?,
      -1
    );

    Ok(())
  }

  #[tokio::test]
  async fn force_reindex_block_resets_all_public_state() -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    sql::bootstrap::reset(&pool).await?;

    seed_block(&pool, 0, vec![13_u8; 32], vec![14_u8; 32], vec![15_u8; 32]).await?;
    seed_transaction(&pool, vec![16_u8; 32], "normal", 50).await?;
    seed_output(&pool, vec![16_u8; 32], vec![0x51_u8]).await?;
    seed_metrics_row(&pool).await?;
    for stage in [
      Stage::Block,
      Stage::Transaction,
      Stage::Script,
      Stage::Spent,
      Stage::Metrics,
      Stage::Validator,
    ] {
      seed_sync_height(&pool, stage.name(), 0).await?;
    }

    let db = open_test_db()?;
    let message = force_reindex_stage(&pool, &db, "block").await?;

    assert_eq!(
      message,
      "reindex started for: spent, script, validator, metrics, transaction, block"
    );
    assert_eq!(count_rows(&pool, "blocks").await?, 0);
    assert_eq!(count_rows(&pool, "transactions").await?, 0);
    assert_eq!(count_rows(&pool, "tx_outputs").await?, 0);
    assert_eq!(count_rows(&pool, "block_metrics").await?, 0);
    assert_eq!(
      sql::query::indexed_stage_height(&pool, Stage::Block).await?,
      -1
    );
    assert_eq!(
      sql::query::indexed_stage_height(&pool, Stage::Transaction).await?,
      -1
    );
    assert_eq!(
      sql::query::indexed_stage_height(&pool, Stage::Metrics).await?,
      -1
    );

    Ok(())
  }

  /// Open one isolated RocksDB instance because public reindex entrypoints require a real handle.
  fn open_test_db() -> Result<rocksdb::DB, crate::error::DaemonError> {
    let mut path = std::env::temp_dir();
    path.push(unique_db_name());
    crate::db::open(&path, None)
  }

  /// Build one unique RocksDB path so reindex tests do not share on-disk state.
  fn unique_db_name() -> PathBuf {
    static NEXT_DB_ID: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|duration| duration.as_nanos())
      .unwrap_or(0);
    let unique_id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!("hyphend-rewind-test-{nanos}-{unique_id}"))
  }
}
