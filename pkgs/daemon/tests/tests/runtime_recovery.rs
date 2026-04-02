//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use std::sync::Arc;

use async_trait::async_trait;

use hyphend::error::DaemonError;
use hyphend::index::Stage;
use hyphend::live::protx::rpc::refresh_once;
use hyphend::sources::rpc::{
  BestChainLock, MempoolInfo, MempoolTxMap, NetworkInfo, PeerInfo, ProTxListEntry, ProTxState,
};
use hyphend::sources::RpcSource;

#[derive(Debug)]
struct FakeRpc {
  registered: Vec<ProTxListEntry>,
  valid: Vec<ProTxListEntry>,
}

#[async_trait]
impl RpcSource for FakeRpc {
  async fn get_mempool_info(&self) -> Result<MempoolInfo, DaemonError> {
    unreachable!("unused in validator protx tests")
  }

  async fn get_raw_mempool(&self) -> Result<MempoolTxMap, DaemonError> {
    unreachable!("unused in validator protx tests")
  }

  async fn get_raw_transaction(&self, _txid: &str) -> Result<Vec<u8>, DaemonError> {
    unreachable!("unused in validator protx tests")
  }

  async fn get_network_info(&self) -> Result<NetworkInfo, DaemonError> {
    unreachable!("unused in validator protx tests")
  }

  async fn get_peer_info(&self) -> Result<PeerInfo, DaemonError> {
    unreachable!("unused in validator protx tests")
  }

  async fn get_best_chainlock(&self) -> Result<BestChainLock, DaemonError> {
    unreachable!("unused in validator protx tests")
  }

  async fn get_protx_list_registered(&self) -> Result<Vec<ProTxListEntry>, DaemonError> {
    Ok(self.registered.clone())
  }

  async fn get_protx_list_valid(&self) -> Result<Vec<ProTxListEntry>, DaemonError> {
    Ok(self.valid.clone())
  }

  fn diag(&self) -> hyphend::wire::msg::RpcDiag {
    hyphend::wire::msg::RpcDiag::default()
  }
}

async fn seed_block(
  pool: &hyphend::sql::SqlPool,
  height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
  let hash = vec![height as u8; 32];
  let prev_hash = if height == 0 {
    vec![0_u8; 32]
  } else {
    vec![(height - 1) as u8; 32]
  };
  let merkle_root = vec![height.wrapping_add(1) as u8; 32];
  sqlx::query(
    "INSERT INTO blocks (
       height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count, chainwork
     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
  )
  .bind(height as i32)
  .bind(hash)
  .bind(prev_hash)
  .bind(merkle_root)
  .bind(height as i32)
  .bind(486_604_799_i32)
  .bind(i64::from(height))
  .bind(1_i32)
  .bind(80_i32)
  .bind(1_i32)
  .bind(format!("{height:064x}"))
  .execute(pool)
  .await?;
  Ok(())
}

async fn seed_transaction(
  pool: &hyphend::sql::SqlPool,
  txid: Vec<u8>,
  block_height: i32,
) -> Result<(), Box<dyn std::error::Error>> {
  sqlx::query(
    "INSERT INTO transactions (
       txid, block_height, block_pos, version, tx_type, lock_time, size,
       value_out, input_count, output_count, fee
     ) VALUES ($1, $2, $3, $4, 'normal', $5, $6, $7, $8, $9, $10)",
  )
  .bind(txid)
  .bind(block_height)
  .bind(0_i32)
  .bind(3_i32)
  .bind(0_i32)
  .bind(250_i32)
  .bind(1_000_i64)
  .bind(1_i32)
  .bind(1_i32)
  .bind(Some(10_i64))
  .execute(pool)
  .await?;
  Ok(())
}

async fn set_stage_heights(
  pool: &hyphend::sql::SqlPool,
  indexed_height: i32,
) -> Result<(), Box<dyn std::error::Error>> {
  for stage in [
    Stage::Block,
    Stage::Transaction,
    Stage::Script,
    Stage::Spent,
    Stage::Metrics,
    Stage::Validator,
  ] {
    hyphend::sql::query::update_indexed_stage_height(pool, stage, indexed_height).await?;
  }
  Ok(())
}

