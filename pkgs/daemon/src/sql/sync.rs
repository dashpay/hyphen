//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! sync_state query helpers.

use sqlx::PgPool;

use crate::error::DaemonError;
use crate::index::Stage;

/// One `sync_state` row projected into a small transport-friendly struct.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SyncStateRow {
  pub stage: String,
  pub indexed_height: i32,
  pub updated_at: i64,
}

/// Read the `indexed_height` for a given stage from `sync_state`.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when the row is missing or PostgreSQL rejects
/// the query.
pub async fn indexed_height(pool: &PgPool, stage: &str) -> Result<i32, DaemonError> {
  sqlx::query_scalar("SELECT indexed_height FROM sync_state WHERE stage = $1")
    .bind(stage)
    .fetch_one(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read {stage} sync_state: {e}")))
}

/// Read the `indexed_height` for a typed stage from `sync_state`.
///
/// # Errors
///
/// Returns the same errors as [`indexed_height`].
pub async fn indexed_stage_height(pool: &PgPool, stage: Stage) -> Result<i32, DaemonError> {
  indexed_height(pool, stage.name()).await
}

/// Read the `indexed_height` for a typed stage from `sync_state` inside an
/// explicit SQL transaction.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when the row is missing or PostgreSQL rejects the
/// query.
pub async fn indexed_stage_height_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  stage: Stage,
) -> Result<i32, DaemonError> {
  sqlx::query_scalar("SELECT indexed_height FROM sync_state WHERE stage = $1")
    .bind(stage.name())
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("read {} sync_state in tx: {e}", stage.name())))
}

/// Read the durable height for a stage from its owning table.
///
/// # Errors
///
/// Returns `DaemonError::Index` when the stage does not expose a durable
/// height query, or `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn durable_stage_height(pool: &PgPool, stage: Stage) -> Result<i32, DaemonError> {
  let stage_name = stage.name();
  let query = stage.durable_height_query().ok_or_else(|| {
    DaemonError::Index(format!("startup clamp unsupported for stage {stage_name}"))
  })?;

  sqlx::query_scalar(query)
    .fetch_one(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read {stage_name} actual height: {e}")))
}

/// Update one stage height outside an explicit SQL transaction.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the update.
pub async fn update_indexed_stage_height(
  pool: &PgPool,
  stage: Stage,
  indexed_height: i32,
) -> Result<(), DaemonError> {
  sqlx::query("UPDATE sync_state SET indexed_height = $1 WHERE stage = $2")
    .bind(indexed_height)
    .bind(stage.name())
    .execute(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("update {} sync_state: {e}", stage.name())))?;
  Ok(())
}

/// Update one stage height inside an explicit SQL transaction.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the update.
pub async fn update_indexed_stage_height_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  stage: Stage,
  indexed_height: i32,
  updated_at: i64,
) -> Result<(), DaemonError> {
  sqlx::query("UPDATE sync_state SET indexed_height = $1, updated_at = $2 WHERE stage = $3")
    .bind(indexed_height)
    .bind(updated_at)
    .bind(stage.name())
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("update {} sync_state: {e}", stage.name())))?;
  Ok(())
}

/// Read the canonical block hash for one height inside an explicit SQL
/// transaction.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn block_hash_at_height_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<Option<Vec<u8>>, DaemonError> {
  sqlx::query_scalar::<_, Vec<u8>>("SELECT hash FROM blocks WHERE height = $1")
    .bind(height)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("read block hash at height {height} in tx: {e}")))
}

/// Relax commit durability for the current transaction only.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the session setting.
pub async fn set_local_async_commit(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), DaemonError> {
  sqlx::query("SET LOCAL synchronous_commit = off")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("SET sync_commit: {e}")))?;
  Ok(())
}

/// Return all `sync_state` rows ordered by stage name.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn sync_state_rows(pool: &PgPool) -> Result<Vec<SyncStateRow>, DaemonError> {
  use sqlx::Row;

  let rows = sqlx::query("SELECT stage, indexed_height, updated_at FROM sync_state ORDER BY stage")
    .fetch_all(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read sync_state rows: {e}")))?;

  Ok(
    rows
      .iter()
      .map(|row| SyncStateRow {
        stage: row.get("stage"),
        indexed_height: row.get("indexed_height"),
        updated_at: row.get("updated_at"),
      })
      .collect(),
  )
}

/// Count rows in a known application table.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn table_row_count(pool: &PgPool, table_name: &'static str) -> Result<i64, DaemonError> {
  sqlx::query_scalar(&format!("SELECT COUNT(*) FROM {table_name}"))
    .fetch_one(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("count {table_name}: {e}")))
}

#[cfg(test)]
mod tests {
  // Nontrivial SQL-backed tests live in `pkgs/daemon/tests`.
}
