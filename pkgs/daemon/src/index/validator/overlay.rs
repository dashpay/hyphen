//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Thin validator overlay backed by persisted live `protx` snapshots.
//!
//! Overlay modules enrich indexed responses with nullable live fields only.
//! They must not duplicate facts already exposed by the indexed payload.

use std::collections::HashMap;

use crate::sql::SqlPool;

use crate::error::DaemonError;
use crate::utils::{self, HashEndian};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorOverlay {
  pub current_type: Option<String>,
  pub current_status: Option<String>,
  pub last_paid_height: Option<i32>,
  pub consecutive_payments: Option<i32>,
  pub pose_penalty: Option<i32>,
  pub pose_revived_height: Option<i32>,
  pub pose_ban_height: Option<i32>,
  pub revocation_reason: Option<i32>,
}

pub async fn fetch_overlay_map(
  pool: &SqlPool,
) -> Result<HashMap<String, ValidatorOverlay>, DaemonError> {
  let rows = crate::sql::live::protx::snapshot(pool).await?;
  Ok(
    rows
      .into_iter()
      .map(|row| {
        (
          utils::view_hex(&row.pro_tx_hash, HashEndian::Ser),
          ValidatorOverlay {
            current_type: row.validator_type,
            current_status: row.current_status,
            last_paid_height: row.last_paid_height,
            consecutive_payments: row.consecutive_payments,
            pose_penalty: row.pose_penalty,
            pose_revived_height: row.pose_revived_height,
            pose_ban_height: row.pose_ban_height,
            revocation_reason: row.revocation_reason,
          },
        )
      })
      .collect(),
  )
}

#[cfg(test)]
mod tests {
  use super::ValidatorOverlay;
  use crate::utils::HashEndian;

  #[tokio::test]
  async fn fetch_overlay_map_reads_persisted_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut tx = pool.begin().await?;
    crate::sql::live::protx::replace_snapshot(
      &mut tx,
      &[crate::sql::live::protx::ValidatorProTxSnapshotRow {
        pro_tx_hash: vec![0x11; 32],
        validator_type: Some("Evo".to_owned()),
        current_status: Some("valid".to_owned()),
        last_paid_height: Some(42),
        consecutive_payments: None,
        pose_penalty: None,
        pose_revived_height: None,
        pose_ban_height: None,
        revocation_reason: None,
        updated_at: 100,
      }],
    )
    .await?;
    tx.commit().await?;

    let overlay = super::fetch_overlay_map(&pool).await?;
    assert_eq!(
      overlay.get(&"11".repeat(32)),
      Some(&ValidatorOverlay {
        current_type: Some("Evo".to_owned()),
        current_status: Some("valid".to_owned()),
        last_paid_height: Some(42),
        consecutive_payments: None,
        pose_penalty: None,
        pose_revived_height: None,
        pose_ban_height: None,
        revocation_reason: None,
      })
    );
    Ok(())
  }

  #[tokio::test]
  async fn fetch_overlay_map_matches_serialized_hash_rows_by_public_hex(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let public_hex = "00112233".repeat(8);
    let mut tx = pool.begin().await?;
    crate::sql::live::protx::replace_snapshot(
      &mut tx,
      &[crate::sql::live::protx::ValidatorProTxSnapshotRow {
        pro_tx_hash: crate::utils::decode_hex_to(&public_hex, HashEndian::Ser)?,
        validator_type: Some("Evo".to_owned()),
        current_status: Some("valid".to_owned()),
        last_paid_height: None,
        consecutive_payments: None,
        pose_penalty: None,
        pose_revived_height: None,
        pose_ban_height: None,
        revocation_reason: None,
        updated_at: 100,
      }],
    )
    .await?;
    tx.commit().await?;

    let overlay = super::fetch_overlay_map(&pool).await?;
    assert!(overlay.contains_key(&public_hex));
    Ok(())
  }
}