#[tokio::test]
async fn verify_rocks_coverage_skips_empty_postgres_block_index(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;

  let db = Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::populate_block_height_range(&db, 0, 2)?;

  hyphend::tests::verify_rocks_covers_pg(&pool, &db).await?;
  Ok(())
}

#[tokio::test]
async fn verify_rocks_coverage_allows_rocks_to_lead_pg() -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  hyphend::sql::query::update_indexed_stage_height(&pool, hyphend::index::Stage::Block, 1).await?;

  let db = Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::populate_block_height_range(&db, 0, 2)?;

  hyphend::tests::verify_rocks_covers_pg(&pool, &db).await?;
  Ok(())
}

#[tokio::test]
async fn verify_pg_undo_coverage_skips_empty_postgres_block_index(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  hyphend::sql::progress::ensure_chain_progress(&pool).await?;

  hyphend::recovery::verify_pg_undo_coverage_or_empty(&pool).await?;
  Ok(())
}

#[tokio::test]
async fn verify_pg_undo_coverage_rejects_indexed_postgres_without_undo(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  seed_block(&pool, 0).await?;
  hyphend::sql::query::update_indexed_stage_height(&pool, Stage::Block, 0).await?;
  hyphend::sql::progress::ensure_chain_progress(&pool).await?;

  let err = hyphend::recovery::verify_pg_undo_coverage_or_empty(&pool)
    .await
    .expect_err("expected undo coverage failure");
  assert!(matches!(
    err,
    DaemonError::Index(message)
      if message.contains("lacks contiguous undo coverage")
        && message.contains("covered_rows=0")
        && message.contains("expected_rows=1")
        && message.contains("wipe or reindex postgres")
  ));
  Ok(())
}

#[tokio::test]
async fn verify_pg_undo_coverage_accepts_indexed_postgres_with_matching_undo(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  seed_block(&pool, 0).await?;
  hyphend::sql::query::update_indexed_stage_height(&pool, Stage::Block, 0).await?;
  sqlx::query(
    "INSERT INTO block_undo_meta (
       height,
       block_hash,
       prev_hash,
       transactions_total_after_disconnect,
       address_balances_total_after_disconnect,
       address_supply_after_disconnect,
       created_at
     )
     VALUES (0, $1, $2, 0, 0, 0, 0)",
  )
  .bind(vec![0_u8; 32])
  .bind(vec![0_u8; 32])
  .execute(&pool)
  .await?;
  hyphend::sql::progress::ensure_chain_progress(&pool).await?;

  hyphend::recovery::verify_pg_undo_coverage_or_empty(&pool).await?;
  let progress = hyphend::sql::progress::chain_progress(&pool).await?;
  assert_eq!(progress.undo_tip_height, 0);
  Ok(())
}

#[tokio::test]
async fn verify_pg_undo_coverage_rejects_hole_below_tip() -> Result<(), Box<dyn std::error::Error>>
{
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  seed_block(&pool, 0).await?;
  seed_block(&pool, 1).await?;
  hyphend::sql::query::update_indexed_stage_height(&pool, Stage::Block, 1).await?;
  sqlx::query(
    "INSERT INTO block_undo_meta (
       height,
       block_hash,
       prev_hash,
       transactions_total_after_disconnect,
       address_balances_total_after_disconnect,
       address_supply_after_disconnect,
       created_at
     )
     VALUES (1, $1, $2, 0, 0, 0, 0)",
  )
  .bind(vec![1_u8; 32])
  .bind(vec![0_u8; 32])
  .execute(&pool)
  .await?;
  hyphend::sql::progress::ensure_chain_progress(&pool).await?;

  let err = hyphend::recovery::verify_pg_undo_coverage_or_empty(&pool)
    .await
    .expect_err("expected contiguous undo coverage failure");
  assert!(matches!(
    err,
    DaemonError::Index(message)
      if message.contains("lacks contiguous undo coverage")
        && message.contains("covered_rows=1")
        && message.contains("expected_rows=2")
  ));
  Ok(())
}

