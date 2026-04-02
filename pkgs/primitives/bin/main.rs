//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Parse raw Dash blocks from stdin to JSON.

use std::io::Read;

use dash_primitives::block::full::Block;
use dash_primitives::cursor::Cursor;

fn run() -> Result<(), String> {
  let mut buf = Vec::new();
  std::io::stdin()
    .read_to_end(&mut buf)
    .map_err(|e| format!("failed to read stdin: {e}"))?;

  if buf.is_empty() {
    return Err("no data provided on stdin".to_string());
  }

  let mut cursor = Cursor::new(&buf);
  let block = Block::consensus_decode_block(&mut cursor)
    .map_err(|e| format!("failed to decode block: {e}"))?;

  let json = serde_json::to_string_pretty(&block)
    .map_err(|e| format!("failed to serialize to JSON: {e}"))?;

  println!("{json}");
  Ok(())
}

fn main() {
  if let Err(e) = run() {
    eprintln!("error: {e}");
    std::process::exit(1);
  }
}
