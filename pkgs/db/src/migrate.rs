//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Schema migration. The DDL under `sql/` is embedded at compile time and
//! applied on connect; sqlx tracks applied versions in `_sqlx_migrations`.

use sqlx::PgPool;

use crate::error::HyDbError;

/// The embedded migration set: one file, `sql/0001_schema.sql`, which states
/// the whole schema as an end state rather than a history.
static MIGRATOR: sqlx::migrate::Migrator = sqlx::migrate!("./sql");

/// The highest migration version embedded in this build. The health check
/// compares it with `_sqlx_migrations` to report schema currency.
pub(crate) fn latest_version() -> i64 {
  MIGRATOR.iter().map(|m| m.version).max().unwrap_or(0)
}

/// Apply any pending migrations. Idempotent: already-applied versions are
/// skipped. Runs against the write pool inside sqlx's own advisory lock, so it
/// is safe even if two processes start at once.
///
/// # Upgrading a pre-migration database
///
/// Migration `0001` creates every object unconditionally, so it can only run
/// against a database that has never held Hyphen state. A database created by
/// an older Hyphen (whose bootstrap SQL made tables without recording them in
/// `_sqlx_migrations`) is detected below and refused with an explicit message
/// rather than failing on the first duplicate object. There is deliberately
/// no automatic adoption path: the old and new schemas are NOT equivalent
/// (different tables, partitioning, and accumulations), so the supported
/// upgrade is an explicit reset — point the daemon at a fresh database (or
/// drop and recreate this one) and let it re-sync from the node.
pub(crate) async fn apply(pool: &PgPool) -> Result<(), HyDbError> {
  let preexisting: bool = sqlx::query_scalar(
    r#"SELECT to_regclass('public.blocks') IS NOT NULL
          AND to_regclass('public._sqlx_migrations') IS NULL"#,
  )
  .fetch_one(pool)
  .await
  .map_err(|e| HyDbError::Migrate(format!("preflight check failed: {e}")))?;
  if preexisting {
    return Err(HyDbError::Migrate(
      "this database holds a pre-migration Hyphen schema (tables exist but \
       _sqlx_migrations does not); the schemas are not compatible — point the \
       daemon at a fresh database and re-sync"
        .to_owned(),
    ));
  }
  MIGRATOR
    .run(pool)
    .await
    .map_err(|e| HyDbError::Migrate(e.to_string()))
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn migrator_embeds_at_least_the_base_schema() {
    assert!(MIGRATOR.iter().any(|m| m.version == 1));
  }
}
