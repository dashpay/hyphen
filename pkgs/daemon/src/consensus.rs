//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Local consensus helpers wrapping Base SDK primitives.

use dash_primitives::BlockHash;

use crate::error::DaemonError;

/// Compute the block hash from a raw block's 80-byte header.
pub fn block_hash(raw: &[u8]) -> Result<BlockHash, DaemonError> {
  if raw.len() < 80 {
    return Err(DaemonError::Index(format!(
      "block too short for header hash: {} bytes",
      raw.len()
    )));
  }
  let hash = dash_pow::hash(&raw[..80]);
  Ok(BlockHash::from(hash))
}

/// Read the parent object hash from a raw governance vote.
///
/// Returns display-order bytes suitable for RPC.
pub fn gov_vote_parent_hash(raw_vote: &[u8]) -> Result<[u8; 32], DaemonError> {
  let vote = dash_primitives::gov::GovVote::decode(raw_vote)
    .map_err(|e| DaemonError::Index(format!("governance vote decode: {e}")))?;
  let mut hash = vote.parent_hash.to_bytes();
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
  let obj = dash_primitives::gov::GovObject::decode(raw_object)
    .map_err(|e| DaemonError::Index(format!("governance object decode: {e}")))?;
  let mut hash = obj.hash().to_bytes();
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
}
