//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::FinalCommitment;
use dash_types::Numeric;
use serde_json::{json, Value};

pub fn to_json(p: &FinalCommitment) -> Value {
  json!({
    "version": p.version().to_base(),
    "height": p.height().to_u32(),
    "commitmentVersion": p.commitment().version(),
    "llmqType": format!("{:?}", p.commitment().llmq_type()),
    "quorumHash": hex::encode(p.commitment().quorum_hash().as_bytes()),
    "quorumIndex": p.commitment().quorum_index(),
  })
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}
