//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::script;
use dash_primitives::tx::proupservtx::CProUpServTx;
use serde_json::{json, Value};

use super::{net_info_to_addrs, ValidatorEffect};
use crate::utils::{self, HashEndian};

pub fn to_json(p: &CProUpServTx) -> Value {
  json!({
    "version": p.version,
    "proTxHash": utils::view_hex(p.pro_tx_hash.as_bytes(), HashEndian::Ser),
  })
}

pub fn to_effect(p: &CProUpServTx, p2pkh_version: u8, p2sh_version: u8) -> ValidatorEffect {
  let (core_addrs, plat_p2p_addrs, plat_https_addrs) = net_info_to_addrs(&p.net_info);
  ValidatorEffect::UpdateService {
    pro_tx_hash: p.pro_tx_hash.as_bytes().to_vec(),
    core_addrs,
    plat_p2p_addrs,
    plat_https_addrs,
    operator_payout_addr: script::derive_address(
      p.script_operator_payout.as_bytes(),
      p2pkh_version,
      p2sh_version,
    ),
  }
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}
