//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::ProRegTx;
use serde_json::{json, Value};

use super::{net_info_to_addrs, payout_address, payouts_json, ValidatorEffect, ValidatorInsert};
use crate::utils::{self, HashEndian};

pub fn to_json(p: &ProRegTx, p2pkh_version: u8, p2sh_version: u8) -> Value {
  let mut value = json!({
    "version": p.version().to_base(),
    "mnType": format!("{:?}", p.mn_type()),
    "collateralHash": utils::view_hex(p.collateral_hash().as_bytes(), HashEndian::Ser),
    "collateralIndex": p.collateral_index(),
    "ownerKeyId": hex::encode(p.key_id_owner().as_bytes()),
    "operatorPubKey": hex::encode(p.pub_key_operator().as_bytes()),
    "votingKeyId": hex::encode(p.key_id_voting().as_bytes()),
    "operatorReward": p.operator_reward(),
  });
  if !p.payouts().is_empty() {
    value["payouts"] = Value::Array(payouts_json(p.payouts(), p2pkh_version, p2sh_version));
  }
  value
}

pub fn to_effect(
  p: &ProRegTx,
  txid: &[u8],
  p2pkh_version: u8,
  p2sh_version: u8,
) -> ValidatorEffect {
  let (core_addrs, plat_p2p_addrs, plat_https_addrs) = net_info_to_addrs(p.net_info());
  let payout_addr = payout_address(
    p.script_payout().as_bytes(),
    p.payouts(),
    p2pkh_version,
    p2sh_version,
  );
  let owner_address = Some(p.key_id_owner().to_base58c(p2pkh_version));
  let voting_address = Some(p.key_id_voting().to_base58c(p2pkh_version));
  ValidatorEffect::Insert(ValidatorInsert {
    pro_tx_hash: txid.to_vec(),
    collateral_hash: p.collateral_hash().as_bytes().to_vec(),
    collateral_index: p.collateral_index() as i32,
    collateral_address: None,
    operator_pubkey: p.pub_key_operator().as_bytes().to_vec(),
    owner_address,
    voting_address,
    operator_reward: p.operator_reward() as i32,
    validator_type: format!("{:?}", p.mn_type()),
    payout_addr,
    operator_payout_addr: None,
    core_addrs,
    plat_p2p_addrs,
    plat_https_addrs,
  })
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}

#[cfg(test)]
mod tests {
  use super::*;
  use dash_primitives::{SpecialPayload, Transaction};
  use dash_types::Hashable;

  #[test]
  fn core_v24_vector_uses_highest_payout_and_exposes_shares() {
    let raw = hex::decode(include_str!("testdata/proreg_v3.hex").trim()).expect("valid SDK vector");
    let tx: Transaction = crate::consensus::decode(&raw).expect("valid transaction");
    let mut payload = tx.extra_payload().as_slice();
    let payload = SpecialPayload::decode(tx.tx_type(), &mut payload).expect("valid payload");
    let SpecialPayload::ProviderRegister(payload) = payload else {
      panic!("expected provider register payload");
    };
    assert!(payload.script_payout().as_bytes().is_empty());
    let json = to_json(&payload, 111, 196);
    assert_eq!(json["payouts"][0]["reward"], 10_000);
    assert!(json["payouts"][0]["address"].as_str().is_some());
    let effect = to_effect(&payload, tx.hash().as_bytes(), 111, 196);
    let ValidatorEffect::Insert(insert) = effect else {
      panic!("expected provider register effect");
    };
    assert_eq!(
      insert.payout_addr.as_deref(),
      json["payouts"][0]["address"].as_str()
    );
  }
}
