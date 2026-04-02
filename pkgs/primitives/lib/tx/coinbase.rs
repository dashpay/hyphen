//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CCbTx coinbase payload (type 5).

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::types::common::{BlsSignature, MerkleRoot};

/// CCbTx -- coinbase commitment payload (type 5).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CCbTx {
  /// 1=base, 2=+quorums, 3=+ChainLock.
  pub version: u16,
  pub height: u32,
  pub merkle_root_mn_list: MerkleRoot,
  /// v2+.
  pub merkle_root_quorums: Option<MerkleRoot>,
  /// v3+.
  pub best_cl_height_diff: Option<u64>,
  /// v3+.
  pub best_cl_signature: Option<BlsSignature>,
  /// Duffs, v3+.
  pub credit_pool_balance: Option<i64>,
}

impl Decode for CCbTx {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = u16::consensus_decode(cursor)?;
    let height = u32::consensus_decode(cursor)?;
    let merkle_root_mn_list = MerkleRoot::consensus_decode(cursor)?;

    let merkle_root_quorums = if version >= 2 {
      Some(MerkleRoot::consensus_decode(cursor)?)
    } else {
      None
    };

    let (best_cl_height_diff, best_cl_signature, credit_pool_balance) = if version >= 3 {
      let diff = cursor.read_compact_size()?;
      let sig = BlsSignature::consensus_decode(cursor)?;
      let balance = i64::consensus_decode(cursor)?;
      (Some(diff), Some(sig), Some(balance))
    } else {
      (None, None, None)
    };

    Ok(Self {
      version,
      height,
      merkle_root_mn_list,
      merkle_root_quorums,
      best_cl_height_diff,
      best_cl_signature,
      credit_pool_balance,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::prelude::Vec;

  fn make_v1_payload() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&100u32.to_le_bytes());
    buf.extend_from_slice(&[0xAA; 32]);
    buf
  }

  fn make_v2_payload() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&200u32.to_le_bytes());
    buf.extend_from_slice(&[0xAA; 32]);
    buf.extend_from_slice(&[0xBB; 32]);
    buf
  }

  fn make_v3_payload() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&3u16.to_le_bytes());
    buf.extend_from_slice(&300u32.to_le_bytes());
    buf.extend_from_slice(&[0xAA; 32]);
    buf.extend_from_slice(&[0xBB; 32]);
    buf.push(0x05);
    buf.extend_from_slice(&[0xCC; 96]);
    buf.extend_from_slice(&1_000_000i64.to_le_bytes());
    buf
  }

  #[test]
  fn decode_ccbtx_v1() {
    let data = make_v1_payload();
    let mut cur = Cursor::new(&data);
    let payload = CCbTx::consensus_decode(&mut cur).expect("should decode v1");
    assert_eq!(payload.version, 1);
    assert_eq!(payload.height, 100);
    assert_eq!(payload.merkle_root_mn_list, MerkleRoot([0xAA; 32]));
    assert!(payload.merkle_root_quorums.is_none());
    assert!(payload.best_cl_height_diff.is_none());
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_ccbtx_v2() {
    let data = make_v2_payload();
    let mut cur = Cursor::new(&data);
    let payload = CCbTx::consensus_decode(&mut cur).expect("should decode v2");
    assert_eq!(payload.version, 2);
    assert_eq!(payload.merkle_root_quorums, Some(MerkleRoot([0xBB; 32])));
    assert!(payload.best_cl_height_diff.is_none());
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_ccbtx_v3() {
    let data = make_v3_payload();
    let mut cur = Cursor::new(&data);
    let payload = CCbTx::consensus_decode(&mut cur).expect("should decode v3");
    assert_eq!(payload.version, 3);
    assert_eq!(payload.best_cl_height_diff, Some(5));
    assert_eq!(payload.best_cl_signature, Some(BlsSignature([0xCC; 96])));
    assert_eq!(payload.credit_pool_balance, Some(1_000_000));
    assert!(cur.is_empty());
  }
}