#[tokio::test]
async fn verify_rocks_coverage_rejects_pg_ahead_of_rocks() -> Result<(), Box<dyn std::error::Error>>
{
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  hyphend::sql::query::update_indexed_stage_height(&pool, hyphend::index::Stage::Block, 3).await?;

  let db = Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::populate_block_height_range(&db, 0, 2)?;

  let err = hyphend::tests::verify_rocks_covers_pg(&pool, &db)
    .await
    .expect_err("expected RocksDB coverage failure");
  assert!(matches!(
    err,
    DaemonError::Db(message) if message.contains("PostgreSQL block index ahead of RocksDB by 1 heights")
  ));

  Ok(())
}

#[tokio::test]
async fn refresh_once_persists_registered_snapshot_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;

  let rpc = FakeRpc {
    registered: vec![ProTxListEntry {
      pro_tx_hash: "11".repeat(32),
      validator_type: None,
      state: Some(ProTxState {
        last_paid_height: Some(42),
        consecutive_payments: Some(2),
        pose_penalty: Some(1),
        pose_revived_height: None,
        pose_ban_height: None,
        revocation_reason: None,
        service: None,
      }),
    }],
    valid: vec![ProTxListEntry {
      pro_tx_hash: "11".repeat(32),
      validator_type: None,
      state: None,
    }],
  };

  refresh_once(&pool, &rpc).await?;
  let rows = hyphend::sql::live::protx::snapshot(&pool).await?;
  assert_eq!(rows.len(), 1);
  assert_eq!(rows[0].current_status.as_deref(), Some("valid"));
  assert_eq!(rows[0].last_paid_height, Some(42));
  Ok(())
}

#[tokio::test]
async fn startup_reconcile_lowers_sparse_local_tip_to_pg_frontier(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  hyphend::sql::query::update_indexed_stage_height(&pool, Stage::Block, 1).await?;

  let db = std::sync::Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::populate_block_height_range(&db, 0, 1)?;
  hyphend::tests::write_tip(&db, 3, [3_u8; 32])?;

  let report = hyphend::sql::verbs::rewind::reconcile_postgres_with_local_rocks(
    &pool,
    std::sync::Arc::clone(&db),
    None,
    0x4c,
    0x10,
    150.0,
  )
  .await?;

  assert_eq!(report.pg_block_before, 1);
  assert_eq!(report.pg_block_after, 1);
  assert_eq!(report.local_tip_before_repair, Some(3));
  assert_eq!(report.local_tip_after_repair, Some(1));
  assert_eq!(report.disconnected_heights, 0);
  assert_eq!(report.clamped_sync_rows, 0);
  assert_eq!(hyphend::tests::tip_height(&db), Some(1));
  Ok(())
}

#[tokio::test]
async fn startup_reconcile_rewinds_postgres_and_clamps_all_stage_heights(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  seed_block(&pool, 0).await?;
  seed_block(&pool, 1).await?;
  set_stage_heights(&pool, 1).await?;

  let db = std::sync::Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::write_tip(&db, 0, [0_u8; 32])?;

  let report = hyphend::sql::verbs::rewind::reconcile_postgres_with_local_rocks(
    &pool,
    std::sync::Arc::clone(&db),
    None,
    0x4c,
    0x10,
    150.0,
  )
  .await?;

  assert_eq!(report.pg_block_before, 1);
  assert_eq!(report.pg_block_after, 0);
  assert_eq!(report.local_tip_after_repair, Some(0));
  assert_eq!(report.disconnected_heights, 1);
  for stage in [
    Stage::Block,
    Stage::Transaction,
    Stage::Script,
    Stage::Spent,
    Stage::Metrics,
    Stage::Validator,
  ] {
    assert_eq!(
      hyphend::sql::query::indexed_stage_height(&pool, stage).await?,
      0
    );
  }
  Ok(())
}

