//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::script;
use dash_primitives::tx::proregtx::CProRegTx;
use dash_primitives::util::encode_key_id_address;
use serde_json::{json, Value};

use super::{net_info_to_addrs, ValidatorEffect, ValidatorInsert};
use crate::utils::{self, HashEndian};

pub fn to_json(p: &CProRegTx) -> Value {
  json!({
    "version": p.version,
    "mnType": format!("{:?}", p.mn_type),
    "collateralHash": utils::view_hex(p.collateral_outpoint.hash.as_bytes(), HashEndian::Ser),
    "collateralIndex": p.collateral_outpoint.index,
    "ownerKeyId": hex::encode(p.key_id_owner.as_bytes()),
    "operatorPubKey": hex::encode(p.pub_key_operator.as_bytes()),
    "votingKeyId": hex::encode(p.key_id_voting.as_bytes()),
    "operatorReward": p.operator_reward,
  })
}

pub fn to_effect(
  p: &CProRegTx,
  txid: &[u8],
  p2pkh_version: u8,
  p2sh_version: u8,
) -> ValidatorEffect {
  let (core_addrs, plat_p2p_addrs, plat_https_addrs) = net_info_to_addrs(&p.net_info);
  let payout_addr = script::derive_address(p.script_payout.as_bytes(), p2pkh_version, p2sh_version);
  let owner_address = Some(encode_key_id_address(&p.key_id_owner, p2pkh_version));
  let voting_address = Some(encode_key_id_address(&p.key_id_voting, p2pkh_version));
  ValidatorEffect::Insert(ValidatorInsert {
    pro_tx_hash: txid.to_vec(),
    collateral_hash: p.collateral_outpoint.hash.as_bytes().to_vec(),
    collateral_index: p.collateral_outpoint.index as i32,
    collateral_address: None,
    operator_pubkey: p.pub_key_operator.as_bytes().to_vec(),
    owner_address,
    voting_address,
    operator_reward: p.operator_reward as i32,
    validator_type: format!("{:?}", p.mn_type),
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
