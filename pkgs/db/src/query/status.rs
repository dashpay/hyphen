//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The status domain: health, on-disk size diagnostics, and the derived chain
//! tip. Progress is *derived from the data* (the canonical tip is the max
//! height in `blocks`) rather than read from a stored cursor.

use async_trait::async_trait;

use super::PgRead;
use crate::error::{HyDbError, HyDbResult};
use crate::types::{HyDbHash, HyDbHeight};

/// The canonical chain tip as recorded in the projection.
#[derive(Debug, Clone, Copy)]
pub struct HyDbQrChainTip {
  pub height: HyDbHeight,
  pub hash: HyDbHash,
}

/// Overall storage health.
#[derive(Debug, Clone)]
pub struct HyDbQrHealth {
  /// The derived canonical tip, or `None` on an empty database.
  pub tip: Option<HyDbQrChainTip>,
  /// Whether the schema is at the latest known migration version.
  pub schema_current: bool,
  /// Free-form notes (partition gaps, lag, etc.).
  pub notes: Vec<String>,
}

/// On-disk size of one relation.
#[derive(Debug, Clone)]
pub struct HyDbQrSizeDiag {
  pub relation: String,
  pub bytes: i64,
}

/// One pool's utilization, for operator diagnostics.
#[derive(Debug, Clone)]
pub struct HyDbQrPoolStat {
  pub name: String,
  pub size: u32,
  pub idle: usize,
}

/// One pool's per-session settings, as the *server* reports them.
///
/// Read back rather than echoed from the config on purpose: what an operator
/// needs to see is what the backends are actually running with. A `SET` that
/// silently failed to apply, or a value Postgres normalised differently from
/// what was written, is exactly the discrepancy this makes visible.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct HyDbQrPoolSession {
  /// Pool name, and the suffix on `application_name`.
  pub name: String,
  /// `application_name` as `pg_stat_activity` sees it.
  pub app_name: String,
  pub synchronous_commit: String,
  /// Milliseconds, in the GUC's base unit (`0` is unbounded).
  pub statement_timeout: String,
  pub work_mem: String,
  pub maintenance_work_mem: String,
  pub jit: String,
}

/// Server-level storage diagnostics for the operator console.
#[derive(Debug, Clone, Default)]
pub struct HyDbQrStorageDiag {
  pub server_version: String,
  pub database_bytes: i64,
  pub wal_bytes: i64,
  pub connections: i64,
  pub max_connections: i64,
  pub autovacuum_running: i64,
  pub shared_buffers: String,
  pub work_mem: String,
  pub effective_cache_size: String,
}

/// Query handle for status and diagnostics.
#[async_trait]
pub trait HyDbStatus: Send + Sync {
  /// Overall health, including the derived tip.
  async fn health(&self) -> HyDbResult<HyDbQrHealth>;

  /// The derived canonical tip (max height in `blocks`).
  async fn tip(&self) -> HyDbResult<Option<HyDbQrChainTip>>;

  /// Per-relation on-disk sizes, largest first.
  async fn sizes(&self) -> HyDbResult<Vec<HyDbQrSizeDiag>>;

  /// Server-level storage diagnostics (version, sizes, connection counts).
  /// A mock facade has no server to report on.
  async fn storage_diag(&self) -> HyDbResult<HyDbQrStorageDiag> {
    Err(HyDbError::MockGap("status.storage_diag"))
  }

  /// The height span currently covered by the undo tables, or `None` when
  /// they are empty. The upper bound trails the canonical tip.
  async fn undo_span(&self) -> HyDbResult<Option<(HyDbHeight, HyDbHeight)>> {
    Ok(None)
  }
}

#[derive(Debug)]
pub(crate) struct PgStatus {
  pub(crate) pg: PgRead,
}