#[tokio::test]
async fn startup_reconcile_is_idempotent_after_repairs() -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  seed_block(&pool, 0).await?;
  seed_block(&pool, 1).await?;
  seed_transaction(&pool, vec![1_u8; 32], 0).await?;
  seed_transaction(&pool, vec![2_u8; 32], 1).await?;
  set_stage_heights(&pool, 1).await?;
  sqlx::query(
    "INSERT INTO address_stats (
       address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
     ) VALUES
       ($1, $2, $2, 1, 1, 0, 0),
       ($3, $4, $4, 1, 1, 0, 0)",
  )
  .bind("XidempotentOne")
  .bind(25_i64)
  .bind("XidempotentTwo")
  .bind(75_i64)
  .execute(&pool)
  .await?;
  sqlx::query(
    "INSERT INTO transactions_summary (singleton, total)
     VALUES (TRUE, -1)
     ON CONFLICT (singleton) DO UPDATE SET total = EXCLUDED.total",
  )
  .execute(&pool)
  .await?;
  sqlx::query(
    "INSERT INTO address_balances_summary (singleton, total, included_supply)
     VALUES (TRUE, -9, -11)
     ON CONFLICT (singleton) DO UPDATE SET total = EXCLUDED.total, included_supply = EXCLUDED.included_supply",
  )
  .execute(&pool)
  .await?;

  let db = std::sync::Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::write_tip(&db, 0, [0_u8; 32])?;

  let first = hyphend::sql::verbs::rewind::reconcile_postgres_with_local_rocks(
    &pool,
    std::sync::Arc::clone(&db),
    None,
    0x4c,
    0x10,
    150.0,
  )
  .await?;
  let tx_summary_after_first = hyphend::sql::index::tx::transactions_summary(&pool).await?;
  let balances_summary_after_first = hyphend::sql::index::spent::balances_summary(&pool).await?;

  let second = hyphend::sql::verbs::rewind::reconcile_postgres_with_local_rocks(
    &pool,
    std::sync::Arc::clone(&db),
    None,
    0x4c,
    0x10,
    150.0,
  )
  .await?;
  let tx_summary_after_second = hyphend::sql::index::tx::transactions_summary(&pool).await?;
  let balances_summary_after_second = hyphend::sql::index::spent::balances_summary(&pool).await?;

  assert_eq!(first.pg_block_after, 0);
  assert_eq!(second.pg_block_before, 0);
  assert_eq!(second.pg_block_after, 0);
  assert_eq!(second.disconnected_heights, 0);
  assert_eq!(second.clamped_sync_rows, 0);
  assert_eq!(tx_summary_after_first.total, tx_summary_after_second.total);
  assert_eq!(
    balances_summary_after_first.total,
    balances_summary_after_second.total
  );
  assert_eq!(
    balances_summary_after_first.included_supply,
    balances_summary_after_second.included_supply
  );
  Ok(())
}

#[tokio::test]
async fn startup_summary_refresh_repairs_transactions_and_balances_singletons(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  seed_block(&pool, 0).await?;
  seed_block(&pool, 1).await?;
  seed_transaction(&pool, vec![1_u8; 32], 0).await?;
  seed_transaction(&pool, vec![2_u8; 32], 1).await?;
  sqlx::query(
    "INSERT INTO address_stats (
       address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
     ) VALUES
       ($1, $2, $2, 1, 1, 0, 0),
       ($3, $4, $4, 1, 1, 0, 0),
       ($5, $6, $6, 1, 1, 0, 0)",
  )
  .bind("XsummaryOne")
  .bind(25_i64)
  .bind("XsummaryTwo")
  .bind(75_i64)
  .bind("XsummaryGone")
  .bind(0_i64)
  .execute(&pool)
  .await?;
  sqlx::query(
    "INSERT INTO transactions_summary (singleton, total)
     VALUES (TRUE, -1)
     ON CONFLICT (singleton) DO UPDATE SET total = EXCLUDED.total",
  )
  .execute(&pool)
  .await?;
  sqlx::query(
    "INSERT INTO address_balances_summary (singleton, total, included_supply)
     VALUES (TRUE, -9, -11)
     ON CONFLICT (singleton) DO UPDATE SET total = EXCLUDED.total, included_supply = EXCLUDED.included_supply",
  )
  .execute(&pool)
  .await?;

  hyphend::sql::index::tx::refresh_transactions_summary(&pool).await?;
  hyphend::sql::index::spent::refresh_address_balances_summary_pool(&pool).await?;

  let tx_summary = hyphend::sql::index::tx::transactions_summary(&pool).await?;
  let balances_summary = hyphend::sql::index::spent::balances_summary(&pool).await?;
  assert_eq!(tx_summary.total, 2);
  assert_eq!(balances_summary.total, 2);
  assert_eq!(balances_summary.included_supply, 100);
  Ok(())
}

