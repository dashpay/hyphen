//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Dash transaction type modules -- per-type extraction and storage.
//!
//! Each submodule is the sole authority on:
//! - What data to store in `special_tx_payloads` (`to_json`)
//! - How to shape that stored JSONB for API responses (`to_api`)
//! - What side-effect the tx has on the `validators` table (`to_effect`)

pub mod asset_lock;
pub mod asset_unlock;
pub mod coinbase;
pub mod mnhf;
pub mod provider_register;
pub mod provider_update_registrar;
pub mod provider_update_revoke;
pub mod provider_update_service;
pub mod quorum;

use dash_primitives::script::{classify_script, ScriptKind};
use dash_primitives::tx::SpecialPayload;
use serde_json::Value;

/// Side-effect a provider-tx has on the `validators` table.
#[derive(Clone, Debug)]
pub enum ValidatorEffect {
  Insert(ValidatorInsert),
  UpdateService {
    pro_tx_hash: Vec<u8>,
    core_addrs: Vec<String>,
    plat_p2p_addrs: Vec<String>,
    plat_https_addrs: Vec<String>,
  },
  UpdateRegistrar {
    pro_tx_hash: Vec<u8>,
    operator_pubkey: Vec<u8>,
    payout_addr: Option<String>,
    voting_addr: Option<String>,
  },
  Revoke {
    pro_tx_hash: Vec<u8>,
  },
}

/// Fields for a new row in the `validators` table (produced by ProRegTx).
#[derive(Clone, Debug)]
pub struct ValidatorInsert {
  pub pro_tx_hash: Vec<u8>,
  pub collateral_hash: Vec<u8>,
  pub collateral_index: i32,
  pub collateral_address: Option<String>,
  pub operator_pubkey: Vec<u8>,
  pub owner_address: Option<String>,
  pub voting_address: Option<String>,
  pub operator_reward: i32,
  pub validator_type: String,
  pub payout_addr: Option<String>,
  pub core_addrs: Vec<String>,
  pub plat_p2p_addrs: Vec<String>,
  pub plat_https_addrs: Vec<String>,
}

/// Return the `(pro_tx_hash, payload_json)` pair for `special_tx_payloads`.
/// `txid` is the txid of the transaction (used as pro_tx_hash for ProRegTx).
pub fn payload_for_storage(payload: &SpecialPayload, txid: &[u8]) -> (Option<Vec<u8>>, Value) {
  match payload {
    SpecialPayload::Coinbase(p) => (None, coinbase::to_json(p)),
    SpecialPayload::ProviderRegister(p) => (Some(txid.to_vec()), provider_register::to_json(p)),
    SpecialPayload::ProviderUpdateService(p) => {
      let pth = p.pro_tx_hash.as_bytes().to_vec();
      (Some(pth), provider_update_service::to_json(p))
    }
    SpecialPayload::ProviderUpdateRegistrar(p) => {
      let pth = p.pro_tx_hash.as_bytes().to_vec();
      (Some(pth), provider_update_registrar::to_json(p))
    }
    SpecialPayload::ProviderUpdateRevoke(p) => {
      let pth = p.pro_tx_hash.as_bytes().to_vec();
      (Some(pth), provider_update_revoke::to_json(p))
    }
    SpecialPayload::QuorumCommitment(p) => (None, quorum::to_json(p)),
    SpecialPayload::MnhfSignal(p) => (None, mnhf::to_json(p)),
    SpecialPayload::AssetLock(p) => (None, asset_lock::to_json(p)),
    SpecialPayload::AssetUnlock(p) => (None, asset_unlock::to_json(p)),
    SpecialPayload::Unknown { .. } => (None, serde_json::json!({})),
  }
}

