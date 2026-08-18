//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Validator registry accumulation, as an [`IngestStage`]: applying a block's
//! delta-shaped lifecycle events, reaping validators whose collateral was spent,
//! recomputing counts, and the reorg inverse (restore from `undo_validators`).

use async_trait::async_trait;

use super::pipeline::IngestStage;
use super::scratch::{HyDbStageError, Scratch};
use crate::error::{HyDbError, HyDbResult};
use crate::records::{HyDbBlock, PgTx};
use crate::types::{HyDbHash, HyDbOutpoint, HyDbVlType};

/// The validator-registry accumulation stage.
pub(super) struct ValidatorsStage;

#[async_trait]
impl IngestStage for ValidatorsStage {
  const NAME: &'static str = "validators";

  async fn connect(&self, tx: &mut PgTx<'_>, scratch: &Scratch) -> Result<(), HyDbStageError> {
    let height = scratch.height;
    let block = scratch.block(Self::NAME)?;
    let spent_outpoints = scratch.spent_outpoints(Self::NAME)?;

    // Apply this block's lifecycle events and reap any validator whose
    // collateral outpoint was spent. The counts rebuild is skipped entirely
    // for the overwhelming majority of blocks that touch no validator.
    let events_applied = Self::apply_events(tx, height, block)
      .await
      .map_err(HyDbStageError::wrap(Self::NAME))?;
    let reaped = Self::reap_collaterals(tx, height, spent_outpoints)
      .await
      .map_err(HyDbStageError::wrap(Self::NAME))?;
    if events_applied > 0 || reaped > 0 {
      Self::refresh_counts(tx)
        .await
        .map_err(HyDbStageError::wrap(Self::NAME))?;
    }
    Ok(())
  }

  async fn disconnect(&self, tx: &mut PgTx<'_>, height: i32) -> Result<(), HyDbStageError> {
    let restored = Self::restore(tx, height)
      .await
      .map_err(HyDbStageError::wrap(Self::NAME))?;
    if restored > 0 {
      Self::refresh_counts(tx)
        .await
        .map_err(HyDbStageError::wrap(Self::NAME))?;
    }
    sqlx::query!("DELETE FROM undo_validators WHERE height = $1", height)
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(
        Self::NAME,
        "disconnect: delete validator undo",
      ))?;
    Ok(())
  }
}

impl ValidatorsStage {
  /// Apply each transaction's validator lifecycle events onto the `validators`
  /// registry. Register creates a row; the update variants patch only their
  /// changed columns; revoke marks the operator revoked. Every touched validator's
  /// pre-image is captured into `undo_validators` first.
  async fn apply_events(tx: &mut PgTx<'_>, height: i32, block: &HyDbBlock) -> HyDbResult<usize> {
    let mut applied = 0usize;
    for btx in &block.txs {
      for event in &btx.validator_events {
        // Capture the pre-image for reorg undo, then apply the delta (the event
        // owns its own registry write).
        Self::snapshot(tx, height, event.protx_hash()).await?;
        event.apply(tx, height).await?;
        applied += 1;
      }
    }
    Ok(applied)
  }

