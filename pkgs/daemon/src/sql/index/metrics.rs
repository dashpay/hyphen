//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL queries for the block metrics index.

use sqlx::PgPool;

use crate::error::DaemonError;

#[derive(Debug, Clone)]
pub struct MetricsRow {
  pub height: i32,
  pub tx_count: i64,
  pub utxos_spendable: i64,
  pub utxos_unspendable: i64,
  pub blockchain_size: i64,
  pub credit_pool_balance: Option<i64>,
  pub circulating_supply: i64,
  pub difficulty: f64,
  pub hashrate: f64,
}

/// Read the previous metrics snapshot used to derive the next block metrics row.
///
/// # Errors
///
/// Returns an error when the previous metrics row is missing for a non-genesis
/// height or when PostgreSQL query execution fails.
pub async fn query_prev_metrics(
  conn: &mut sqlx::PgConnection,
  height: u32,
) -> Result<(i64, i64, i64, i64, i64), DaemonError> {
  if height == 0 {
    return Ok((0, 0, 0, 0, 0));
  }
  let prev = (height - 1) as i32;
  let row: Option<(i64, i64, i64, i64, i64)> = sqlx::query_as(
    "SELECT circulating_supply, tx_count, utxos_spendable, \
            utxos_unspendable, blockchain_size \
     FROM block_metrics WHERE height = $1",
  )
  .bind(prev)
  .fetch_optional(conn)
  .await
  .map_err(|e| DaemonError::Pg(format!("prev metrics: {e}")))?;

  match row {
    Some(r) => Ok(r),
    None => Err(DaemonError::Index(format!(
      "block_metrics gap: no row for height {prev}"
    ))),
  }
}

#[expect(clippy::too_many_arguments)]
/// Upsert one block metrics snapshot.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn insert_block_metrics(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
  tx_count: i64,
  utxos_spendable: i64,
  utxos_unspendable: i64,
  blockchain_size: i64,
  credit_pool: Option<i64>,
  circulating_supply: i64,
  difficulty: f64,
  hashrate: f64,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO block_metrics \
       (height, tx_count, utxos_spendable, utxos_unspendable, \
        blockchain_size, credit_pool_balance, circulating_supply, \
        difficulty, hashrate) \
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9) \
     ON CONFLICT (height) DO UPDATE SET
       tx_count = EXCLUDED.tx_count,
       utxos_spendable = EXCLUDED.utxos_spendable,
       utxos_unspendable = EXCLUDED.utxos_unspendable,
       blockchain_size = EXCLUDED.blockchain_size,
       credit_pool_balance = EXCLUDED.credit_pool_balance,
       circulating_supply = EXCLUDED.circulating_supply,
       difficulty = EXCLUDED.difficulty,
       hashrate = EXCLUDED.hashrate",
  )
  .bind(height as i32)
  .bind(tx_count)
  .bind(utxos_spendable)
  .bind(utxos_unspendable)
  .bind(blockchain_size)
  .bind(credit_pool)
  .bind(circulating_supply)
  .bind(difficulty)
  .bind(hashrate)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("insert metrics {height}: {e}")))?;
  Ok(())
}

/// Read the most recent metrics snapshot.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn latest_metrics(pool: &PgPool) -> Result<Option<MetricsRow>, DaemonError> {
  let row: Option<(i32, i64, i64, i64, i64, Option<i64>, i64, f64, f64)> = sqlx::query_as(
    "SELECT height, tx_count, utxos_spendable, utxos_unspendable,
            blockchain_size, credit_pool_balance, circulating_supply,
            difficulty, hashrate
       FROM block_metrics ORDER BY height DESC LIMIT 1",
  )
  .fetch_optional(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read latest metrics: {e}")))?;

  Ok(row.map(metrics_row_from_tuple))
}

/// Read all metrics snapshots within the inclusive height range.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn metrics_range(
  pool: &PgPool,
  start_height: i32,
  end_height: i32,
) -> Result<Vec<MetricsRow>, DaemonError> {
  let rows: Vec<(i32, i64, i64, i64, i64, Option<i64>, i64, f64, f64)> = sqlx::query_as(
    "SELECT height, tx_count, utxos_spendable, utxos_unspendable,
            blockchain_size, credit_pool_balance, circulating_supply,
            difficulty, hashrate
       FROM block_metrics
      WHERE height >= $1 AND height <= $2
      ORDER BY height ASC",
  )
  .bind(start_height)
  .bind(end_height)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read metrics range: {e}")))?;

  Ok(rows.into_iter().map(metrics_row_from_tuple).collect())
}

/// Read one metrics snapshot per chart bucket using the last height in each bucket.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub(crate) async fn metrics_bucketed_range(
  pool: &PgPool,
  start_height: i32,
  end_height: i32,
  bucket_blocks: i32,
) -> Result<Vec<MetricsRow>, DaemonError> {
  if bucket_blocks <= 1 {
    return metrics_range(pool, start_height, end_height).await;
  }

  let rows: Vec<(i32, i64, i64, i64, i64, Option<i64>, i64, f64, f64)> = sqlx::query_as(
    "WITH bucket_heights AS (
       SELECT LEAST(bucket_start + $3 - 1, $2) AS height
         FROM generate_series($1, $2, $3) AS bucket_start
     )
     SELECT m.height,
            m.tx_count,
            m.utxos_spendable,
            m.utxos_unspendable,
            m.blockchain_size,
            m.credit_pool_balance,
            m.circulating_supply,
            m.difficulty,
            m.hashrate
       FROM block_metrics AS m
       JOIN bucket_heights AS b ON b.height = m.height
      ORDER BY m.height ASC",
  )
  .bind(start_height)
  .bind(end_height)
  .bind(bucket_blocks)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read metrics bucketed range: {e}")))?;

  Ok(rows.into_iter().map(metrics_row_from_tuple).collect())
}

