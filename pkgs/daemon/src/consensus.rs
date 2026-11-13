//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Local consensus helpers wrapping Base SDK primitives.

use dash_primitives::BlockHash;
use dash_types::codec::BaseCodec;
use dash_types::CompactSize;
use dash_types::Hashable;

use crate::error::DaemonError;

/// Compute the block hash from a raw block's 80-byte header.
pub fn block_hash(raw: &[u8]) -> Result<BlockHash, DaemonError> {
  if raw.len() < 80 {
    return Err(DaemonError::Index(format!(
      "block too short for header hash: {} bytes",
      raw.len()
    )));
  }
  let header: dash_primitives::BlockHeader = decode(&raw[..80])?;
  Ok(header.hash())
}

pub fn decode<T: BaseCodec>(raw: &[u8]) -> Result<T, DaemonError> {
  let mut cursor = raw;
  let value =
    T::decode(&mut cursor).map_err(|e| DaemonError::Index(format!("consensus decode: {e}")))?;
  if !cursor.is_empty() {
    return Err(DaemonError::Index(format!(
      "consensus decode: {} trailing bytes",
      cursor.len()
    )));
  }
  Ok(value)
}

pub fn txid(raw: &[u8]) -> Result<[u8; 32], DaemonError> {
  let tx: dash_primitives::Transaction = decode(raw)?;
  Ok(*tx.hash().as_bytes())
}

pub fn tx_ranges(
  raw: &[u8],
  txs: &[dash_primitives::Transaction],
) -> Result<Vec<(usize, usize)>, String> {
  if raw.len() < 80 {
    return Err("block is under 80 bytes".to_owned());
  }
  let mut count_bytes = Vec::new();
  CompactSize::from(txs.len()).encode(&mut count_bytes);
  let mut offset = 80 + count_bytes.len();
  if raw.get(80..offset) != Some(count_bytes.as_slice()) {
    return Err("non-canonical transaction count".to_owned());
  }
  let mut ranges = Vec::with_capacity(txs.len());
  for tx in txs {
    let size = dash_types::codec::encoded_size(tx);
    let end = offset
      .checked_add(size)
      .ok_or_else(|| "tx range overflow".to_owned())?;
    if end > raw.len() {
      return Err("transaction extends past block".to_owned());
    }
    let mut encoded = Vec::new();
    tx.encode(&mut encoded);
    if raw[offset..end] != encoded {
      return Err(format!("non-canonical transaction bytes at {offset}"));
    }
    ranges.push((offset, end));
    offset = end;
  }
  if offset != raw.len() {
    return Err(format!("{} trailing block bytes", raw.len() - offset));
  }
  Ok(ranges)
}

/// Read the parent object hash from a raw governance vote.
///
/// Returns display-order bytes suitable for RPC.
pub fn gov_vote_parent_hash(raw_vote: &[u8]) -> Result<[u8; 32], DaemonError> {
  let vote: dash_primitives::GovVote = decode(raw_vote)?;
  let mut hash = match vote {
    dash_primitives::GovVote::Bls(vote) => *vote.parent_hash().as_bytes(),
    dash_primitives::GovVote::Ecdsa(vote) => *vote.parent_hash().as_bytes(),
  };
  hash.reverse();
  Ok(hash)
}

/// Compute the canonical governance object hash from a raw governance object.
///
/// Returns display-order bytes suitable for RPC.
///
/// Delegates to `GovObject::hash()` which includes the full outpoint
/// serialization, then reverses to display order.
pub fn gov_object_hash(raw_object: &[u8]) -> Result<[u8; 32], DaemonError> {
  let obj: dash_primitives::GovObject = decode(raw_object)?;
  let mut hash = obj.hash().as_bytes().to_owned();
  hash.reverse();
  Ok(hash)
}

/// Convert compact "nBits" target encoding to approximate difficulty.
pub fn bits_to_difficulty(bits: u32) -> f64 {
  let exponent = (bits >> 24) & 0xFF;
  let mantissa = bits & 0x007F_FFFF;
  if mantissa == 0 {
    return 0.0;
  }
  let genesis_target = 0x00FFFF_u64 as f64 * 256.0_f64.powi(0x1d - 3);
  let target = mantissa as f64 * 256.0_f64.powi(exponent as i32 - 3);
  genesis_target / target
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn block_hash_rejects_short_input() {
    assert!(block_hash(&[0u8; 79]).is_err());
  }

  #[test]
  fn block_hash_accepts_80_bytes() {
    assert!(block_hash(&[0u8; 80]).is_ok());
  }

  #[test]
  fn bits_to_difficulty_handles_genesis_target() {
    let diff = bits_to_difficulty(0x1d00ffff);
    assert!((diff - 1.0).abs() < 1e-6);
  }

  #[test]
  fn bits_to_difficulty_handles_zero_mantissa() {
    assert_eq!(bits_to_difficulty(0x1d000000), 0.0);
  }
  #[test]
  fn v2_asset_unlock_uses_stable_txid_hash() {
    let raw = hex::decode("03000900000190cff40500000000232102472c66bc4fd71f30167ac9b8888944ca749dcb0a130329ab95380be0a43c25b0ac000000009102210300000000000070110100c80300002a3631100ae30009d1bf863e5db641069985121ad4ab37dfda0800a65cc1116088250d04fb32aef4219a552f35627d46a9f448a742564cb9e7c4b107325e1f875f64a0c90d517b137cb44d359e401dce015a077ec33ba4c078e4b74f252181943daa2cc30eb90b693e3aee521016f22ddecc52b0e4c79781b76c8def1c66db46").expect("vector");
    let tx: dash_primitives::Transaction = decode(&raw).expect("transaction");
    assert_ne!(tx.hash().as_bytes(), tx.instance_hash().as_bytes());
    assert_eq!(
      hex::encode(
        crate::consensus::txid(&raw)
          .expect("transaction txid")
          .iter()
          .rev()
          .copied()
          .collect::<Vec<_>>()
      ),
      "0f9db35643e5ebca413fe5be12144185adb7236f051dbcb872162c14cea2523a"
    );
  }
}
