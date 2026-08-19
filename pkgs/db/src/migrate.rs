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
///
/// # Upgrading across a rewritten migration set
///
/// A recorded history that disagrees with the embedded one is refused for the
/// same reason and separately, because it does not look pre-migration at all —
/// the table is present and populated. This is what a database built by a
/// generation whose schema was stated as a sequence of deltas looks like once
/// those are restated as one file: versions this build does not carry, or a
/// version whose checksum differs. Left to `MIGRATOR.run` it surfaces as a
/// checksum complaint with no indication of what to do about it.
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
  verify_history(pool).await?;
  MIGRATOR
    .run(pool)
    .await
    .map_err(|e| HyDbError::Migrate(e.to_string()))
}

/// Refuse a recorded migration history this build cannot continue.
///
/// # Errors
///
/// [`HyDbError::Migrate`] when `_sqlx_migrations` records a version this build
/// does not embed, or records one whose checksum differs from the embedded file,
/// or [`HyDbError::Pg`] if the history cannot be read.
async fn verify_history(pool: &PgPool) -> Result<(), HyDbError> {
  let tracked: bool =
    sqlx::query_scalar("SELECT to_regclass('public._sqlx_migrations') IS NOT NULL")
      .fetch_one(pool)
      .await
      .map_err(|e| HyDbError::Migrate(format!("preflight check failed: {e}")))?;
  if !tracked {
    return Ok(());
  }

  let applied: Vec<(i64, Vec<u8>)> =
    sqlx::query_as("SELECT version, checksum FROM _sqlx_migrations WHERE success ORDER BY version")
      .fetch_all(pool)
      .await
      .map_err(|e| HyDbError::Migrate(format!("reading applied migrations failed: {e}")))?;

  let mut unknown: Vec<i64> = Vec::new();
  let mut rewritten: Vec<i64> = Vec::new();
  for (version, checksum) in applied {
    match MIGRATOR.iter().find(|m| m.version == version) {
      None => unknown.push(version),
      Some(embedded) if embedded.checksum.as_ref() != checksum.as_slice() => {
        rewritten.push(version);
      }
      Some(_) => {}
    }
  }
  if unknown.is_empty() && rewritten.is_empty() {
    return Ok(());
  }

  let mut why: Vec<String> = Vec::new();
  if !unknown.is_empty() {
    why.push(format!(
      "it records migration(s) {} that this build does not carry",
      join_versions(&unknown)
    ));
  }
  if !rewritten.is_empty() {
    why.push(format!(
      "migration(s) {} were applied from different SQL than this build embeds",
      join_versions(&rewritten)
    ));
  }
  Err(HyDbError::Migrate(format!(
    "this database was built by a different schema generation: {}. the schemas are \
     not equivalent and there is no adoption path — point the daemon at a fresh \
     database and re-sync from the node",
    why.join(", and ")
  )))
}

fn join_versions(versions: &[i64]) -> String {
  versions
    .iter()
    .map(i64::to_string)
    .collect::<Vec<_>>()
    .join(", ")
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn migrator_embeds_at_least_the_base_schema() {
    assert!(MIGRATOR.iter().any(|m| m.version == 1));
  }
}
