//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use hyphend::index::Stage;
use hyphend::sql;
use hyphend::sql::verbs::rewind::disconnect_tip;

/// Seed one canonical block row because rewind tests must drive the real public entrypoint.
async fn seed_block(
  pool: &hyphend::sql::SqlPool,
  height: u32,
  hash: &[u8],
  prev_hash: &[u8],
  merkle_root: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
  let mut tx = pool.begin().await?;
  hyphend::sql::index::block::insert_block(
    &mut tx,
    height,
    hash,
    prev_hash,
    merkle_root,
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

async fn seed_transaction(
  pool: &hyphend::sql::SqlPool,
  txid: &[u8],
  block_height: i32,
) -> Result<(), Box<dyn std::error::Error>> {
  sqlx::query(
    "INSERT INTO transactions (
       txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
     ) VALUES ($1, $2, 0, 3, 'normal', 0, 200, 1000, 1, 1, 10)",
  )
  .bind(txid)
  .bind(block_height)
  .execute(pool)
  .await?;
  Ok(())
}

#[tokio::test]
async fn disconnect_tip_removes_genesis_from_public_entrypoint(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  sql::bootstrap::reset(&pool).await?;

  seed_block(&pool, 0, &[1_u8; 32], &[2_u8; 32], &[3_u8; 32]).await?;
  sql::query::update_indexed_stage_height(&pool, Stage::Block, 0).await?;

  let db = std::sync::Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::write_tip(&db, 0, [1_u8; 32])?;

  let message = disconnect_tip(&pool, &db, None, 0x4c, 0x10, 150.0).await?;

  assert_eq!(message, "disconnected 1 block, block index is now empty");
  assert_eq!(
    sql::query::indexed_stage_height(&pool, Stage::Block).await?,
    -1
  );
  assert!(hyphend::tests::tip_height(&db).is_none());
  assert!(hyphend::tests::tip_hash(&db).is_none());
  assert!(hyphend::tests::hash_for_height(&db, 0).is_none());

  Ok(())
}

#[tokio::test]
async fn disconnect_tip_rewinds_one_height_from_public_entrypoint(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  sql::bootstrap::reset(&pool).await?;

  seed_block(&pool, 0, &[1_u8; 32], &[2_u8; 32], &[3_u8; 32]).await?;
  seed_block(&pool, 1, &[4_u8; 32], &[1_u8; 32], &[5_u8; 32]).await?;
  sql::query::update_indexed_stage_height(&pool, Stage::Block, 1).await?;

  let db = std::sync::Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::write_tip(&db, 0, [1_u8; 32])?;
  hyphend::tests::write_tip(&db, 1, [4_u8; 32])?;

  let message = disconnect_tip(&pool, &db, None, 0x4c, 0x10, 150.0).await?;

  assert_eq!(message, "rewound 1 blocks, now at height 0");
  assert_eq!(
    sql::query::indexed_stage_height(&pool, Stage::Block).await?,
    0
  );
  assert_eq!(hyphend::tests::tip_height(&db), Some(0));
  assert_eq!(hyphend::tests::tip_hash(&db), Some([1_u8; 32]));
  assert_eq!(hyphend::tests::hash_for_height(&db, 0), Some([1_u8; 32]));
  assert!(hyphend::tests::hash_for_height(&db, 1).is_none());

  Ok(())
}

#[tokio::test]
async fn disconnect_tip_self_heals_corrupted_summaries_and_address_stats(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  sql::bootstrap::reset(&pool).await?;

  seed_block(&pool, 0, &[1_u8; 32], &[2_u8; 32], &[3_u8; 32]).await?;
  seed_block(&pool, 1, &[4_u8; 32], &[1_u8; 32], &[5_u8; 32]).await?;
  let txid0 = vec![8_u8; 32];
  let txid1 = vec![9_u8; 32];
  seed_transaction(&pool, &txid0, 0).await?;
  seed_transaction(&pool, &txid1, 1).await?;
  sqlx::query(
    "INSERT INTO tx_outputs (
       txid, vout_index, block_height, value, script_pubkey, script_type, address, spent_txid, spent_vin_index
     ) VALUES ($1, 0, 0, 40, $2, 'p2pkh', $3, NULL, NULL),
              ($4, 0, 1, 60, $5, 'p2pkh', $6, NULL, NULL)",
  )
  .bind(&txid0)
  .bind(vec![0x51_u8])
  .bind("XrewindSelfHeal11111111111111111111")
  .bind(&txid1)
  .bind(vec![0x51_u8])
  .bind("XrewindSelfHeal11111111111111111111")
  .execute(&pool)
  .await?;
  sqlx::query(
    "INSERT INTO address_txs (address, txid, block_height, block_pos)
     VALUES ($1, $2, 0, 0), ($1, $3, 1, 0)",
  )
  .bind("XrewindSelfHeal11111111111111111111")
  .bind(&txid0)
  .bind(&txid1)
  .execute(&pool)
  .await?;
  sqlx::query(
    "INSERT INTO address_stats (
       address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
     ) VALUES ($1, 999, 999, 50, 50, 1, 99)",
  )
  .bind("XrewindSelfHeal11111111111111111111")
  .execute(&pool)
  .await?;
  sqlx::query(
    "INSERT INTO transactions_summary (singleton, total)
     VALUES (TRUE, -13)
     ON CONFLICT (singleton) DO UPDATE SET total = EXCLUDED.total",
  )
  .execute(&pool)
  .await?;
  sqlx::query(
    "INSERT INTO address_balances_summary (singleton, total, included_supply)
     VALUES (TRUE, -214, 1563655803660)
     ON CONFLICT (singleton) DO UPDATE SET total = EXCLUDED.total, included_supply = EXCLUDED.included_supply",
  )
  .execute(&pool)
  .await?;
  sql::query::update_indexed_stage_height(&pool, Stage::Block, 1).await?;
  sql::query::update_indexed_stage_height(&pool, Stage::Transaction, 1).await?;
  sql::query::update_indexed_stage_height(&pool, Stage::Spent, 1).await?;

  let db = std::sync::Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::write_tip(&db, 0, [1_u8; 32])?;
  hyphend::tests::write_tip(&db, 1, [4_u8; 32])?;

  let message = disconnect_tip(&pool, &db, None, 0x4c, 0x10, 150.0).await?;

  assert_eq!(message, "rewound 1 blocks, now at height 0");
  assert_eq!(
    sql::query::indexed_stage_height(&pool, Stage::Block).await?,
    0
  );
  assert_eq!(
    hyphend::sql::index::tx::transactions_summary(&pool)
      .await?
      .total,
    1
  );
  let balances_summary = hyphend::sql::index::spent::balances_summary(&pool).await?;
  assert_eq!(balances_summary.total, 1);
  assert_eq!(balances_summary.included_supply, 40);
  let stats = sqlx::query_as::<_, (i64, i64, i32, i32, Option<i32>, Option<i32>)>(
    "SELECT balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
       FROM address_stats
      WHERE address = $1",
  )
  .bind("XrewindSelfHeal11111111111111111111")
  .fetch_one(&pool)
  .await?;
  assert_eq!(stats, (40, 40, 1, 1, Some(0), Some(0)));

  Ok(())
}

