//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::tx::coinbase::CCbTx;
use serde_json::{json, Value};

pub fn to_json(p: &CCbTx) -> Value {
  json!({
    "version": p.version,
    "height": p.height,
    "merkleRootMnList": hex::encode(p.merkle_root_mn_list.as_bytes()),
    "merkleRootQuorums": p.merkle_root_quorums.as_ref().map(|r| hex::encode(r.as_bytes())),
    "bestClHeightDiff": p.best_cl_height_diff,
    "bestClSignature": p.best_cl_signature.as_ref().map(|s| hex::encode(s.as_bytes())),
    "creditPoolBalance": p.credit_pool_balance,
  })
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}
