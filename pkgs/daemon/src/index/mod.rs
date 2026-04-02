//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Index pipeline traits and stage metadata.
//!
//! Forward processing now happens in the block loop as a single
//! per-height PostgreSQL transaction across all enabled stages.

pub mod batch;
pub mod block;
mod block_ctx;
mod connect;
mod control;
mod disconnect;
pub mod metrics;
pub mod runner;
pub mod script;
pub mod spent;
mod stage;
pub mod tx;
pub mod validator;
pub mod visor;

use crate::error::DaemonError;

pub use block_ctx::BlockCtx;
#[cfg(test)]
pub(crate) use connect::sync_stages_for;
pub use connect::ForwardPipeline;
pub(crate) use control::{
  block_indexer_supervised_task, indexer_channel, BlockIndexerWorker, IndexerControl,
  IndexerHandle, IndexerLoopCtx,
};
#[cfg(test)]
pub(crate) use disconnect::rewound_height;
pub use disconnect::DisconnectPipeline;
pub use stage::Stage;
pub(crate) use stage::{
  connect_stage_order, disconnect_stage_order, enabled_stage_order, exact_stage_order,
  IndexStageFlags,
};

/// Async operations for connecting/disconnecting blocks.
#[async_trait::async_trait]
pub trait IndexOps: Send + Sync {
  /// Reverse all effects owned by this stage for one canonical block.
  ///
  /// # Errors
  ///
  /// Returns a stage-specific `DaemonError` when the disconnect cannot be
  /// planned or applied inside the provided transaction.
  async fn disconnect_block(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    height: u32,
  ) -> Result<(), DaemonError>;

  /// Clear any in-memory caches after rewind.
  fn reset_cache(&self) {}
}

#[cfg(test)]
mod tests {
  use std::collections::{BTreeSet, HashMap};
  use std::sync::Arc;
  use std::time::{SystemTime, UNIX_EPOCH};

  use dash_primitives::block::full::Block;
  use dash_primitives::hash;
  use rocksdb::IteratorMode;
  use rocksdb::DB;

  use super::{
    connect_stage_order, disconnect_stage_order, enabled_stage_order, rewound_height,
    sync_stages_for, IndexStageFlags, Stage,
  };

  #[test]
  fn stage_table_ownership_is_unique() {
    let stages = [
      Stage::Block,
      Stage::Transaction,
      Stage::Metrics,
      Stage::Validator,
      Stage::Script,
      Stage::Spent,
    ];
    let mut owners: HashMap<&str, &str> = HashMap::new();
    for stage in stages {
      for table in stage.tables() {
        let prev = owners.insert(table, stage.name());
        assert!(
          prev.is_none(),
          "table {table} owned by both {prev:?} and {}",
          stage.name()
        );
      }
    }
  }

  #[test]
  fn special_tx_payloads_belongs_to_transaction_stage() {
    assert!(Stage::Transaction.tables().contains(&"special_tx_payloads"));
    assert!(!Stage::Validator.tables().contains(&"special_tx_payloads"));
  }

  #[test]
  fn block_reset_order_covers_all_downstream_stages_once() {
    let stages: BTreeSet<&str> = Stage::Block
      .reset_stages()
      .iter()
      .copied()
      .map(Stage::name)
      .collect();
    let expected: BTreeSet<&str> = [
      "block",
      "transaction",
      "metrics",
      "validator",
      "script",
      "spent",
    ]
    .into_iter()
    .collect();
    assert_eq!(stages, expected);
  }

  #[test]
  fn forward_plan_syncs_block_and_enabled_stages() {
    let sync_stages = sync_stages_for(&[Stage::Transaction, Stage::Script, Stage::Spent]);
    assert_eq!(
      sync_stages,
      vec![
        Stage::Block,
        Stage::Transaction,
        Stage::Script,
        Stage::Spent
      ]
    );
  }

  #[test]
  fn enabled_stage_order_matches_connect_dependencies() {
    assert_eq!(
      enabled_stage_order(IndexStageFlags {
        address: true,
        metrics: true,
        validator: true,
      }),
      vec![
        Stage::Transaction,
        Stage::Script,
        Stage::Spent,
        Stage::Metrics,
        Stage::Validator
      ]
    );
    assert_eq!(
      enabled_stage_order(IndexStageFlags {
        address: false,
        metrics: true,
        validator: false,
      }),
      vec![Stage::Transaction, Stage::Metrics]
    );
  }