  /// Capture one validator's current registry row into `undo_validators` (or an
  /// `existed = false` marker when it is not registered). Keeps the earliest
  /// pre-image for the block via `ON CONFLICT DO NOTHING`.
  async fn snapshot(tx: &mut PgTx<'_>, height: i32, protx_hash: &HyDbHash) -> HyDbResult<()> {
    let cur = sqlx::query!(
      r#"SELECT collateral_hash, collateral_index, collateral_address, owner_address,
                operator_pubkey, voting_address, addr_core_p2p, addr_platform_p2p,
                addr_platform_https, payout_address, operator_payout_address, operator_reward,
                validator_type AS "validator_type: HyDbVlType", status,
                registered_height, last_updated_height
           FROM validators WHERE protx_hash = $1"#,
      &protx_hash.0[..]
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("validator: snapshot read", e))?;

    sqlx::query!(
      r#"INSERT INTO undo_validators
           (height, protx_hash, existed, collateral_hash, collateral_index, collateral_address,
            owner_address, operator_pubkey, voting_address, addr_core_p2p, addr_platform_p2p,
            addr_platform_https, payout_address, operator_payout_address, operator_reward,
            validator_type, status, registered_height, last_updated_height)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17,$18,$19)
         ON CONFLICT (height, protx_hash) DO NOTHING"#,
      height,
      &protx_hash.0[..],
      cur.is_some(),
      cur.as_ref().map(|c| c.collateral_hash.clone()),
      cur.as_ref().map(|c| c.collateral_index),
      cur.as_ref().and_then(|c| c.collateral_address.clone()),
      cur.as_ref().and_then(|c| c.owner_address.clone()),
      cur.as_ref().and_then(|c| c.operator_pubkey.clone()),
      cur.as_ref().and_then(|c| c.voting_address.clone()),
      cur.as_ref().and_then(|c| c.addr_core_p2p.as_deref()),
      cur.as_ref().and_then(|c| c.addr_platform_p2p.as_deref()),
      cur.as_ref().and_then(|c| c.addr_platform_https.as_deref()),
      cur.as_ref().and_then(|c| c.payout_address.clone()),
      cur.as_ref().and_then(|c| c.operator_payout_address.clone()),
      cur.as_ref().map(|c| c.operator_reward),
      cur.as_ref().map(|c| c.validator_type) as _,
      cur.as_ref().map(|c| c.status.clone()),
      cur.as_ref().map(|c| c.registered_height),
      cur.as_ref().map(|c| c.last_updated_height),
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("validator: snapshot write", e))?;
    Ok(())
  }

  /// Deregister validators whose collateral outpoint was spent this block.
  /// One batched probe over every spent outpoint (typically finding nothing),
  /// then per-hit snapshot + one batched delete.
  async fn reap_collaterals(
    tx: &mut PgTx<'_>,
    height: i32,
    spent_outpoints: &[HyDbOutpoint],
  ) -> HyDbResult<usize> {
    if spent_outpoints.is_empty() {
      return Ok(0);
    }
    let hashes: Vec<Vec<u8>> = spent_outpoints.iter().map(|o| o.txid.0.to_vec()).collect();
    let indices: Vec<i32> = spent_outpoints.iter().map(|o| o.vout as i32).collect();

    let hit_rows = sqlx::query_scalar!(
      r#"SELECT v.protx_hash AS "protx_hash!"
           FROM validators v
           JOIN UNNEST($1::bytea[], $2::int4[]) AS s(collateral_hash, collateral_index)
             ON v.collateral_hash = s.collateral_hash
            AND v.collateral_index = s.collateral_index"#,
      &hashes,
      &indices,
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("validator: find reaped collateral", e))?;
    if hit_rows.is_empty() {
      return Ok(0);
    }

    for raw_hash in &hit_rows {
      let protx_hash = HyDbHash::from_slice(raw_hash)?;
      Self::snapshot(tx, height, &protx_hash).await?;
    }
    sqlx::query!(
      "DELETE FROM validators WHERE protx_hash = ANY($1)",
      &hit_rows
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("validator: reap collateral", e))?;
    Ok(hit_rows.len())
  }

  /// Recompute `validator_counts` from the registry (small table).
  async fn refresh_counts(tx: &mut PgTx<'_>) -> HyDbResult<()> {
    sqlx::query!("DELETE FROM validator_counts")
      .execute(&mut **tx)
      .await
      .map_err(|e| HyDbError::pg("validator: clear counts", e))?;
    sqlx::query!(
      r#"INSERT INTO validator_counts (status, validator_type, total)
         SELECT status, validator_type, COUNT(*)::bigint FROM validators
          GROUP BY status, validator_type"#
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("validator: recompute counts", e))?;
    Ok(())
  }

  /// Restore the validator registry to its pre-block state from
  /// `undo_validators`, returning how many rows were replayed.
  async fn restore(tx: &mut PgTx<'_>, height: i32) -> HyDbResult<usize> {
    let undos = sqlx::query!(
      r#"SELECT protx_hash, existed, collateral_hash, collateral_index, collateral_address,
                owner_address, operator_pubkey, voting_address, addr_core_p2p, addr_platform_p2p,
                addr_platform_https, payout_address, operator_payout_address, operator_reward,
                validator_type AS "validator_type: HyDbVlType", status,
                registered_height, last_updated_height
           FROM undo_validators WHERE height = $1"#,
      height
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("validator: read undo", e))?;

    for u in &undos {
      if !u.existed {
        // The validator was created this block — drop it.
        sqlx::query!(
          "DELETE FROM validators WHERE protx_hash = $1",
          &u.protx_hash
        )
        .execute(&mut **tx)
        .await
        .map_err(|e| HyDbError::pg("validator: undo delete", e))?;
        continue;
      }
      // Restore the captured pre-image (all columns are non-null when existed).
      sqlx::query!(
        r#"INSERT INTO validators
             (protx_hash, collateral_hash, collateral_index, collateral_address, owner_address,
              operator_pubkey, voting_address, addr_core_p2p, addr_platform_p2p, addr_platform_https,
              payout_address, operator_payout_address, operator_reward, validator_type, status,
              registered_height, last_updated_height)
           VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
           ON CONFLICT (protx_hash) DO UPDATE SET
             collateral_hash = EXCLUDED.collateral_hash,
             collateral_index = EXCLUDED.collateral_index,
             collateral_address = EXCLUDED.collateral_address,
             owner_address = EXCLUDED.owner_address,
             operator_pubkey = EXCLUDED.operator_pubkey,
             voting_address = EXCLUDED.voting_address,
             addr_core_p2p = EXCLUDED.addr_core_p2p,
             addr_platform_p2p = EXCLUDED.addr_platform_p2p,
             addr_platform_https = EXCLUDED.addr_platform_https,
             payout_address = EXCLUDED.payout_address,
             operator_payout_address = EXCLUDED.operator_payout_address,
             operator_reward = EXCLUDED.operator_reward,
             validator_type = EXCLUDED.validator_type,
             status = EXCLUDED.status,
             registered_height = EXCLUDED.registered_height,
             last_updated_height = EXCLUDED.last_updated_height"#,
        &u.protx_hash,
        u.collateral_hash.as_deref(),
        u.collateral_index,
        u.collateral_address.as_deref(),
        u.owner_address.as_deref(),
        u.operator_pubkey.as_deref(),
        u.voting_address.as_deref(),
        u.addr_core_p2p.as_deref(),
        u.addr_platform_p2p.as_deref(),
        u.addr_platform_https.as_deref(),
        u.payout_address.as_deref(),
        u.operator_payout_address.as_deref(),
        u.operator_reward,
        u.validator_type as _,
        u.status.as_deref(),
        u.registered_height,
        u.last_updated_height,
      )
      .execute(&mut **tx)
      .await
      .map_err(|e| HyDbError::pg("validator: undo restore", e))?;
    }
    Ok(undos.len())
  }
}
