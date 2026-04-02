//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! WASM entry points for the block parser.

use alloc::string::{String, ToString};

use wasm_bindgen::prelude::*;

use crate::block::full::Block;
use crate::cursor::Cursor;
use crate::hex;

/// Hex block data in, pretty JSON string out.
#[wasm_bindgen]
pub fn parse_block_hex(hex_str: &str) -> Result<String, String> {
  let bytes = hex::decode_hex(hex_str).map_err(|e| alloc::format!("invalid hex: {e}"))?;

  if bytes.is_empty() {
    return Err("no data provided".to_string());
  }

  let mut cursor = Cursor::new(&bytes);
  let block = Block::consensus_decode_block(&mut cursor)
    .map_err(|e| alloc::format!("failed to decode block: {e}"))?;

  serde_json::to_string_pretty(&block)
    .map_err(|e| alloc::format!("failed to serialize to JSON: {e}"))
}
