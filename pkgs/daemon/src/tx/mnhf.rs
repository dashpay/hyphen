//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use dash_primitives::payload::MnHardFork;
use serde_json::{json, Value};

pub fn to_json(p: &MnHardFork) -> Value {
  json!({
    "version": p.version,
    "versionBit": p.version_bit,
    "quorumHash": hex::encode(p.quorum_hash.as_bytes()),
  })
}

pub fn to_api(stored: &Value) -> Value {
  stored.clone()
}