  #[test]
  fn typed_connect_stage_order_includes_block_and_enabled_stages() {
    assert_eq!(
      connect_stage_order(&[Stage::Transaction, Stage::Metrics]),
      vec![Stage::Block, Stage::Transaction, Stage::Metrics]
    );
    assert_eq!(
      connect_stage_order(&[Stage::Transaction, Stage::Script, Stage::Spent]),
      vec![
        Stage::Block,
        Stage::Transaction,
        Stage::Script,
        Stage::Spent
      ]
    );
  }

  #[test]
  fn disconnect_order_reuses_block_reset_order() {
    assert_eq!(
      disconnect_stage_order(),
      Stage::Block.reset_stages().to_vec()
    );
  }

  #[test]
  fn disconnecting_height_zero_rewinds_to_negative_one() {
    assert_eq!(rewound_height(0), -1);
    assert_eq!(rewound_height(7), 6);
  }

  #[test]
  fn block_context_matches_legacy_tx_derivation() -> Result<(), Box<dyn std::error::Error>> {
    let raw = minimal_block_bytes(41);
    let ctx = super::BlockCtx::from_raw(0, raw.clone())?;
    let block = Block::from_bytes(&raw)?;
    let ranges = hash::tx_byte_ranges(&raw)?;

    assert_eq!(ctx.height(), 0);
    assert_eq!(ctx.block().transactions.len(), block.transactions.len());
    assert_eq!(ctx.raw(), raw.as_slice());
    assert_eq!(ctx.block_hash(), hash::block_hash(&raw)?.0);

    for (index, (start, end)) in ranges.into_iter().enumerate() {
      assert_eq!(ctx.tx_byte_range(index), Some((start, end)));
      assert_eq!(
        ctx.txid(index).copied(),
        Some(hash::tx_hash(&raw[start..end]).0)
      );
    }

    Ok(())
  }

  #[test]
  fn block_context_rejects_truncated_block() {
    let mut raw = minimal_block_bytes(7);
    raw.truncate(raw.len().saturating_sub(3));
    assert!(super::BlockCtx::from_raw(0, raw).is_err());
  }

