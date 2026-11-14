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

use dash_primitives::{PayoutShare, SpecialPayload};
use dash_script::Recipient;
use dash_types::Hashable;
use serde_json::Value;

/// Side-effect a provider-tx has on the `validators` table.
#[derive(Clone, Debug)]
pub enum ValidatorEffect {
  Insert(ValidatorInsert),
  ReapSpentCollateral {
    collateral_hash: Vec<u8>,
    collateral_index: i32,
  },
  UpdateService {
    pro_tx_hash: Vec<u8>,
    core_addrs: Vec<String>,
    plat_p2p_addrs: Vec<String>,
    plat_https_addrs: Vec<String>,
    operator_payout_addr: Option<String>,
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
  pub operator_payout_addr: Option<String>,
  pub core_addrs: Vec<String>,
  pub plat_p2p_addrs: Vec<String>,
  pub plat_https_addrs: Vec<String>,
}

/// Return the `(pro_tx_hash, payload_json)` pair for `special_tx_payloads`.
/// `txid` is the txid of the transaction (used as pro_tx_hash for ProRegTx).
pub fn payload_for_storage(
  payload: &SpecialPayload,
  txid: &[u8],
  p2pkh_version: u8,
  p2sh_version: u8,
) -> (Option<Vec<u8>>, Value) {
  match payload {
    SpecialPayload::CoinbaseCommitment(p) => (None, coinbase::to_json(p)),
    SpecialPayload::ProviderRegister(p) => (
      Some(txid.to_vec()),
      provider_register::to_json(p, p2pkh_version, p2sh_version),
    ),
    SpecialPayload::ProviderUpdateService(p) => {
      let pth = p.pro_tx_hash().as_bytes().to_vec();
      (Some(pth), provider_update_service::to_json(p))
    }
    SpecialPayload::ProviderUpdateRegistrar(p) => {
      let pth = p.pro_tx_hash().as_bytes().to_vec();
      (
        Some(pth),
        provider_update_registrar::to_json(p, p2pkh_version, p2sh_version),
      )
    }
    SpecialPayload::ProviderUpdateRevoke(p) => {
      let pth = p.pro_tx_hash().as_bytes().to_vec();
      (Some(pth), provider_update_revoke::to_json(p))
    }
    SpecialPayload::QuorumCommitment(p) => (None, quorum::to_json(p)),
    SpecialPayload::MnhfSignal(p) => (None, mnhf::to_json(p)),
    SpecialPayload::AssetLock(p) => (None, asset_lock::to_json(p)),
    SpecialPayload::AssetUnlock(p) => (None, asset_unlock::to_json(p)),
    SpecialPayload::Unknown { .. }
    | SpecialPayload::ProviderDissolve(_)
    | SpecialPayload::ProviderUpdateShare(_)
    | SpecialPayload::ProviderUpdateSharedRegistrar(_) => (None, serde_json::json!({})),
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
    SpecialPayload::ProviderUpdateService(p) => Some(provider_update_service::to_effect(
      p,
      p2pkh_version,
      p2sh_version,
    )),
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
pub fn tx_type_str(t: dash_primitives::TxType) -> &'static str {
  use dash_primitives::TxType;
  match t {
    TxType::Spend => "normal",
    TxType::ProviderRegister => "provider_register",
    TxType::ProviderUpdateService => "provider_update_service",
    TxType::ProviderUpdateRegistrar => "provider_update_registrar",
    TxType::ProviderUpdateRevoke => "provider_update_revoke",
    TxType::CoinbaseCommitment => "coinbase",
    TxType::QuorumCommitment => "quorum_commitment",
    TxType::AssetLock => "asset_lock",
    TxType::AssetUnlock => "asset_unlock",
    TxType::MnhfSignal => "mnhf",
    TxType::ProviderDissolve
    | TxType::ProviderUpdateShare
    | TxType::ProviderUpdateSharedRegistrar => "normal",
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
    && output_scripts.into_iter().any(|script| {
      matches!(
        Recipient::from_script(script.as_ref()),
        Some(Recipient::Unspendable)
      )
    })
  {
    "data"
  } else {
    stored_tx_type
  }
}

// ── Address helpers (shared by provider tx modules) ──────────────────────────

pub(crate) fn script_address(script: &[u8], p2pkh_version: u8, p2sh_version: u8) -> Option<String> {
  match Recipient::from_script(script)? {
    Recipient::PubKeyHash(hash) => Some(hash.to_base58c(p2pkh_version)),
    Recipient::ScriptHash(hash) => Some(hash.to_base58c(p2sh_version)),
    Recipient::PubKey(key) => Some(key.hash().to_base58c(p2pkh_version)),
    Recipient::Unspendable => None,
  }
}

pub(crate) fn payouts_json(
  payouts: &[PayoutShare],
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Vec<Value> {
  payouts
    .iter()
    .map(|share| {
      serde_json::json!({
        "address": script_address(
          share.script_payout().as_bytes(),
          p2pkh_version,
          p2sh_version,
        )
        .unwrap_or_else(|| hex::encode(share.script_payout().as_bytes())),
        "reward": share.reward(),
      })
    })
    .collect()
}

pub(crate) fn payout_address(
  script: &[u8],
  payouts: &[PayoutShare],
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Option<String> {
  if !script.is_empty() {
    return script_address(script, p2pkh_version, p2sh_version);
  }
  payouts
    .iter()
    .reduce(|best, share| {
      if share.reward() > best.reward() {
        share
      } else {
        best
      }
    })
    .and_then(|share| {
      script_address(
        share.script_payout().as_bytes(),
        p2pkh_version,
        p2sh_version,
      )
    })
}

pub(crate) fn net_info_to_addrs(
  net_info: &dash_primitives::NetInfo,
) -> (Vec<String>, Vec<String>, Vec<String>) {
  use dash_primitives::NetInfo;
  use dash_primitives::{NIEntry, NIPurpose};
  match net_info {
    NetInfo::Legacy(svc) => (
      vec![format_service(&dash_primitives::ServiceV2::from(
        svc.service(),
      ))],
      vec![],
      vec![],
    ),
    NetInfo::Extended(ext) => {
      let mut core = vec![];
      let mut plat_p2p = vec![];
      let mut plat_https = vec![];
      for (purpose, entries) in ext.groups() {
        let target = match purpose {
          NIPurpose::CoreP2p => &mut core,
          NIPurpose::PlatformP2p => &mut plat_p2p,
          NIPurpose::PlatformHttps => &mut plat_https,
          NIPurpose::Unknown(_) => continue,
        };
        for entry in entries {
          match entry {
            NIEntry::Service(svc) => target.push(format_service(svc)),
            NIEntry::Domain { name, port } => {
              if let Ok(s) = std::str::from_utf8(name) {
                target.push(format!("{s}:{port}"));
              }
            }
            NIEntry::Invalid => {}
          }
        }
      }
      (core, plat_p2p, plat_https)
    }
  }
}

fn format_service(svc: &dash_primitives::ServiceV2) -> String {
  match svc.addr() {
    dash_primitives::AddrV2::Ipv4(addr) => {
      format!(
        "{}.{}.{}.{}:{}",
        addr[0],
        addr[1],
        addr[2],
        addr[3],
        svc.port()
      )
    }
    dash_primitives::AddrV2::Ipv6(addr) => {
      let address = addr
        .chunks_exact(2)
        .map(|chunk| format!("{:02x}{:02x}", chunk[0], chunk[1]))
        .collect::<Vec<_>>()
        .join(":");
      format!("{address}:{}", svc.port())
    }
    _ => svc.to_string(),
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

  #[test]
  fn format_service_ipv4() {
    let service =
      dash_primitives::ServiceV2::new(dash_primitives::AddrV2::Ipv4([1, 2, 3, 4]), 9999);
    assert_eq!(super::format_service(&service), "1.2.3.4:9999");
  }

  #[test]
  fn format_service_ipv6() {
    let service = dash_primitives::ServiceV2::new(
      dash_primitives::AddrV2::Ipv6([
        0x20, 0x01, 0x0d, 0xb8, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x01,
      ]),
      9999,
    );
    assert_eq!(
      super::format_service(&service),
      "2001:0db8:0000:0000:0000:0000:0000:0001:9999"
    );
  }

  #[test]
  fn format_service_legacy_ipv4_mapped() {
    let legacy = dash_primitives::ServiceV1::new("192.168.1.100".parse().expect("IPv4"), 9999);
    let service = dash_primitives::ServiceV2::from(legacy);
    assert_eq!(super::format_service(&service), "192.168.1.100:9999");
  }
}
