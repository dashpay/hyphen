//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::payload::ProUpRegTx;
use serde_json::{json, Value};

use super::ValidatorEffect;
use crate::utils::{self, HashEndian};

pub fn to_json(p: &ProUpRegTx) -> Value {
  json!({
    "version": p.version,
    "proTxHash": utils::view_hex(p.pro_tx_hash.as_bytes(), HashEndian::Ser),
    "operatorPubKey": hex::encode(p.pub_key_operator.0),
    "votingKeyId": hex::encode(p.key_id_voting.0),
  })
}

pub fn to_effect(p: &ProUpRegTx, p2pkh_version: u8, p2sh_version: u8) -> ValidatorEffect {
  let payout_addr =
    dash_script::derive_address(p.script_payout.as_bytes(), p2pkh_version, p2sh_version);
  let voting_addr = Some(dash_script::encode_key_id(&p.key_id_voting, p2pkh_version));
  ValidatorEffect::UpdateRegistrar {
    pro_tx_hash: p.pro_tx_hash.as_bytes().to_vec(),
    operator_pubkey: p.pub_key_operator.0.to_vec(),
    payout_addr,
    voting_addr,
  }
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}
