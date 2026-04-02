//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL helpers for the latest ChainLock snapshot.

use sqlx::PgPool;

use crate::error::DaemonError;

/// Latest known ChainLock snapshot stored in PostgreSQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainLockSnapshotRow {
  pub block_hash: Vec<u8>,
  pub block_height: Option<i32>,
  pub core_height: Option<i32>,
  pub raw_chainlock: Vec<u8>,
  pub seen_at: i64,
}

/// Replace the singleton ChainLock snapshot row.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the write.
pub async fn upsert_snapshot(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  row: &ChainLockSnapshotRow,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO chainlock_snapshot
       (singleton_key, block_hash, block_height, core_height, raw_chainlock, seen_at)
     VALUES (TRUE, $1, $2, $3, $4, $5)
     ON CONFLICT (singleton_key) DO UPDATE SET
       block_hash = EXCLUDED.block_hash,
       block_height = EXCLUDED.block_height,
       core_height = EXCLUDED.core_height,
       raw_chainlock = EXCLUDED.raw_chainlock,
       seen_at = EXCLUDED.seen_at",
  )
  .bind(&row.block_hash)
  .bind(row.block_height)
  .bind(row.core_height)
  .bind(&row.raw_chainlock)
  .bind(row.seen_at)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("upsert chainlock snapshot: {e}")))?;
  Ok(())
}

/// Read the current ChainLock snapshot.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn snapshot(pool: &PgPool) -> Result<Option<ChainLockSnapshotRow>, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query(
    "SELECT block_hash, block_height, core_height, raw_chainlock, seen_at
       FROM chainlock_snapshot
      WHERE singleton_key = TRUE",
  )
  .fetch_optional(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read chainlock snapshot: {e}")))?;

  Ok(row.map(|row| ChainLockSnapshotRow {
    block_hash: row.get("block_hash"),
    block_height: row.get("block_height"),
    core_height: row.get("core_height"),
    raw_chainlock: row.get("raw_chainlock"),
    seen_at: row.get("seen_at"),
  }))
}
