//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Schema bootstrap and additive refresh.

use sqlx::PgPool;

use crate::error::DaemonError;
use crate::sql::pg_err;

/// Apply or refresh the consolidated schema.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when schema inspection fails, when the target
/// database is not blank, or when applying the embedded schema fails.
pub async fn bootstrap(pool: &PgPool) -> Result<(), DaemonError> {
  let schema_present: bool = sqlx::query_scalar(
    "SELECT EXISTS (
       SELECT 1 FROM information_schema.tables
       WHERE table_schema = 'public' AND table_name = 'sync_state'
     )",
  )
  .fetch_one(pool)
  .await
  .map_err(|e| pg_err("check schema", e))?;

  if !schema_present {
    let public_table_count: i64 = sqlx::query_scalar(
      "SELECT COUNT(*)
         FROM information_schema.tables
        WHERE table_schema = 'public'
          AND table_type = 'BASE TABLE'",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| pg_err("check blank schema", e))?;

    if public_table_count != 0 {
      return Err(DaemonError::Pg {
        message: "refusing to apply schema to non-blank database without sync_state; recreate the database"
          .to_owned(),
        code: None,
      });
    }
  }

  if schema_present {
    tracing::info!("schema present, re-applying consolidated schema for additive refresh");
  } else {
    tracing::info!("applying consolidated schema");
  }

  sqlx::raw_sql(super::SCHEMA)
    .execute(pool)
    .await
    .map_err(|e| pg_err("apply schema", e))?;
  crate::sql::partition::ensure_core_history_partitions(pool).await?;
  crate::sql::partition::ensure_core_canonical_partitions(pool).await?;
  crate::sql::progress::ensure_chain_progress(pool).await?;

  if schema_present {
    tracing::info!("schema refresh applied");
  } else {
    tracing::info!("schema applied");
  }

  Ok(())
}