#[tokio::test]
async fn disconnect_tip_recovers_from_deeply_negative_transactions_summary(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  sql::bootstrap::reset(&pool).await?;

  seed_block(&pool, 0, &[1_u8; 32], &[2_u8; 32], &[3_u8; 32]).await?;
  seed_block(&pool, 1, &[4_u8; 32], &[1_u8; 32], &[5_u8; 32]).await?;
  seed_transaction(&pool, &[8_u8; 32], 0).await?;
  seed_transaction(&pool, &[9_u8; 32], 1).await?;
  sqlx::query(
    "INSERT INTO transactions_summary (singleton, total)
     VALUES (TRUE, -2817)
     ON CONFLICT (singleton) DO UPDATE SET total = EXCLUDED.total",
  )
  .execute(&pool)
  .await?;
  sql::query::update_indexed_stage_height(&pool, Stage::Block, 1).await?;
  sql::query::update_indexed_stage_height(&pool, Stage::Transaction, 1).await?;

  let db = std::sync::Arc::new(hyphend::tests::open_temp_db()?);
  hyphend::tests::write_tip(&db, 0, [1_u8; 32])?;
  hyphend::tests::write_tip(&db, 1, [4_u8; 32])?;

  let message = disconnect_tip(&pool, &db, None, 0x4c, 0x10, 150.0).await?;

  assert_eq!(message, "rewound 1 blocks, now at height 0");
  assert_eq!(
    hyphend::sql::index::tx::transactions_summary(&pool)
      .await?
      .total,
    1
  );
  let remaining = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM transactions")
    .fetch_one(&pool)
    .await?;
  assert_eq!(remaining, 1);

  Ok(())
}
