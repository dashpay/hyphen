//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::tx::asset_lock::CAssetLockPayload;
use serde_json::{json, Value};

pub fn to_json(p: &CAssetLockPayload) -> Value {
  let outputs: Vec<Value> = p
    .credit_outputs
    .iter()
    .map(|o| {
      json!({
        "value": o.value,
        "scriptPubKey": hex::encode(o.script_pubkey.as_bytes()),
      })
    })
    .collect();
  json!({
    "version": p.version,
    "creditOutputs": outputs,
  })
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}
