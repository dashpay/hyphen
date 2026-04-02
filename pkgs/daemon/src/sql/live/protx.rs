//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL helpers for the live validator `protx` snapshot used by overlays.

use sqlx::PgPool;

use crate::error::DaemonError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatorProTxSummaryRow {
  pub total_nodes: i64,
  pub enabled_nodes: i64,
}

/// One current validator `protx` overlay row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorProTxSnapshotRow {
  pub pro_tx_hash: Vec<u8>,
  pub validator_type: Option<String>,
  pub current_status: Option<String>,
  pub last_paid_height: Option<i32>,
  pub consecutive_payments: Option<i32>,
  pub pose_penalty: Option<i32>,
  pub pose_revived_height: Option<i32>,
  pub pose_ban_height: Option<i32>,
  pub revocation_reason: Option<i32>,
  pub updated_at: i64,
}

/// Replace the full validator `protx` overlay snapshot.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the refresh cycle.
pub async fn replace_snapshot(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  rows: &[ValidatorProTxSnapshotRow],
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM validator_protx_snapshot")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear validator protx snapshot: {e}")))?;

  if rows.is_empty() {
    return Ok(());
  }
  let pro_tx_hashes = rows
    .iter()
    .map(|row| row.pro_tx_hash.clone())
    .collect::<Vec<_>>();
  let validator_types = rows
    .iter()
    .map(|row| row.validator_type.clone())
    .collect::<Vec<_>>();
  let current_statuses = rows
    .iter()
    .map(|row| row.current_status.clone())
    .collect::<Vec<_>>();
  let last_paid_heights = rows
    .iter()
    .map(|row| row.last_paid_height)
    .collect::<Vec<_>>();
  let consecutive_payments = rows
    .iter()
    .map(|row| row.consecutive_payments)
    .collect::<Vec<_>>();
  let pose_penalties = rows.iter().map(|row| row.pose_penalty).collect::<Vec<_>>();
  let pose_revived_heights = rows
    .iter()
    .map(|row| row.pose_revived_height)
    .collect::<Vec<_>>();
  let pose_ban_heights = rows
    .iter()
    .map(|row| row.pose_ban_height)
    .collect::<Vec<_>>();
  let revocation_reasons = rows
    .iter()
    .map(|row| row.revocation_reason)
    .collect::<Vec<_>>();
  let updated_ats = rows.iter().map(|row| row.updated_at).collect::<Vec<_>>();
  sqlx::query(
    "INSERT INTO validator_protx_snapshot
       (pro_tx_hash, validator_type, current_status, last_paid_height, consecutive_payments,
        pose_penalty, pose_revived_height, pose_ban_height, revocation_reason, updated_at)
     SELECT * FROM unnest(
       $1::bytea[], $2::text[], $3::text[], $4::int[], $5::int[], $6::int[],
       $7::int[], $8::int[], $9::int[], $10::bigint[]
     )",
  )
  .bind(pro_tx_hashes)
  .bind(validator_types)
  .bind(current_statuses)
  .bind(last_paid_heights)
  .bind(consecutive_payments)
  .bind(pose_penalties)
  .bind(pose_revived_heights)
  .bind(pose_ban_heights)
  .bind(revocation_reasons)
  .bind(updated_ats)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("insert validator protx snapshot: {e}")))?;
  Ok(())
}

/// Read all current validator `protx` overlay rows.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn snapshot(pool: &PgPool) -> Result<Vec<ValidatorProTxSnapshotRow>, DaemonError> {
  use sqlx::Row;

  let rows = match sqlx::query(
    "SELECT pro_tx_hash, validator_type, current_status, last_paid_height, consecutive_payments,
            pose_penalty, pose_revived_height, pose_ban_height, revocation_reason, updated_at
       FROM validator_protx_snapshot
       ORDER BY pro_tx_hash",
  )
  .fetch_all(pool)
  .await
  {
    Ok(rows) => rows,
    Err(e) if is_missing_relation(&e) => return Ok(Vec::new()),
    Err(e) => {
      return Err(DaemonError::Pg(format!(
        "read validator protx snapshot: {e}"
      )));
    }
  };

  Ok(
    rows
      .into_iter()
      .map(|row| ValidatorProTxSnapshotRow {
        pro_tx_hash: row.get("pro_tx_hash"),
        validator_type: row.get("validator_type"),
        current_status: row.get("current_status"),
        last_paid_height: row.get("last_paid_height"),
        consecutive_payments: row.get("consecutive_payments"),
        pose_penalty: row.get("pose_penalty"),
        pose_revived_height: row.get("pose_revived_height"),
        pose_ban_height: row.get("pose_ban_height"),
        revocation_reason: row.get("revocation_reason"),
        updated_at: row.get("updated_at"),
      })
      .collect(),
  )
}

/// Read aggregate counts for the current validator `protx` overlay snapshot.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn summary(pool: &PgPool) -> Result<ValidatorProTxSummaryRow, DaemonError> {
  let row = match sqlx::query_as::<_, (i64, i64)>(
    "SELECT COUNT(*)::bigint AS total_nodes,
            COUNT(*) FILTER (WHERE current_status = 'valid')::bigint AS enabled_nodes
       FROM validator_protx_snapshot",
  )
  .fetch_optional(pool)
  .await
  {
    Ok(Some(row)) => row,
    Ok(None) => (0, 0),
    Err(e) if is_missing_relation(&e) => {
      return Ok(ValidatorProTxSummaryRow {
        total_nodes: 0,
        enabled_nodes: 0,
      })
    }
    Err(e) => {
      return Err(DaemonError::Pg(format!(
        "read validator protx summary: {e}"
      )));
    }
  };

  Ok(ValidatorProTxSummaryRow {
    total_nodes: row.0,
    enabled_nodes: row.1,
  })
}

fn is_missing_relation(error: &sqlx::Error) -> bool {
  error
    .as_database_error()
    .and_then(|db| db.code())
    .is_some_and(|code| code == "42P01")
}
