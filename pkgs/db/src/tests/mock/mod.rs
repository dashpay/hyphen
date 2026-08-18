//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Mock scaffolding. Deliberately thin: the real memory-backed fake domain
//! impls live in the daemon (next to their consumers). What lives here is only
//! what the daemon cannot build for itself without reaching into crate
//! internals — a trivial data source over a self-describing synthetic encoding.
//!
//! Encoding: the raw bytes are `hash(32) || prev_hash(32)` followed by an
//! optional single output value as 8 big-endian bytes. This lets ingest tests
//! round-trip a chain without a real Dash decoder.

use crate::error::{HyDbError, HyDbResult};
use crate::records::{
  HyDbBlock, HyDbBlockId, HyDbDataSrc, HyDbHeader, HyDbTx, HyDbTxIn, HyDbTxOut,
};
use crate::types::{HyDbAddrType, HyDbAddress, HyDbAmount, HyDbHash};

#[derive(Debug, Default)]
pub struct MockDataSrc;

fn split_id(raw: &[u8]) -> HyDbResult<(HyDbHash, HyDbHash)> {
  if raw.len() < 64 {
    return Err(HyDbError::Derive(format!(
      "mock block needs >= 64 bytes, got {}",
      raw.len()
    )));
  }
  Ok((
    HyDbHash::from_slice(&raw[..32])?,
    HyDbHash::from_slice(&raw[32..64])?,
  ))
}

impl HyDbDataSrc for MockDataSrc {
  fn identify(&self, raw: &[u8]) -> HyDbResult<HyDbBlockId> {
    let (hash, prev_hash) = split_id(raw)?;
    Ok(HyDbBlockId { hash, prev_hash })
  }

  fn decode(&self, raw: &[u8]) -> HyDbResult<HyDbBlock> {
    let (hash, prev_hash) = split_id(raw)?;
    let value = if raw.len() >= 72 {
      let mut v = [0u8; 8];
      v.copy_from_slice(&raw[64..72]);
      i64::from_be_bytes(v)
    } else {
      0
    };
    // A single coinbase-style transaction paying `value` to a fixed address.
    let txid = hash; // deterministic, unique per block
    let tx = HyDbTx {
      txid,
      block_pos: 0,
      version: 1,
      tx_type: crate::types::HyDbTxType::Coinbase,
      lock_time: 0,
      size: raw.len() as u32,
      value_out: HyDbAmount(value),
      inputs: vec![HyDbTxIn {
        vin_index: 0,
        prev_outpoint: None,
        coinbase: true,
        script_sig: Vec::new(),
        sequence: 0xffff_ffff,
      }],
      outputs: vec![HyDbTxOut {
        vout: 0,
        value: HyDbAmount(value),
        script_pubkey: vec![0x76, 0xa9, 0x14],
        address: Some(HyDbAddress("yMockAddr".to_owned())),
        address_type: HyDbAddrType::P2pkh,
      }],
      payload: None,
      validator_events: Vec::new(),
    };
    Ok(HyDbBlock {
      header: HyDbHeader {
        hash,
        prev_hash,
        merkle_root: txid,
        time: 1_600_000_000,
        bits: 0x1e0f_ffff,
        difficulty: 1.0,
        nonce: 0,
        version: 1,
        size: raw.len() as u32,
        tx_count: 1,
        chainwork: None,
        coinbase: None,
      },
      txs: vec![tx],
    })
  }
}