#[async_trait]
impl HyDbStatus for PgStatus {
  async fn health(&self) -> HyDbResult<HyDbQrHealth> {
    let tip = self.tip().await?;
    let mut notes = Vec::new();

    // Actually measure schema currency instead of asserting it: compare the
    // applied migration ledger against the versions embedded in this build.
    let applied: Option<i64> =
      sqlx::query_scalar(r#"SELECT MAX(version) FROM _sqlx_migrations WHERE success"#)
        .fetch_one(&self.pg.admin)
        .await
        .map_err(|e| HyDbError::pg("schema version", e))?;
    let latest = crate::migrate::latest_version();
    let schema_current = applied == Some(latest);
    if !schema_current {
      notes.push(format!(
        "schema at migration {} but this build embeds {latest}",
        applied.map_or_else(|| "<none>".to_owned(), |v| v.to_string())
      ));
    }
    if tip.is_none() {
      notes.push("projection is empty (no canonical blocks yet)".to_owned());
    }

    Ok(HyDbQrHealth {
      tip,
      schema_current,
      notes,
    })
  }

  async fn tip(&self) -> HyDbResult<Option<HyDbQrChainTip>> {
    let row = sqlx::query!("SELECT height, hash FROM blocks ORDER BY height DESC LIMIT 1")
      .fetch_optional(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("chain tip", e))?;
    row
      .map(|r| {
        Ok(HyDbQrChainTip {
          height: HyDbHeight::try_from_i32(r.height)?,
          hash: HyDbHash::from_slice(&r.hash)?,
        })
      })
      .transpose()
  }

  async fn storage_diag(&self) -> HyDbResult<HyDbQrStorageDiag> {
    let row = sqlx::query!(
      r#"SELECT current_setting('server_version') AS "server_version!",
                pg_database_size(current_database())::bigint AS "database_bytes!",
                pg_wal_lsn_diff(pg_current_wal_lsn(), '0/0')::bigint AS "wal_bytes!",
                (SELECT COUNT(*) FROM pg_stat_activity WHERE datname = current_database())::bigint
                  AS "connections!",
                current_setting('max_connections')::bigint AS "max_connections!",
                (SELECT COUNT(*) FROM pg_stat_activity
                  WHERE backend_type = 'autovacuum worker')::bigint AS "autovacuum_running!",
                current_setting('shared_buffers') AS "shared_buffers!",
                current_setting('work_mem') AS "work_mem!",
                current_setting('effective_cache_size') AS "effective_cache_size!""#
    )
    .fetch_one(&self.pg.admin)
    .await
    .map_err(|e| HyDbError::pg("storage diag", e))?;
    Ok(HyDbQrStorageDiag {
      server_version: row.server_version,
      database_bytes: row.database_bytes,
      wal_bytes: row.wal_bytes,
      connections: row.connections,
      max_connections: row.max_connections,
      autovacuum_running: row.autovacuum_running,
      shared_buffers: row.shared_buffers,
      work_mem: row.work_mem,
      effective_cache_size: row.effective_cache_size,
    })
  }

  async fn undo_span(&self) -> HyDbResult<Option<(HyDbHeight, HyDbHeight)>> {
    let row = sqlx::query!(r#"SELECT MIN(height) AS lo, MAX(height) AS hi FROM undo_blocks"#)
      .fetch_one(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("undo span", e))?;
    match (row.lo, row.hi) {
      (Some(lo), Some(hi)) => Ok(Some((
        HyDbHeight::try_from_i32(lo)?,
        HyDbHeight::try_from_i32(hi)?,
      ))),
      _ => Ok(None),
    }
  }

  async fn sizes(&self) -> HyDbResult<Vec<HyDbQrSizeDiag>> {
    let rows = sqlx::query!(
      r#"SELECT c.relname::text AS "relation!",
                pg_total_relation_size(c.oid)::bigint AS "bytes!"
           FROM pg_class c
           JOIN pg_namespace n ON n.oid = c.relnamespace
          WHERE n.nspname = 'public' AND c.relkind IN ('r', 'p')
          ORDER BY pg_total_relation_size(c.oid) DESC"#
    )
    .fetch_all(&self.pg.admin)
    .await
    .map_err(|e| HyDbError::pg("relation sizes", e))?;

    Ok(
      rows
        .into_iter()
        .map(|r| HyDbQrSizeDiag {
          relation: r.relation,
          bytes: r.bytes,
        })
        .collect(),
    )
  }
}
