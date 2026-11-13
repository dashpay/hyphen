//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::CoinbaseCommitment;
use dash_types::Numeric;
use serde_json::{json, Value};

pub fn to_json(p: &CoinbaseCommitment) -> Value {
  let mut value = json!({
    "version": p.version().to_base(),
    "height": p.height().to_u32(),
    "merkleRootMnList": hex::encode(p.merkle_root_mn_list().as_bytes()),
    "merkleRootQuorums": p.merkle_root_quorums().as_ref().map(|r| hex::encode(r.as_bytes())),
    "bestClHeightDiff": p.best_cl_height_diff(),
    "bestClSignature": p.best_cl_signature().as_ref().map(|s| hex::encode(s.as_bytes())),
    "creditPoolBalance": p.credit_pool_balance(),
  });
  if let Some(root) = p.merkle_root_asset_unlocks() {
    value["merkleRootAssetUnlocks"] = Value::String(hex::encode(root.as_bytes()));
  }
  value
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}
