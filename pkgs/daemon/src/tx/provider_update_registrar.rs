//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::script;
use dash_primitives::tx::proupregtx::CProUpRegTx;
use dash_primitives::util::encode_key_id_address;
use serde_json::{json, Value};

use super::ValidatorEffect;
use crate::utils::{self, HashEndian};

pub fn to_json(p: &CProUpRegTx) -> Value {
  json!({
    "version": p.version,
    "proTxHash": utils::view_hex(p.pro_tx_hash.as_bytes(), HashEndian::Ser),
    "operatorPubKey": hex::encode(p.pub_key_operator.as_bytes()),
    "votingKeyId": hex::encode(p.key_id_voting.as_bytes()),
  })
}

pub fn to_effect(p: &CProUpRegTx, p2pkh_version: u8, p2sh_version: u8) -> ValidatorEffect {
  let payout_addr = script::derive_address(p.script_payout.as_bytes(), p2pkh_version, p2sh_version);
  let voting_addr = Some(encode_key_id_address(&p.key_id_voting, p2pkh_version));
  ValidatorEffect::UpdateRegistrar {
    pro_tx_hash: p.pro_tx_hash.as_bytes().to_vec(),
    operator_pubkey: p.pub_key_operator.as_bytes().to_vec(),
    payout_addr,
    voting_addr,
  }
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}
