//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Deterministic builders for synthetic facts, so tests and the daemon's mock
//! backend can construct chains without decoding real Dash blocks.

use crate::records::{HyDbBlock, HyDbHeader, HyDbTx, HyDbTxIn, HyDbTxOut};
use crate::types::{HyDbAddrType, HyDbAddress, HyDbAmount, HyDbHash, HyDbOutpoint};

/// A distinct 32-byte hash seeded from a `u64` (little-endian in the low
/// bytes).
pub fn hash_from_u64(seed: u64) -> HyDbHash {
  let mut bytes = [0u8; 32];
  bytes[..8].copy_from_slice(&seed.to_le_bytes());
  HyDbHash(bytes)
}

/// A one-transaction block at `height`, linked to `prev_hash`, paying `value`
/// to `address` and (unless height 0) spending `spends`.
pub fn simple_block(
  height: u32,
  prev_hash: HyDbHash,
  address: &str,
  value: i64,
  spends: Option<HyDbOutpoint>,
) -> HyDbBlock {
  let txid = hash_from_u64(0x1000_0000 + height as u64);
  let inputs = match spends {
    Some(op) => vec![HyDbTxIn {
      vin_index: 0,
      prev_outpoint: Some(op),
      coinbase: false,
      script_sig: Vec::new(),
      sequence: 0xffff_ffff,
    }],
    None => vec![HyDbTxIn {
      vin_index: 0,
      prev_outpoint: None,
      coinbase: true,
      script_sig: vec![height as u8],
      sequence: 0xffff_ffff,
    }],
  };
  let tx = HyDbTx {
    txid,
    block_pos: 0,
    version: 1,
    tx_type: crate::types::HyDbTxType::Normal,
    lock_time: 0,
    size: 200,
    value_out: HyDbAmount(value),
    inputs,
    outputs: vec![HyDbTxOut {
      vout: 0,
      value: HyDbAmount(value),
      script_pubkey: Vec::new(),
      address: Some(HyDbAddress(address.to_owned())),
      address_type: HyDbAddrType::P2pkh,
    }],
    payload: None,
    validator_events: Vec::new(),
  };
  HyDbBlock {
    header: HyDbHeader {
      hash: hash_from_u64(0x2000_0000 + height as u64),
      prev_hash,
      merkle_root: txid,
      time: 1_600_000_000 + height,
      bits: 0x1e0f_ffff,
      difficulty: 1.0,
      nonce: 0,
      version: 1,
      size: 285,
      tx_count: 1,
      chainwork: None,
      coinbase: None,
    },
    txs: vec![tx],
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn hashes_are_distinct_per_seed() {
    assert_ne!(hash_from_u64(1), hash_from_u64(2));
  }

  #[test]
  fn genesis_block_is_coinbase_only() {
    let b = simple_block(0, HyDbHash([0u8; 32]), "addr", 50, None);
    assert_eq!(b.header.prev_hash, HyDbHash([0u8; 32]));
    assert!(b.txs[0].inputs[0].coinbase);
  }
}