/// Return the `ValidatorEffect` for the given payload, if any.
pub fn validator_effect(
  payload: &SpecialPayload,
  txid: &[u8],
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Option<ValidatorEffect> {
  match payload {
    SpecialPayload::ProviderRegister(p) => Some(provider_register::to_effect(
      p,
      txid,
      p2pkh_version,
      p2sh_version,
    )),
    SpecialPayload::ProviderUpdateService(p) => Some(provider_update_service::to_effect(p)),
    SpecialPayload::ProviderUpdateRegistrar(p) => Some(provider_update_registrar::to_effect(
      p,
      p2pkh_version,
      p2sh_version,
    )),
    SpecialPayload::ProviderUpdateRevoke(p) => Some(provider_update_revoke::to_effect(p)),
    _ => None,
  }
}

/// Shape stored JSONB for an API response.
pub fn enrich_api(tx_type: &str, stored: &Value) -> Value {
  match tx_type {
    "coinbase" => coinbase::to_api(stored),
    "provider_register" => provider_register::to_api(stored),
    "provider_update_service" => provider_update_service::to_api(stored),
    "provider_update_registrar" => provider_update_registrar::to_api(stored),
    "provider_update_revoke" => provider_update_revoke::to_api(stored),
    "quorum_commitment" => quorum::to_api(stored),
    "mnhf" => mnhf::to_api(stored),
    "asset_lock" => asset_lock::to_api(stored),
    "asset_unlock" => asset_unlock::to_api(stored),
    _ => stored.clone(),
  }
}

/// Convert a `TxType` to the string stored in the `tx_type` PG enum column.
pub fn tx_type_str(t: dash_primitives::types::tx::TxType) -> &'static str {
  use dash_primitives::types::tx::TxType;
  match t {
    TxType::Normal => "normal",
    TxType::ProviderRegister => "provider_register",
    TxType::ProviderUpdateService => "provider_update_service",
    TxType::ProviderUpdateRegistrar => "provider_update_registrar",
    TxType::ProviderUpdateRevoke => "provider_update_revoke",
    TxType::Coinbase => "coinbase",
    TxType::QuorumCommitment => "quorum_commitment",
    TxType::AssetLock => "asset_lock",
    TxType::AssetUnlock => "asset_unlock",
    TxType::MnhfSignal => "mnhf",
    TxType::Unknown(_) => "normal",
  }
}

/// Return the public-facing transaction type for API responses.
pub fn public_tx_type<I>(stored_tx_type: &str, output_scripts: I) -> &str
where
  I: IntoIterator,
  I::Item: AsRef<[u8]>,
{
  if stored_tx_type == "normal"
    && output_scripts
      .into_iter()
      .any(|script| matches!(classify_script(script.as_ref()), ScriptKind::OpReturn))
  {
    "data"
  } else {
    stored_tx_type
  }
}

// ── Address helpers (shared by provider tx modules) ──────────────────────────

pub(crate) fn net_info_to_addrs(
  net_info: &dash_primitives::types::sptx::NetInfo,
) -> (Vec<String>, Vec<String>, Vec<String>) {
  use dash_primitives::types::sptx::{NetInfo, NetInfoEntry, NetInfoPurpose};
  match net_info {
    NetInfo::Legacy(svc) => (vec![format_service(svc)], vec![], vec![]),
    NetInfo::Extended(ext) => {
      let mut core = vec![];
      let mut plat_p2p = vec![];
      let mut plat_https = vec![];
      for (purpose, entries) in &ext.entries {
        let target = match purpose {
          NetInfoPurpose::CoreP2P => &mut core,
          NetInfoPurpose::PlatformP2P => &mut plat_p2p,
          NetInfoPurpose::PlatformHttps => &mut plat_https,
          NetInfoPurpose::Unknown(_) => continue,
        };
        for entry in entries {
          match entry {
            NetInfoEntry::Service(svc) => target.push(format_service(svc)),
            NetInfoEntry::Domain { name, port } => {
              if let Ok(s) = std::str::from_utf8(name) {
                target.push(format!("{s}:{port}"));
              }
            }
            NetInfoEntry::Invalid => {}
          }
        }
      }
      (core, plat_p2p, plat_https)
    }
  }
}

fn format_service(svc: &dash_primitives::types::sptx::CService) -> String {
  format!("{}:{}", format_addr(&svc.addr), svc.port)
}

fn format_addr(addr: &[u8]) -> String {
  match addr.len() {
    4 => format!("{}.{}.{}.{}", addr[0], addr[1], addr[2], addr[3]),
    16 => {
      let mut parts = Vec::new();
      for chunk in addr.chunks(2) {
        parts.push(format!("{:02x}{:02x}", chunk[0], chunk[1]));
      }
      parts.join(":")
    }
    _ => hex::encode(addr),
  }
}

#[cfg(test)]
mod tests {
  #[test]
  fn public_tx_type_marks_normal_op_return_transactions_as_data() {
    let tx_type = super::public_tx_type("normal", [vec![0x6a, 0x01, 0x01]]);
    assert_eq!(tx_type, "data");
  }

  #[test]
  fn public_tx_type_keeps_non_op_return_transactions_normal() {
    let tx_type = super::public_tx_type(
      "normal",
      [vec![
        0x76, 0xa9, 0x14, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
        0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x88, 0xac,
      ]],
    );
    assert_eq!(tx_type, "normal");
  }

  #[test]
  fn public_tx_type_keeps_special_transaction_types() {
    let tx_type = super::public_tx_type("asset_lock", [vec![0x6a, 0x01, 0x01]]);
    assert_eq!(tx_type, "asset_lock");
  }
}
