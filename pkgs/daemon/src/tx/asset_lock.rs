//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::payload::AssetLock;
use serde_json::{json, Value};

pub fn to_json(p: &AssetLock) -> Value {
  let outputs: Vec<Value> = p
    .credit_outputs
    .iter()
    .map(|o| {
      json!({
        "value": o.value.to_sat(),
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