#[cfg(feature = "bench")]
/// Read bucketed metrics rows through the production chart SQL path for benchmarks.
pub async fn bench_metrics_bucketed_range(
  pool: &PgPool,
  start_height: i32,
  end_height: i32,
  bucket_blocks: i32,
) -> Result<Vec<MetricsRow>, DaemonError> {
  metrics_bucketed_range(pool, start_height, end_height, bucket_blocks).await
}

/// Delete the metrics snapshot stored for one block height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn delete_block_metrics(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM block_metrics WHERE height = $1")
    .bind(height as i32)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("delete metrics {height}: {e}")))?;
  Ok(())
}

fn metrics_row_from_tuple(
  (
    height,
    tx_count,
    utxos_spendable,
    utxos_unspendable,
    blockchain_size,
    credit_pool_balance,
    circulating_supply,
    difficulty,
    hashrate,
  ): (i32, i64, i64, i64, i64, Option<i64>, i64, f64, f64),
) -> MetricsRow {
  MetricsRow {
    height,
    tx_count,
    utxos_spendable,
    utxos_unspendable,
    blockchain_size,
    credit_pool_balance,
    circulating_supply,
    difficulty,
    hashrate,
  }
}

#[cfg(test)]
mod tests {
  use sqlx::PgPool;

  use super::{latest_metrics, metrics_bucketed_range, metrics_range, query_prev_metrics};

  /// Seed one metrics row because point and range reads operate entirely on `block_metrics`.
  async fn seed_metric_row(pool: &PgPool, height: i32) -> Result<(), crate::error::DaemonError> {
    sqlx::query(
      "INSERT INTO block_metrics (
         height,
         tx_count,
         utxos_spendable,
         utxos_unspendable,
         blockchain_size,
         credit_pool_balance,
         circulating_supply,
         difficulty,
         hashrate
       ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
       ON CONFLICT (height) DO UPDATE SET
         tx_count = EXCLUDED.tx_count,
         utxos_spendable = EXCLUDED.utxos_spendable,
         utxos_unspendable = EXCLUDED.utxos_unspendable,
         blockchain_size = EXCLUDED.blockchain_size,
         credit_pool_balance = EXCLUDED.credit_pool_balance,
         circulating_supply = EXCLUDED.circulating_supply,
         difficulty = EXCLUDED.difficulty,
         hashrate = EXCLUDED.hashrate",
    )
    .bind(height)
    .bind(i64::from(height))
    .bind(10_i64)
    .bind(5_i64)
    .bind(1_000_i64 + i64::from(height))
    .bind(Some(100_i64 + i64::from(height)))
    .bind(5_000_i64 + i64::from(height))
    .bind(1.0_f64 + f64::from(height))
    .bind(2.0_f64 + f64::from(height))
    .execute(pool)
    .await
    .map_err(|e| crate::error::DaemonError::Pg(format!("seed metrics row {height}: {e}")))?;
    Ok(())
  }

  #[tokio::test]
  async fn metrics_bucketed_range_reads_last_height_in_each_bucket_when_pg_is_configured(
  ) -> Result<(), crate::error::DaemonError> {
    let Some(pool) = crate::tests::pg_pool()
      .await
      .map_err(|e| crate::error::DaemonError::Pg(format!("connect metrics test pool: {e}")))?
    else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    for height in 10..=13 {
      seed_metric_row(&pool, height).await?;
    }

    let rows = metrics_bucketed_range(&pool, 10, 13, 2).await?;
    assert_eq!(
      rows.iter().map(|row| row.height).collect::<Vec<_>>(),
      vec![11, 13]
    );

    Ok(())
  }

  #[tokio::test]
  async fn latest_metrics_and_range_read_seeded_rows_when_pg_is_configured(
  ) -> Result<(), crate::error::DaemonError> {
    let Some(pool) = crate::tests::pg_pool()
      .await
      .map_err(|e| crate::error::DaemonError::Pg(format!("connect metrics test pool: {e}")))?
    else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    for height in 4..=6 {
      seed_metric_row(&pool, height).await?;
    }

    let latest = latest_metrics(&pool).await?;
    assert!(latest.is_some());
    assert_eq!(latest.expect("latest metrics should exist").height, 6);

    let rows = metrics_range(&pool, 4, 5).await?;
    assert_eq!(
      rows.iter().map(|row| row.height).collect::<Vec<_>>(),
      vec![4, 5]
    );

    Ok(())
  }

  #[tokio::test]
  async fn query_prev_metrics_reads_previous_height_when_pg_is_configured(
  ) -> Result<(), crate::error::DaemonError> {
    let Some(pool) = crate::tests::pg_pool()
      .await
      .map_err(|e| crate::error::DaemonError::Pg(format!("connect metrics test pool: {e}")))?
    else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;
    seed_metric_row(&pool, 2).await?;

    let mut conn = pool.acquire().await.map_err(|e| {
      crate::error::DaemonError::Pg(format!("acquire metrics test connection: {e}"))
    })?;
    let prev = query_prev_metrics(&mut conn, 3).await?;
    assert_eq!(prev.0, 5_002);
    assert_eq!(prev.1, 2);

    Ok(())
  }
}
