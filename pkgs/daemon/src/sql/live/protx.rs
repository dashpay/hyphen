//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL helpers for the live validator `protx` snapshot used by overlays.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::error::DaemonError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatorProTxSummaryRow {
  pub total_nodes: i64,
  pub enabled_nodes: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorProTxAggregateRow {
  pub regular_total: i64,
  pub regular_status_buckets: HashMap<String, i64>,
  pub evonode_total: i64,
  pub evonode_status_buckets: HashMap<String, i64>,
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

/// Read aggregate counts for the current validator `protx` overlay snapshot.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn aggregates(pool: &PgPool) -> Result<ValidatorProTxAggregateRow, DaemonError> {
  use sqlx::Row;

  let rows = snapshot(pool).await?;
  let indexed_types = if rows.is_empty() {
    HashMap::new()
  } else {
    let pro_tx_hashes = rows
      .iter()
      .map(|row| row.pro_tx_hash.clone())
      .collect::<Vec<_>>();
    sqlx::query(
      "SELECT pro_tx_hash, validator_type
         FROM validators
        WHERE pro_tx_hash = ANY($1::bytea[])",
    )
    .bind(&pro_tx_hashes)
    .fetch_all(pool)
    .await
    .map_err(|e| {
      DaemonError::Pg(format!(
        "read indexed validator types for protx aggregate: {e}"
      ))
    })?
    .into_iter()
    .map(|row| {
      (
        row.get::<Vec<u8>, _>("pro_tx_hash"),
        row.get::<String, _>("validator_type"),
      )
    })
    .collect::<HashMap<_, _>>()
  };
  let mut regular_status_buckets = HashMap::new();
  let mut evonode_status_buckets = HashMap::new();
  let mut regular_total = 0i64;
  let mut evonode_total = 0i64;

  for row in &rows {
    let validator_type = indexed_types
      .get(&row.pro_tx_hash)
      .map(String::as_str)
      .or(row.validator_type.as_deref());
    match public_validator_type(validator_type) {
      "regular" => {
        regular_total += 1;
        if let Some(status) = row.current_status.as_deref() {
          *regular_status_buckets
            .entry(public_validator_status(status).to_owned())
            .or_insert(0) += 1;
        }
      }
      "evonode" => {
        evonode_total += 1;
        if let Some(status) = row.current_status.as_deref() {
          *evonode_status_buckets
            .entry(public_validator_status(status).to_owned())
            .or_insert(0) += 1;
        }
      }
      _ => {}
    }
  }

  Ok(ValidatorProTxAggregateRow {
    regular_total,
    regular_status_buckets,
    evonode_total,
    evonode_status_buckets,
  })
}

fn public_validator_type(value: Option<&str>) -> &'static str {
  let compact = value
    .unwrap_or_default()
    .chars()
    .filter(|ch| ch.is_ascii_alphanumeric())
    .flat_map(char::to_lowercase)
    .collect::<String>();
  match compact.as_str() {
    "regular" => "regular",
    "highperformance" | "evo" | "evonode" => "evonode",
    _ => "unknown",
  }
}

fn public_validator_status(status: &str) -> &'static str {
  match status {
    "valid" => "valid",
    "registered" => "registered",
    "revoked" => "revoked",
    "pose_banned" => "banned",
    _ => "unknown",
  }
}

fn is_missing_relation(error: &sqlx::Error) -> bool {
  error
    .as_database_error()
    .and_then(|db| db.code())
    .is_some_and(|code| code == "42P01")
}

#[cfg(test)]
mod tests {
  use super::{aggregates, replace_snapshot, ValidatorProTxSnapshotRow};

  #[tokio::test]
  async fn aggregates_prefer_indexed_validator_type_when_snapshot_disagrees(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut conn = pool.acquire().await?;
    crate::sql::index::validator::insert_validator(
      &mut conn,
      crate::tx::ValidatorInsert {
        pro_tx_hash: vec![0x11; 32],
        collateral_hash: vec![0x22; 32],
        collateral_index: 1,
        collateral_address: None,
        operator_pubkey: vec![0x33; 48],
        owner_address: None,
        voting_address: None,
        operator_reward: 0,
        validator_type: "Regular".to_owned(),
        payout_addr: None,
        operator_payout_addr: None,
        core_addrs: Vec::new(),
        plat_p2p_addrs: Vec::new(),
        plat_https_addrs: Vec::new(),
      },
      10,
      0,
    )
    .await?;
    drop(conn);

    let mut tx = pool.begin().await?;
    replace_snapshot(
      &mut tx,
      &[ValidatorProTxSnapshotRow {
        pro_tx_hash: vec![0x11; 32],
        validator_type: Some("Evo".to_owned()),
        current_status: Some("valid".to_owned()),
        last_paid_height: None,
        consecutive_payments: None,
        pose_penalty: None,
        pose_revived_height: None,
        pose_ban_height: None,
        revocation_reason: None,
        updated_at: 42,
      }],
    )
    .await?;
    tx.commit().await?;

    let row = aggregates(&pool).await?;
    assert_eq!(row.regular_total, 1);
    assert_eq!(row.evonode_total, 0);
    assert_eq!(row.regular_status_buckets.get("valid"), Some(&1));
    Ok(())
  }
}
