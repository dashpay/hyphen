//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::ProUpRegTx;
use serde_json::{json, Value};

use super::{payout_address, payouts_json, ValidatorEffect};
use crate::utils::{self, HashEndian};

pub fn to_json(p: &ProUpRegTx, p2pkh_version: u8, p2sh_version: u8) -> Value {
  let mut value = json!({
    "version": p.version().to_base(),
    "proTxHash": utils::view_hex(p.pro_tx_hash().as_bytes(), HashEndian::Ser),
    "operatorPubKey": hex::encode(p.pub_key_operator().as_bytes()),
    "votingKeyId": hex::encode(p.key_id_voting().as_bytes()),
  });
  if !p.payouts().is_empty() {
    value["payouts"] = Value::Array(payouts_json(p.payouts(), p2pkh_version, p2sh_version));
  }
  value
}

pub fn to_effect(p: &ProUpRegTx, p2pkh_version: u8, p2sh_version: u8) -> ValidatorEffect {
  let payout_addr = payout_address(
    p.script_payout().as_bytes(),
    p.payouts(),
    p2pkh_version,
    p2sh_version,
  );
  let voting_addr = Some(p.key_id_voting().to_base58c(p2pkh_version));
  ValidatorEffect::UpdateRegistrar {
    pro_tx_hash: p.pro_tx_hash().as_bytes().to_vec(),
    operator_pubkey: p.pub_key_operator().as_bytes().to_vec(),
    payout_addr,
    voting_addr,
  }
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}

#[cfg(test)]
mod tests {
  use super::*;
  use dash_primitives::{SpecialPayload, Transaction};

  #[test]
  fn core_v24_vector_exposes_all_payout_shares() {
    let raw =
      hex::decode(include_str!("testdata/proupreg_v3.hex").trim()).expect("valid SDK vector");
    let tx: Transaction = crate::consensus::decode(&raw).expect("valid transaction");
    let mut payload = tx.extra_payload().as_slice();
    let payload = SpecialPayload::decode(tx.tx_type(), &mut payload).expect("valid payload");
    let SpecialPayload::ProviderUpdateRegistrar(payload) = payload else {
      panic!("expected provider update registrar payload");
    };
    assert!(payload.script_payout().as_bytes().is_empty());
    let json = to_json(&payload, 111, 196);
    assert_eq!(json["payouts"].as_array().expect("payout list").len(), 8);
    assert!(json["payouts"]
      .as_array()
      .expect("payout list")
      .iter()
      .all(|share| share["address"].as_str().is_some()));
    let effect = to_effect(&payload, 111, 196);
    let ValidatorEffect::UpdateRegistrar { payout_addr, .. } = effect else {
      panic!("expected registrar update effect");
    };
    assert_eq!(
      payout_addr,
      json["payouts"][0]["address"].as_str().map(str::to_owned)
    );
  }

  #[test]
  fn payout_address_selects_highest_reward_and_first_tie() {
    let raw =
      hex::decode(include_str!("testdata/proupreg_v3.hex").trim()).expect("valid SDK vector");
    let tx: Transaction = crate::consensus::decode(&raw).expect("valid transaction");
    let mut payload = tx.extra_payload().as_slice();
    let payload = SpecialPayload::decode(tx.tx_type(), &mut payload).expect("valid payload");
    let SpecialPayload::ProviderUpdateRegistrar(payload) = payload else {
      panic!("expected provider update registrar payload");
    };
    let shares = payload.payouts();
    assert!(shares.len() >= 2, "fixture should contain payout shares");
    let raw_register =
      hex::decode(include_str!("testdata/proreg_v3.hex").trim()).expect("valid SDK vector");
    let tx_register: Transaction =
      crate::consensus::decode(&raw_register).expect("valid transaction");
    let mut register_payload = tx_register.extra_payload().as_slice();
    let register_payload =
      SpecialPayload::decode(tx_register.tx_type(), &mut register_payload).expect("valid payload");
    let SpecialPayload::ProviderRegister(register_payload) = register_payload else {
      panic!("expected provider register payload");
    };
    let reordered = [shares[0].clone(), register_payload.payouts()[0].clone()];
    assert_eq!(
      payout_address(&[], &reordered, 111, 196),
      payout_address(
        &[],
        std::slice::from_ref(&register_payload.payouts()[0]),
        111,
        196,
      )
    );
    let tied = [shares[0].clone(), shares[1].clone()];
    assert_eq!(
      payout_address(&[], &tied, 111, 196),
      payout_address(&[], std::slice::from_ref(&shares[0]), 111, 196)
    );
  }
}
