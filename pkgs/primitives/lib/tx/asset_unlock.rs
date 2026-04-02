//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CAssetUnlockPayload (type 9): Platform to L1.

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::types::common::{BlsSignature, QuorumHash};

/// CAssetUnlockPayload: Platform-to-L1 (type 9).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CAssetUnlockPayload {
  pub version: u8,
  /// Monotonic withdrawal sequence number.
  pub index: u64,
  /// Duffs deducted from withdrawal.
  pub fee: u32,
  pub requested_height: u32,
  pub quorum_hash: QuorumHash,
  /// Quorum BLS authorization signature.
  pub quorum_sig: BlsSignature,
}

impl Decode for CAssetUnlockPayload {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = u8::consensus_decode(cursor)?;
    let index = u64::consensus_decode(cursor)?;
    let fee = u32::consensus_decode(cursor)?;
    let requested_height = u32::consensus_decode(cursor)?;
    let quorum_hash = QuorumHash::consensus_decode(cursor)?;
    let quorum_sig = BlsSignature::consensus_decode(cursor)?;

    Ok(Self {
      version,
      index,
      fee,
      requested_height,
      quorum_hash,
      quorum_sig,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::prelude::Vec;

  fn make_payload(version: u8, index: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(version);
    buf.extend_from_slice(&index.to_le_bytes());
    buf.extend_from_slice(&500u32.to_le_bytes()); // fee
    buf.extend_from_slice(&1000u32.to_le_bytes()); // requested_height
    buf.extend_from_slice(&[0xAA; 32]); // quorum_hash
    buf.extend_from_slice(&[0xBB; 96]); // quorum_sig
    buf
  }

  #[test]
  fn decode_asset_unlock_v1() {
    let data = make_payload(1, 42);
    let mut cur = Cursor::new(&data);
    let payload = CAssetUnlockPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v1");
    assert_eq!(payload.version, 1);
    assert_eq!(payload.index, 42);
    assert_eq!(payload.fee, 500);
    assert_eq!(payload.requested_height, 1000);
    assert_eq!(payload.quorum_hash, QuorumHash([0xAA; 32]));
    assert_eq!(payload.quorum_sig, BlsSignature([0xBB; 96]));
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_asset_unlock_large_index() {
    let data = make_payload(1, u64::MAX);
    let mut cur = Cursor::new(&data);
    let payload = CAssetUnlockPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode large index");
    assert_eq!(payload.index, u64::MAX);
  }

  #[test]
  fn decode_asset_unlock_eof() {
    let data = make_payload(1, 1);
    let mut cur = Cursor::new(&data[..10]);
    assert!(CAssetUnlockPayload::consensus_decode(&mut cur).is_err());
  }
}