#[tokio::test]
async fn schema_validate_reports_missing_address_balances_table(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  sqlx::query("DROP TABLE IF EXISTS address_balances CASCADE")
    .execute(&pool)
    .await?;

  let problems = hyphend::sql::bootstrap::validate(&pool).await?;

  assert!(problems
    .iter()
    .any(|problem| problem == "address_balances: missing table"));
  Ok(())
}

#[tokio::test]
async fn schema_validate_reports_missing_address_balances_undo_table(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  sqlx::query("DROP TABLE IF EXISTS block_undo_address_balances CASCADE")
    .execute(&pool)
    .await?;

  let problems = hyphend::sql::bootstrap::validate(&pool).await?;

  assert!(problems
    .iter()
    .any(|problem| problem == "block_undo_address_balances: missing table"));
  Ok(())
}

#[tokio::test]
async fn schema_validate_reports_missing_foreign_key_support_index(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  sqlx::query("DROP INDEX IF EXISTS idx_address_txs_txid")
    .execute(&pool)
    .await?;

  let problems = hyphend::sql::bootstrap::validate(&pool).await?;

  assert!(problems.iter().any(|problem| {
    problem.contains("address_txs: foreign key") && problem.contains("on (txid)")
  }));
  Ok(())
}

#[tokio::test]
async fn schema_validate_accepts_left_prefix_foreign_key_support_index(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  sqlx::query("DROP INDEX IF EXISTS idx_address_txs_txid")
    .execute(&pool)
    .await?;
  sqlx::query("CREATE INDEX idx_address_txs_txid_alt ON address_txs (txid, address)")
    .execute(&pool)
    .await?;

  let problems = hyphend::sql::bootstrap::validate(&pool).await?;

  assert!(!problems.iter().any(|problem| {
    problem.contains("address_txs: foreign key") && problem.contains("on (txid)")
  }));
  Ok(())
}

#[tokio::test]
async fn schema_validate_reports_missing_governance_jsonb_index(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  sqlx::query("DROP INDEX IF EXISTS idx_governance_objects_data")
    .execute(&pool)
    .await?;

  let problems = hyphend::sql::bootstrap::validate(&pool).await?;

  assert!(problems
    .iter()
    .any(|problem| problem == "missing index idx_governance_objects_data"));
  Ok(())
}

#[tokio::test]
async fn startup_reconcile_finishes_pending_local_disconnect(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  seed_block(&pool, 0).await?;
  hyphend::sql::query::update_indexed_stage_height(&pool, Stage::Block, 0).await?;
  hyphend::sql::progress::ensure_chain_progress(&pool).await?;
  hyphend::sql::progress::mark_recovery_state(&pool, "disconnecting", Some(1)).await?;

  let db = std::sync::Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::populate_block_height_range(&db, 0, 1)?;

  let report = hyphend::sql::verbs::rewind::reconcile_postgres_with_local_rocks(
    &pool,
    std::sync::Arc::clone(&db),
    None,
    0x4c,
    0x10,
    150.0,
  )
  .await?;

  assert_eq!(report.pg_block_before, 0);
  assert_eq!(report.pg_block_after, 0);
  assert_eq!(hyphend::tests::tip_height(&db), Some(0));
  let progress = hyphend::sql::progress::chain_progress(&pool).await?;
  assert_eq!(progress.recovery_state, "clean");
  assert_eq!(progress.recovery_height, None);
  Ok(())
}
