//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::MnHardFork;
use dash_types::Numeric;
use serde_json::{json, Value};

pub fn to_json(p: &MnHardFork) -> Value {
  json!({
    "version": p.version().to_base(),
    "versionBit": p.version_bit(),
    "quorumHash": hex::encode(p.quorum_hash().as_bytes()),
  })
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}
