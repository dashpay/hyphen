//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::ProUpRevTx;
use serde_json::{json, Value};

use super::ValidatorEffect;
use crate::utils::{self, HashEndian};

pub fn to_json(p: &ProUpRevTx) -> Value {
  json!({
    "version": p.version().to_base(),
    "proTxHash": utils::view_hex(p.pro_tx_hash().as_bytes(), HashEndian::Ser),
    "reason": format!("{:?}", p.reason()),
  })
}

pub fn to_effect(p: &ProUpRevTx) -> ValidatorEffect {
  ValidatorEffect::Revoke {
    pro_tx_hash: p.pro_tx_hash().as_bytes().to_vec(),
  }
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}
