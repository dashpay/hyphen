//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL helpers for current InstantSend tx-lock state.

use sqlx::PgPool;

use crate::error::DaemonError;

/// One currently known InstantSend tx-lock row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IslockCurrentRow {
  pub txid: Vec<u8>,
  /// Raw `pubrawtxlock` payload bytes.
  ///
  /// Dash publishes the locked transaction bytes on `rawtxlock`, not the
  /// signature-bearing `rawtxlocksig` payload. The historical column name is
  /// kept for schema compatibility.
  pub raw_islock: Vec<u8>,
  pub seen_at: i64,
}

/// Upsert one current InstantSend tx-lock row.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the write.
pub async fn upsert_current(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  row: &IslockCurrentRow,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO islock_current (txid, raw_islock, seen_at)
     VALUES ($1, $2, $3)
     ON CONFLICT (txid) DO UPDATE SET
       raw_islock = EXCLUDED.raw_islock,
       seen_at = EXCLUDED.seen_at",
  )
  .bind(&row.txid)
  .bind(&row.raw_islock)
  .bind(row.seen_at)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("upsert islock current: {e}")))?;
  Ok(())
}

/// Delete one current InstantSend tx-lock row by txid.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the delete.
pub async fn delete_current(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  txid: &[u8],
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM islock_current WHERE txid = $1")
    .bind(txid)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("delete islock current: {e}")))?;
  Ok(())
}

/// Read one current InstantSend tx-lock row by txid.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn current_by_txid(
  pool: &PgPool,
  txid: &[u8],
) -> Result<Option<IslockCurrentRow>, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query(
    "SELECT txid, raw_islock, seen_at
     FROM islock_current
     WHERE txid = $1",
  )
  .bind(txid)
  .fetch_optional(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read islock current: {e}")))?;

  Ok(row.map(|row| IslockCurrentRow {
    txid: row.get("txid"),
    raw_islock: row.get("raw_islock"),
    seen_at: row.get("seen_at"),
  }))
}
