//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! PostgreSQL diagnostic queries for heartbeat snapshots.

use sqlx::PgPool;

use hyphend::wire::msg::PostgresDiag;

/// Query Postgres version and tuning parameters once at startup.
pub async fn startup_diag(
  pool: &PgPool,
  write_pool_size: usize,
  query_pool_size: usize,
  admin_pool_size: usize,
) -> PostgresDiag {
  use sqlx::Row;

  let mut diag = PostgresDiag {
    write_pool_size: Some(write_pool_size),
    query_pool_size: Some(query_pool_size),
    admin_pool_size: Some(admin_pool_size),
    ..PostgresDiag::default()
  };

  if let Ok(row) = sqlx::query("SELECT version()").fetch_one(pool).await {
    diag.version = row.try_get::<String, _>("version").ok();
  }
  if let Ok(row) = sqlx::query("SHOW max_connections").fetch_one(pool).await {
    diag.max_connections = row
      .try_get::<String, _>("max_connections")
      .ok()
      .and_then(|s| s.parse().ok());
  }
  if let Ok(row) = sqlx::query("SHOW shared_buffers").fetch_one(pool).await {
    diag.shared_buffers = row.try_get::<String, _>("shared_buffers").ok();
  }
  if let Ok(row) = sqlx::query("SHOW work_mem").fetch_one(pool).await {
    diag.work_mem = row.try_get::<String, _>("work_mem").ok();
  }
  if let Ok(row) = sqlx::query("SHOW effective_cache_size")
    .fetch_one(pool)
    .await
  {
    diag.effective_cache_size = row.try_get::<String, _>("effective_cache_size").ok();
  }

  diag
}

/// Best-effort PostgreSQL size counters captured for heartbeat diagnostics.
#[derive(Clone, Debug, Default)]
pub struct PgSizeDiag {
  /// Total size of the current database, including tables and indexes.
  pub db_bytes: Option<u64>,
  /// Largest user tables by on-disk size.
  pub table_sizes: Vec<(String, u64)>,
  /// Total size of files currently present in the WAL directory.
  pub wal_bytes: Option<u64>,
}

/// Query the current database size for heartbeat diagnostics.
pub async fn database_size_diag(pool: &PgPool) -> Option<u64> {
  sqlx::query_scalar::<_, i64>("SELECT pg_database_size(current_database())")
    .fetch_one(pool)
    .await
    .ok()
    .and_then(|v| v.try_into().ok())
}

/// Query the largest user-table sizes for heartbeat diagnostics.
pub async fn table_size_diag(pool: &PgPool) -> Vec<(String, u64)> {
  use sqlx::Row;

  sqlx::query(
    "WITH candidate AS (
       SELECT c.oid AS relid, c.relname
         FROM pg_class AS c
         JOIN pg_namespace AS n ON n.oid = c.relnamespace
        WHERE c.relkind = 'r'
          AND n.nspname NOT IN ('pg_catalog', 'information_schema')
        ORDER BY c.relpages DESC, c.oid
        LIMIT 24
     )
     SELECT relname, pg_total_relation_size(relid)::bigint AS size_bytes
       FROM candidate
      ORDER BY size_bytes DESC
      LIMIT 12",
  )
  .fetch_all(pool)
  .await
  .unwrap_or_default()
  .into_iter()
  .filter_map(|r| {
    let name: String = r.get("relname");
    let bytes = r.try_get::<Option<i64>, _>("size_bytes").ok().flatten()?;
    bytes.try_into().ok().map(|b: u64| (name, b))
  })
  .collect()
}

/// Query the total WAL directory size for heartbeat diagnostics.
pub async fn wal_size_diag(pool: &PgPool) -> Option<u64> {
  sqlx::query_scalar::<_, i64>("SELECT coalesce(sum(size), 0)::bigint FROM pg_ls_waldir()")
    .fetch_one(pool)
    .await
    .ok()
    .and_then(|v| v.try_into().ok())
}

#[cfg(test)]
mod tests {
  #[tokio::test]
  async fn table_size_diag_returns_bounded_user_tables_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let rows = super::table_size_diag(&pool).await;
    assert!(rows.len() <= 12);
    assert!(rows.iter().all(|(name, _)| !name.starts_with("pg_")));
    assert!(rows.iter().all(|(name, _)| name != "information_schema"));

    Ok(())
  }
}
