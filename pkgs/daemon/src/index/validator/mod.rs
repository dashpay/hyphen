//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Validator index from ProTx payloads.

pub(crate) mod ipc;
pub(crate) mod overlay;
#[cfg(feature = "tests")]
pub mod rest;
#[cfg(not(feature = "tests"))]
pub(crate) mod rest;

use dash_primitives::tx::SpecialPayload;
use dash_primitives::types::tx::TxType;

use crate::error::DaemonError;
pub(crate) use crate::sql::index::validator as sql;
use crate::tx::ValidatorEffect;

use super::{BlockCtx, IndexOps};

#[derive(Debug)]
pub struct ValidatorIndex {
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
}

#[async_trait::async_trait]
impl IndexOps for ValidatorIndex {
  async fn disconnect_block(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    height: u32,
  ) -> Result<(), DaemonError> {
    sql::restore_history(tx, height).await?;
    sql::refresh_validator_counts_tx(tx).await
  }
}

impl ValidatorIndex {
  pub(crate) fn plan_connect(&self, block: &BlockCtx) -> Result<ValidatorBlockPlan, DaemonError> {
    ValidatorBlockPlan::build(block, self.p2pkh_version, self.p2sh_version)
  }

  pub(crate) async fn apply_plan(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    plan: &ValidatorBlockPlan,
  ) -> Result<(), DaemonError> {
    plan.apply(tx).await
  }
}

async fn apply_validator_effect(
  conn: &mut crate::sql::SqlConnection,
  effect: ValidatorEffect,
  height: u32,
  sequence: i32,
) -> Result<(), DaemonError> {
  let h = height as i32;
  match effect {
    ValidatorEffect::Insert(v) => sql::insert_validator(conn, v, h, sequence).await,
    ValidatorEffect::UpdateService {
      pro_tx_hash,
      core_addrs,
      plat_p2p_addrs,
      plat_https_addrs,
    } => {
      sql::update_service(
        conn,
        &pro_tx_hash,
        &core_addrs,
        &plat_p2p_addrs,
        &plat_https_addrs,
        h,
        sequence,
      )
      .await
    }
    ValidatorEffect::UpdateRegistrar {
      pro_tx_hash,
      operator_pubkey,
      payout_addr,
      voting_addr,
    } => {
      sql::update_registrar(
        conn,
        &pro_tx_hash,
        &operator_pubkey,
        &payout_addr,
        &voting_addr,
        h,
        sequence,
      )
      .await
    }
    ValidatorEffect::Revoke { pro_tx_hash } => sql::revoke(conn, &pro_tx_hash, h, sequence).await,
  }
}

pub(crate) struct ValidatorBlockPlan {
  height: u32,
  effects: Vec<ValidatorEffect>,
}

impl ValidatorBlockPlan {
  fn build(block: &BlockCtx, p2pkh_version: u8, p2sh_version: u8) -> Result<Self, DaemonError> {
    let mut effects = Vec::new();

    for (pos, transaction) in block.block().transactions.iter().enumerate() {
      if matches!(transaction.tx_type, TxType::Normal | TxType::Unknown(_)) {
        continue;
      }

      let txid = *block.txid(pos).ok_or_else(|| {
        DaemonError::Index(format!("tx {pos} missing txid at {}", block.height()))
      })?;

      match SpecialPayload::decode(transaction.tx_type, &transaction.extra_payload) {
        Ok(payload) => {
          if let Some(effect) =
            crate::tx::validator_effect(&payload, &txid, p2pkh_version, p2sh_version)
          {
            effects.push(effect);
          }
        }
        Err(e) => {
          tracing::warn!(
            height = block.height(),
            pos,
            "failed to decode special tx payload: {e}"
          );
        }
      }
    }

    Ok(Self {
      height: block.height(),
      effects,
    })
  }

  async fn apply(&self, conn: &mut crate::sql::SqlConnection) -> Result<(), DaemonError> {
    for (sequence, effect) in self.effects.iter().enumerate() {
      apply_validator_effect(conn, effect.clone(), self.height, sequence as i32).await?;
    }

    sql::refresh_validator_counts(conn).await?;
    Ok(())
  }
}