  #[tokio::test]
  async fn forward_and_disconnect_round_trip_one_height_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };

    let harness = TestHarness::new(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
      vec![Stage::Transaction, Stage::Metrics],
    )
    .await?;

    harness.connect_block(0, 41).await?;
    harness.connect_block(1, 42).await?;
    harness.disconnect_block(1).await?;

    let block_count = crate::sql::query::table_row_count(&harness.pool, "blocks").await?;
    let transaction_count =
      crate::sql::query::table_row_count(&harness.pool, "transactions").await?;
    let metrics_count = crate::sql::query::table_row_count(&harness.pool, "block_metrics").await?;

    assert_eq!(block_count, 1);
    assert_eq!(transaction_count, 1);
    assert_eq!(metrics_count, 1);
    assert_eq!(
      crate::sql::query::indexed_stage_height(&harness.pool, Stage::Block).await?,
      0
    );
    assert_eq!(
      crate::sql::query::indexed_stage_height(&harness.pool, Stage::Transaction).await?,
      0
    );
    assert_eq!(
      crate::sql::query::indexed_stage_height(&harness.pool, Stage::Metrics).await?,
      0
    );

    Ok(())
  }

  #[tokio::test]
  async fn forward_and_disconnect_genesis_round_trip_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };

    let harness = TestHarness::new(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
      vec![Stage::Transaction, Stage::Metrics],
    )
    .await?;

    harness.connect_block(0, 99).await?;
    harness.disconnect_block(0).await?;

    let block_count = crate::sql::query::table_row_count(&harness.pool, "blocks").await?;
    let transaction_count =
      crate::sql::query::table_row_count(&harness.pool, "transactions").await?;
    let metrics_count = crate::sql::query::table_row_count(&harness.pool, "block_metrics").await?;

    assert_eq!(block_count, 0);
    assert_eq!(transaction_count, 0);
    assert_eq!(metrics_count, 0);
    assert_eq!(
      crate::sql::query::indexed_stage_height(&harness.pool, Stage::Block).await?,
      -1
    );
    assert_eq!(
      crate::sql::query::indexed_stage_height(&harness.pool, Stage::Transaction).await?,
      -1
    );
    assert_eq!(
      crate::sql::query::indexed_stage_height(&harness.pool, Stage::Metrics).await?,
      -1
    );

    Ok(())
  }

  fn minimal_block_bytes(nonce: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&4_i32.to_le_bytes());
    buf.extend_from_slice(&[0xAA; 32]);
    buf.extend_from_slice(&[0xBB; 32]);
    buf.extend_from_slice(&1_700_000_000_u32.to_le_bytes());
    buf.extend_from_slice(&0x1A0F_FFFF_u32.to_le_bytes());
    buf.extend_from_slice(&nonce.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&1_i32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&[0x00; 32]);
    buf.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    buf.push(0x04);
    buf.extend_from_slice(&[nonce as u8, 0x02, 0x03, 0x04]);
    buf.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&50_0000_0000_i64.to_le_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&0_u32.to_le_bytes());
    buf
  }

  #[tokio::test]
  async fn address_stage_round_trip_when_pg_is_configured() -> Result<(), Box<dyn std::error::Error>>
  {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };

    let harness = TestHarness::new(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
      vec![Stage::Transaction, Stage::Script, Stage::Spent],
    )
    .await?;

    harness.connect_raw_block(0, p2pkh_block_bytes(7)).await?;

    let address_stats_count =
      crate::sql::query::table_row_count(&harness.pool, "address_stats").await?;
    let address_history_count =
      crate::sql::query::table_row_count(&harness.pool, "address_txs").await?;
    assert_eq!(address_stats_count, 1);
    assert_eq!(address_history_count, 0);

    harness.disconnect_block(0).await?;

    let address_stats_count =
      crate::sql::query::table_row_count(&harness.pool, "address_stats").await?;
    let address_history_count =
      crate::sql::query::table_row_count(&harness.pool, "address_txs").await?;
    assert_eq!(address_stats_count, 0);
    assert_eq!(address_history_count, 0);
    assert_eq!(
      crate::sql::query::indexed_stage_height(&harness.pool, Stage::Script).await?,
      -1
    );
    assert_eq!(
      crate::sql::query::indexed_stage_height(&harness.pool, Stage::Spent).await?,
      -1
    );

    Ok(())
  }

  #[tokio::test]
  async fn secondary_store_and_postgres_stay_in_sync_for_address_stage_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };

    let harness = TestHarness::new(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
      vec![Stage::Transaction, Stage::Script, Stage::Spent],
    )
    .await?;

    harness.connect_raw_block(0, p2pkh_block_bytes(11)).await?;

    let prevout_db = harness
      .secondary_stores
      .db(crate::store::StoreId::PrevoutLookup)?;
    let prevout_cf = prevout_db
      .cf_handle(crate::store::CF_DATA)
      .ok_or("prevout data CF missing")?;
    let prevout_rows = prevout_db
      .iterator_cf(prevout_cf, IteratorMode::Start)
      .count();
    assert_eq!(prevout_rows, 1);

    let history_db = harness
      .secondary_stores
      .db(crate::store::StoreId::AddressHistory)?;
    let history_cf = history_db
      .cf_handle(crate::store::CF_DATA)
      .ok_or("history data CF missing")?;
    let history_rows = history_db
      .iterator_cf(history_cf, IteratorMode::Start)
      .count();
    assert_eq!(history_rows, 1);

    let address = dash_primitives::script::derive_address(
      &[
        0x76, 0xA9, 0x14, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B,
        0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x88, 0xAC,
      ],
      76,
      16,
    )
    .ok_or("missing derived address")?;
    let history = crate::store::address_history::page_rows(history_db, &address, 0, 10, None)?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].block_height, 0);

    harness.disconnect_block(0).await?;

    let prevout_rows = prevout_db
      .iterator_cf(prevout_cf, IteratorMode::Start)
      .count();
    let history_rows = history_db
      .iterator_cf(history_cf, IteratorMode::Start)
      .count();
    assert_eq!(prevout_rows, 0);
    assert_eq!(history_rows, 0);
    let history = crate::store::address_history::page_rows(history_db, &address, 0, 10, None)?;
    assert!(history.is_empty());

    Ok(())
  }

  #[tokio::test]
  async fn public_disconnect_tip_shared_rewinds_secondary_stores_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };

    let harness = TestHarness::new(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
      vec![Stage::Transaction, Stage::Script, Stage::Spent],
    )
    .await?;

    harness.connect_raw_block(0, p2pkh_block_bytes(12)).await?;

    let message = crate::sql::verbs::rewind::disconnect_tip_shared(
      harness.pool.clone(),
      Arc::clone(&harness.db),
      Some(Arc::clone(&harness.secondary_stores)),
      76,
      16,
      150.0,
    )
    .await?;

    assert_eq!(message, "disconnected 1 block, block index is now empty");

    let prevout_db = harness
      .secondary_stores
      .db(crate::store::StoreId::PrevoutLookup)?;
    let prevout_cf = prevout_db
      .cf_handle(crate::store::CF_DATA)
      .ok_or("prevout data CF missing")?;
    assert_eq!(
      prevout_db
        .iterator_cf(prevout_cf, IteratorMode::Start)
        .count(),
      0
    );

    let history_db = harness
      .secondary_stores
      .db(crate::store::StoreId::AddressHistory)?;
    let history_cf = history_db
      .cf_handle(crate::store::CF_DATA)
      .ok_or("history data CF missing")?;
    assert_eq!(
      history_db
        .iterator_cf(history_cf, IteratorMode::Start)
        .count(),
      0
    );

    Ok(())
  }

  #[tokio::test]
  async fn address_stage_queries_read_materialized_address_domain_tables_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };

    let harness = TestHarness::new(
      &database_url
        .into_string()
        .map_err(|_| "invalid database url")?,
      vec![Stage::Transaction, Stage::Script, Stage::Spent],
    )
    .await?;

    harness.connect_raw_block(0, p2pkh_block_bytes(7)).await?;

    let address = dash_primitives::script::derive_address(
      &[
        0x76, 0xA9, 0x14, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B,
        0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x88, 0xAC,
      ],
      76,
      16,
    )
    .ok_or("missing derived address")?;

    let summary = crate::index::spent::sql::address_summary(&harness.pool, &address)
      .await?
      .ok_or("missing address summary")?;
    assert_eq!(summary.balance, 50_0000_0000);
    assert_eq!(summary.total_received, 50_0000_0000);
    assert_eq!(summary.total_sent, 0);
    assert_eq!(summary.tx_count, 1);
    assert_eq!(summary.first_seen_height, Some(0));
    assert_eq!(summary.last_seen_height, Some(0));

    let history =
      crate::index::spent::sql::address_history_txs(&harness.pool, &address, 10, None).await?;
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].block_height, 0);
    assert_eq!(history[0].block_pos, 0);

    let balances = crate::index::spent::sql::balances_page(&harness.pool, 10, None).await?;
    assert_eq!(balances.len(), 1);
    assert_eq!(balances[0].address, address);
    assert_eq!(balances[0].balance, 50_0000_0000);
    assert_eq!(balances[0].total_received, 50_0000_0000);
    assert_eq!(balances[0].total_sent, 0);
    assert_eq!(balances[0].tx_count, 1);

    Ok(())
  }

  fn p2pkh_block_bytes(nonce: u32) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&4_i32.to_le_bytes());
    buf.extend_from_slice(&[0xAA; 32]);
    buf.extend_from_slice(&[0xCC; 32]);
    buf.extend_from_slice(&1_700_000_000_u32.to_le_bytes());
    buf.extend_from_slice(&0x1A0F_FFFF_u32.to_le_bytes());
    buf.extend_from_slice(&nonce.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&1_i32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&[0x00; 32]);
    buf.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    buf.push(0x04);
    buf.extend_from_slice(&[nonce as u8, 0x02, 0x03, 0x04]);
    buf.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&50_0000_0000_i64.to_le_bytes());
    buf.push(0x19);
    buf.extend_from_slice(&[
      0x76, 0xA9, 0x14, 0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B,
      0x1C, 0x1D, 0x1E, 0x1F, 0x20, 0x21, 0x22, 0x23, 0x24, 0x88, 0xAC,
    ]);
    buf.extend_from_slice(&0_u32.to_le_bytes());
    buf
  }

  struct TestHarness {
    pool: crate::sql::SqlPool,
    db: Arc<DB>,
    db_path: std::path::PathBuf,
    secondary_stores: Arc<crate::store::manager::SecondaryStoreSet>,
    forward: super::ForwardPipeline,
    disconnect: super::DisconnectPipeline,
    enabled_stages: Arc<Vec<Stage>>,
  }

  impl TestHarness {
    async fn new(
      database_url: &str,
      enabled_stages: Vec<Stage>,
    ) -> Result<Self, Box<dyn std::error::Error>> {
      let pool = crate::sql::connect_test_pool(database_url).await?;
      crate::sql::bootstrap::reset(&pool).await?;

      let db_path = std::env::temp_dir().join(format!(
        "hyphen-transition-test-{}",
        SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos()
      ));
      let db = Arc::new(DB::open_default(&db_path)?);
      let enabled_stages = Arc::new(enabled_stages);
      let spent_cache = Arc::new(crate::index::spent::cache::SpentQueryCache::disabled());
      let tx_batch_sizer = Arc::new(std::sync::Mutex::new(crate::index::batch::BatchSizer::new(
        crate::index::batch::DEFAULT_COMMIT_BATCH_BLOCKS,
        crate::index::batch::DEFAULT_MIN_BATCH_SIZE,
        crate::index::batch::DEFAULT_MAX_BATCH_SIZE,
        crate::index::batch::DEFAULT_TARGET_BATCH_MS,
      )));
      let spent_batch_sizer =
        Arc::new(std::sync::Mutex::new(crate::index::batch::BatchSizer::new(
          crate::index::batch::DEFAULT_COMMIT_BATCH_BLOCKS,
          crate::index::batch::DEFAULT_MIN_BATCH_SIZE,
          crate::index::batch::DEFAULT_MAX_BATCH_SIZE,
          crate::index::batch::DEFAULT_TARGET_BATCH_MS,
        )));
      let ingest_runtime = Arc::new(crate::ingest::IngestRuntime::default());
      let lifecycle = crate::lifecycle::Lifecycle::start();
      let scheduler_state = Arc::new(std::sync::Mutex::new(
        crate::index::runner::IndexSchedulerState::default(),
      ));
      let block_worker = Arc::new(crate::index::BlockIndexerWorker::new(
        Arc::clone(&db),
        Arc::clone(&ingest_runtime),
        Arc::clone(&lifecycle),
        Arc::clone(&scheduler_state),
        Arc::new(std::sync::atomic::AtomicBool::new(false)),
      ));
      let secondary_stores = Arc::new(crate::store::manager::SecondaryStoreSet::open(
        &db_path,
        &crate::config::ChainConfig::default(),
        None,
      )?);
      let ctx = super::IndexerLoopCtx {
        db: Arc::clone(&db),
        secondary_stores: Some(Arc::clone(&secondary_stores)),
        pool: pool.clone(),
        block_worker,
        event_tx: tokio::sync::broadcast::channel(1).0,
        lifecycle,
        enabled_stages: Arc::clone(&enabled_stages),
        chain_state_lock: Arc::new(tokio::sync::RwLock::new(())),
        scheduler_state,
        spent_cache: Arc::clone(&spent_cache),
        history_partition_cache: Arc::new(std::sync::Mutex::new(None)),
        canonical_partition_cache: Arc::new(std::sync::Mutex::new(None)),
        tx_batch_sizer,
        spent_batch_sizer,
        operator_pause_requested: Arc::new(std::sync::atomic::AtomicBool::new(false)),
        p2pkh_version: 76,
        p2sh_version: 16,
        block_time_secs: 150.0,
      };

      Ok(Self {
        pool,
        db: Arc::clone(&db),
        db_path,
        secondary_stores: Arc::clone(&secondary_stores),
        forward: super::ForwardPipeline::new(&ctx),
        disconnect: super::DisconnectPipeline::new(
          Some(Arc::clone(&db)),
          Some(Arc::clone(&secondary_stores)),
          76,
          16,
          150.0,
          spent_cache,
        ),
        enabled_stages,
      })
    }

    async fn connect_block(
      &self,
      height: u32,
      nonce: u32,
    ) -> Result<(), Box<dyn std::error::Error>> {
      self
        .connect_raw_block(height, minimal_block_bytes(nonce))
        .await
    }

    async fn connect_raw_block(
      &self,
      height: u32,
      raw: Vec<u8>,
    ) -> Result<(), Box<dyn std::error::Error>> {
      let mut tx = self.pool.begin().await?;
      let block = super::BlockCtx::from_raw(height, raw)?;
      let plan = self
        .forward
        .plan_connect(&mut tx, &block, &self.enabled_stages)
        .await?;
      plan.apply(&self.forward, &mut tx, None).await?;
      tx.commit().await?;
      Ok(())
    }

    async fn disconnect_block(&self, height: u32) -> Result<(), Box<dyn std::error::Error>> {
      let mut tx = self.pool.begin().await?;
      let plan = self.disconnect.plan_disconnect(&self.pool, height).await?;
      plan.apply(&self.disconnect, &mut tx).await?;
      tx.commit().await?;
      Ok(())
    }
  }

  impl Drop for TestHarness {
    fn drop(&mut self) {
      let _ = Arc::strong_count(&self.db);
      let _ = std::fs::remove_dir_all(&self.db_path);
    }
  }
}
