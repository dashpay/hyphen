//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Durable chain progress helpers.

use sqlx::{PgPool, Row};

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct ChainProgressRow {
  pub canonical_tip_height: i32,
  pub canonical_tip_hash: Option<Vec<u8>>,
  pub projection_tip_height: i32,
  pub undo_tip_height: i32,
  pub recovery_state: String,
  pub recovery_height: Option<i32>,
  pub updated_at: i64,
}

async fn current_block_tip(pool: &PgPool) -> Result<(i32, Option<Vec<u8>>), DaemonError> {
  let row = sqlx::query(
    "SELECT COALESCE(MAX(height), -1) AS height,
            (
              SELECT hash
                FROM blocks
               ORDER BY height DESC
               LIMIT 1
            ) AS hash
       FROM blocks",
  )
  .fetch_one(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read current block tip: {e}")))?;
  Ok((row.get("height"), row.get("hash")))
}

async fn current_undo_tip(pool: &PgPool) -> Result<i32, DaemonError> {
  sqlx::query_scalar("SELECT COALESCE(MAX(height), -1) FROM block_undo_meta")
    .fetch_one(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read current undo tip: {e}")))
}

pub async fn ensure_chain_progress(pool: &PgPool) -> Result<(), DaemonError> {
  let (tip_height, tip_hash) = current_block_tip(pool).await?;
  let undo_tip = current_undo_tip(pool).await?;
  let projection_tip =
    crate::sql::query::indexed_stage_height(pool, crate::index::Stage::Block).await?;
  sqlx::query(
    "INSERT INTO chain_progress (
       singleton,
       canonical_tip_height,
       canonical_tip_hash,
       projection_tip_height,
       undo_tip_height,
       recovery_state,
       recovery_height,
       updated_at
     )
     VALUES (TRUE, $1, $2, $3, $4, 'clean', NULL, 0)
     ON CONFLICT (singleton) DO NOTHING",
  )
  .bind(tip_height)
  .bind(tip_hash)
  .bind(projection_tip)
  .bind(undo_tip)
  .execute(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("ensure chain_progress: {e}")))?;
  Ok(())
}

pub async fn chain_progress(pool: &PgPool) -> Result<ChainProgressRow, DaemonError> {
  sqlx::query_as::<_, ChainProgressRow>(
    "SELECT canonical_tip_height,
            canonical_tip_hash,
            projection_tip_height,
            undo_tip_height,
            recovery_state,
            recovery_height,
            updated_at
       FROM chain_progress
      WHERE singleton = TRUE",
  )
  .fetch_one(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read chain_progress: {e}")))
}

/// Count undo rows covering canonical heights from genesis through `height`.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn count_block_undo_rows_through(pool: &PgPool, height: i32) -> Result<i64, DaemonError> {
  sqlx::query_scalar::<_, i64>(
    "SELECT COUNT(*)::bigint
       FROM block_undo_meta
      WHERE height >= 0
        AND height <= $1",
  )
  .bind(height)
  .fetch_one(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("count block_undo_meta rows through {height}: {e}")))
}

pub async fn sync_chain_progress_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
  hash: Option<&[u8]>,
  updated_at: i64,
) -> Result<(), DaemonError> {
  sync_chain_progress_state_tx(tx, height, hash, updated_at, "clean", None).await
}

pub async fn sync_chain_progress_state_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
  hash: Option<&[u8]>,
  updated_at: i64,
  recovery_state: &str,
  recovery_height: Option<i32>,
) -> Result<(), DaemonError> {
  sqlx::query(
    "UPDATE chain_progress
        SET canonical_tip_height = $1,
            canonical_tip_hash = $2,
            projection_tip_height = $1,
            undo_tip_height = GREATEST(undo_tip_height, $1),
            recovery_state = $3,
            recovery_height = $4,
            updated_at = $5
      WHERE singleton = TRUE",
  )
  .bind(height)
  .bind(hash)
  .bind(recovery_state)
  .bind(recovery_height)
  .bind(updated_at)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("sync chain_progress: {e}")))?;
  Ok(())
}

pub async fn mark_recovery_state(
  pool: &PgPool,
  recovery_state: &str,
  recovery_height: Option<i32>,
) -> Result<(), DaemonError> {
  sqlx::query(
    "UPDATE chain_progress
        SET recovery_state = $1,
            recovery_height = $2
      WHERE singleton = TRUE",
  )
  .bind(recovery_state)
  .bind(recovery_height)
  .execute(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("mark chain_progress recovery_state: {e}")))?;
  Ok(())
}
