//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL helpers for live network snapshot state.

use serde_json::Value;
use sqlx::PgPool;

use crate::error::DaemonError;
use crate::sql::pg_err;

/// Current network snapshot persisted in PostgreSQL.
#[derive(Debug, Clone, PartialEq)]
pub struct NetworkSnapshotRow {
  pub payload: Value,
  pub updated_at: i64,
}

/// Upsert the singleton network snapshot row.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the write.
pub async fn upsert_snapshot(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  snapshot: &NetworkSnapshotRow,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO network_snapshot (singleton_key, payload, updated_at)
     VALUES (TRUE, $1, $2)
     ON CONFLICT (singleton_key) DO UPDATE SET
       payload = EXCLUDED.payload,
       updated_at = EXCLUDED.updated_at",
  )
  .bind(&snapshot.payload)
  .bind(snapshot.updated_at)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("upsert network snapshot", e))?;
  Ok(())
}

/// Read the current network snapshot.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn snapshot(pool: &PgPool) -> Result<Option<NetworkSnapshotRow>, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query(
    "SELECT payload, updated_at
     FROM network_snapshot
     WHERE singleton_key = TRUE",
  )
  .fetch_optional(pool)
  .await
  .map_err(|e| pg_err("read network snapshot", e))?;

  Ok(row.map(|row| NetworkSnapshotRow {
    payload: row.get("payload"),
    updated_at: row.get("updated_at"),
  }))
}
