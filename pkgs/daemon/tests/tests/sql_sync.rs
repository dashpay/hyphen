//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use hyphend::index::Stage;
use hyphend::sql;
use hyphend::sql::sync::{
  durable_stage_height, indexed_height, indexed_stage_height, set_local_async_commit,
  sync_state_rows, table_row_count, update_indexed_stage_height, update_indexed_stage_height_tx,
};

/// Seed one canonical block row because sync-state tests must observe durable block height.
async fn seed_block_row(
  pool: &hyphend::sql::SqlPool,
  height: u32,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut tx = pool.begin().await?;
  hyphend::sql::index::block::insert_block(
    &mut tx,
    height,
    &[height as u8 + 1; 32],
    &[height as u8 + 2; 32],
    &[height as u8 + 3; 32],
    height as i32,
    0,
    0,
    1,
    80,
    1,
    "00",
    &hyphend::index::block::util::CbTxFields::default(),
  )
  .await?;
  tx.commit().await?;
  Ok(())
}

#[tokio::test]
async fn sync_queries_read_and_update_stage_rows_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  sql::bootstrap::reset(&pool).await?;

  assert_eq!(indexed_height(&pool, "block").await?, -1);
  assert_eq!(indexed_stage_height(&pool, Stage::Block).await?, -1);

  update_indexed_stage_height(&pool, Stage::Block, 7).await?;
  assert_eq!(indexed_stage_height(&pool, Stage::Block).await?, 7);

  let rows = sync_state_rows(&pool).await?;
  assert!(
    rows
      .iter()
      .any(|row| row.stage == "block" && row.indexed_height == 7),
    "sync_state_rows should expose the block stage after updates"
  );

  Ok(())
}

#[tokio::test]
async fn sync_queries_update_transactional_height_and_durable_state_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  sql::bootstrap::reset(&pool).await?;
  seed_block_row(&pool, 0).await?;
  seed_block_row(&pool, 1).await?;

  assert_eq!(durable_stage_height(&pool, Stage::Block).await?, 1);

  let mut tx = pool.begin().await?;
  set_local_async_commit(&mut tx).await?;
  update_indexed_stage_height_tx(&mut tx, Stage::Block, 1, 1234).await?;
  tx.commit().await?;

  let rows = sync_state_rows(&pool).await?;
  assert!(
    rows
      .iter()
      .any(|row| row.stage == "block" && row.indexed_height == 1 && row.updated_at == 1234),
    "transactional sync updates should persist indexed height and timestamp"
  );
  assert_eq!(table_row_count(&pool, "blocks").await?, 2);

  Ok(())
}
