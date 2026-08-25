//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Provider special-transaction records: the validator-registry transitions a
//! DIP3 ProReg/ProUpServ/ProUpReg/ProUpRev tx emits. [`HyDbVlEvent`] is a dumb
//! dispatcher over four first-class payloads, each of which owns the registry
//! write it performs.

use super::PgTx;
use crate::error::{HyDbError, HyDbResult};
use crate::types::{HyDbAddress, HyDbHash, HyDbVlType};

/// A validator lifecycle transition derived from a provider special
/// transaction.
///
/// Each variant wraps a named payload struct (one per DIP3 provider tx) that
/// owns both its fields and the write it performs. The enum is a dumb
/// dispatcher — no variant is subordinate to another: `HyDbVlRegister` happens
/// to be a full row and the rest are field deltas, but all four are first-class
/// transitions that apply themselves. Collateral reap is handled crate-side
/// during spend resolution (a spent collateral outpoint deregisters its
/// validator), not here.
#[derive(Debug, Clone)]
pub enum HyDbVlEvent {
  /// ProRegTx — a full new registry row.
  Register(HyDbVlRegister),
  /// ProUpServTx — service/connectivity fields only.
  UpdateService(HyDbVlUpdService),
  /// ProUpRegTx — operator key, payout, and voting fields only.
  UpdateRegistrar(HyDbVlUpdRegistrar),
  /// ProUpRevTx — operator revocation (status transitions to `revoked`).
  Revoke(HyDbVlRevoke),
}

impl HyDbVlEvent {
  /// The validator this event transitions.
  pub fn protx_hash(&self) -> &HyDbHash {
    match self {
      Self::Register(e) => &e.protx_hash,
      Self::UpdateService(e) => &e.protx_hash,
      Self::UpdateRegistrar(e) => &e.protx_hash,
      Self::Revoke(e) => &e.protx_hash,
    }
  }

  /// Apply this transition onto the `validators` registry at `height` by
  /// dispatching to the payload's own write. The caller captures the pre-image
  /// into `undo_validators` first (reorg replay reverses this).
  pub(crate) async fn apply(&self, tx: &mut PgTx<'_>, height: i32) -> HyDbResult<()> {
    match self {
      Self::Register(e) => e.apply(tx, height).await,
      Self::UpdateService(e) => e.apply(tx, height).await,
      Self::UpdateRegistrar(e) => e.apply(tx, height).await,
      Self::Revoke(e) => e.apply(tx, height).await,
    }
  }
}

/// The full registry row a ProRegTx creates.
#[derive(Debug, Clone)]
pub struct HyDbVlRegister {
  pub protx_hash: HyDbHash,
  pub collateral_hash: HyDbHash,
  pub collateral_index: i32,
  pub collateral_address: Option<HyDbAddress>,
  pub owner_address: Option<HyDbAddress>,
  pub operator_pubkey: Option<Vec<u8>>,
  pub voting_address: Option<HyDbAddress>,
  pub addr_core_p2p: Vec<String>,
  pub addr_platform_p2p: Vec<String>,
  pub addr_platform_https: Vec<String>,
  pub payout_address: Option<HyDbAddress>,
  pub operator_payout_address: Option<HyDbAddress>,
  pub operator_reward: i32,
  pub validator_type: HyDbVlType,
}

impl HyDbVlRegister {
  /// Upsert the full registry row, registering the validator (or replacing a
  /// prior row for the same ProTx on a reorg replay).
  pub(crate) async fn apply(&self, tx: &mut PgTx<'_>, height: i32) -> HyDbResult<()> {
    sqlx::query!(
      r#"INSERT INTO validators
           (protx_hash, collateral_hash, collateral_index, collateral_address, owner_address,
            operator_pubkey, voting_address, addr_core_p2p, addr_platform_p2p,
            addr_platform_https, payout_address, operator_payout_address, operator_reward,
            validator_type, status, registered_height, last_updated_height)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,'registered',$15,$15)
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
           last_updated_height = EXCLUDED.last_updated_height"#,
      &self.protx_hash.0[..],
      &self.collateral_hash.0[..],
      self.collateral_index,
      self.collateral_address.as_ref().map(|a| a.0.as_str()),
      self.owner_address.as_ref().map(|a| a.0.as_str()),
      self.operator_pubkey.as_deref(),
      self.voting_address.as_ref().map(|a| a.0.as_str()),
      &self.addr_core_p2p,
      &self.addr_platform_p2p,
      &self.addr_platform_https,
      self.payout_address.as_ref().map(|a| a.0.as_str()),
      self.operator_payout_address.as_ref().map(|a| a.0.as_str()),
      self.operator_reward,
      self.validator_type as _,
      height,
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("validator: register", e))?;
    Ok(())
  }
}

/// A ProUpServTx delta: service/connectivity fields only.
#[derive(Debug, Clone)]
pub struct HyDbVlUpdService {
  pub protx_hash: HyDbHash,
  pub addr_core_p2p: Vec<String>,
  pub addr_platform_p2p: Vec<String>,
  pub addr_platform_https: Vec<String>,
  pub operator_payout_address: Option<HyDbAddress>,
}

impl HyDbVlUpdService {
  /// Patch the service/connectivity columns of the existing registry row.
  pub(crate) async fn apply(&self, tx: &mut PgTx<'_>, height: i32) -> HyDbResult<()> {
    let res = sqlx::query!(
      r#"UPDATE validators SET addr_core_p2p = $2, addr_platform_p2p = $3,
           addr_platform_https = $4, operator_payout_address = $5, last_updated_height = $6
         WHERE protx_hash = $1"#,
      &self.protx_hash.0[..],
      &self.addr_core_p2p,
      &self.addr_platform_p2p,
      &self.addr_platform_https,
      self.operator_payout_address.as_ref().map(|a| a.0.as_str()),
      height,
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("validator: update service", e))?;
    require_registered(res.rows_affected(), "ProUpServTx", &self.protx_hash, height)
  }
}

/// A ProUpRegTx delta: operator key, payout, and voting fields only.
#[derive(Debug, Clone)]
pub struct HyDbVlUpdRegistrar {
  pub protx_hash: HyDbHash,
  pub operator_pubkey: Vec<u8>,
  pub payout_address: Option<HyDbAddress>,
  pub voting_address: Option<HyDbAddress>,
}

impl HyDbVlUpdRegistrar {
  /// Patch the operator key, payout, and voting columns of the registry row.
  pub(crate) async fn apply(&self, tx: &mut PgTx<'_>, height: i32) -> HyDbResult<()> {
    let res = sqlx::query!(
      r#"UPDATE validators SET operator_pubkey = $2, payout_address = $3,
           voting_address = $4, last_updated_height = $5 WHERE protx_hash = $1"#,
      &self.protx_hash.0[..],
      &self.operator_pubkey[..],
      self.payout_address.as_ref().map(|a| a.0.as_str()),
      self.voting_address.as_ref().map(|a| a.0.as_str()),
      height,
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("validator: update registrar", e))?;
    require_registered(res.rows_affected(), "ProUpRegTx", &self.protx_hash, height)
  }
}

/// A ProUpRevTx: operator revocation (status transitions to `revoked`).
#[derive(Debug, Clone)]
pub struct HyDbVlRevoke {
  pub protx_hash: HyDbHash,
}

impl HyDbVlRevoke {
  /// Flip the validator's status to `revoked`.
  pub(crate) async fn apply(&self, tx: &mut PgTx<'_>, height: i32) -> HyDbResult<()> {
    let res = sqlx::query!(
      r#"UPDATE validators SET status = 'revoked', last_updated_height = $2
         WHERE protx_hash = $1"#,
      &self.protx_hash.0[..],
      height,
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("validator: revoke", e))?;
    require_registered(res.rows_affected(), "ProUpRevTx", &self.protx_hash, height)
  }
}

/// An update/revoke for a `protx_hash` not in the registry means the registry
/// projection is inconsistent with the chain. Silently affecting zero rows
/// would drop the event forever; error instead so the block's transaction
/// rolls back and the fault is visible.
fn require_registered(
  rows_affected: u64,
  what: &str,
  protx_hash: &HyDbHash,
  height: i32,
) -> HyDbResult<()> {
  if rows_affected == 0 {
    return Err(HyDbError::Derive(format!(
      "{what} at height {height} targets unknown validator {protx_hash:?}"
    )));
  }
  Ok(())
}
