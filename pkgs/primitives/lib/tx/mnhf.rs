//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! MNHFTxPayload hard-fork signal (type 7).

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::types::common::{BlsSignature, QuorumHash};

/// MNHFTxPayload -- hard-fork signal (type 7).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct MNHFTxPayload {
  pub version: u8,
  /// Feature bit being activated.
  pub version_bit: u8,
  pub quorum_hash: QuorumHash,
  /// Quorum BLS signature.
  pub sig: BlsSignature,
}

impl Decode for MNHFTxPayload {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = u8::consensus_decode(cursor)?;
    let version_bit = u8::consensus_decode(cursor)?;
    let quorum_hash = QuorumHash::consensus_decode(cursor)?;
    let sig = BlsSignature::consensus_decode(cursor)?;

    Ok(Self {
      version,
      version_bit,
      quorum_hash,
      sig,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::prelude::Vec;

  fn make_payload(version: u8, version_bit: u8) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(version);
    buf.push(version_bit);
    buf.extend_from_slice(&[0xAA; 32]); // quorum_hash
    buf.extend_from_slice(&[0xBB; 96]); // sig
    buf
  }

  #[test]
  fn decode_mnhf_v1() {
    let data = make_payload(1, 5);
    let mut cur = Cursor::new(&data);
    let payload = MNHFTxPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v1");
    assert_eq!(payload.version, 1);
    assert_eq!(payload.version_bit, 5);
    assert_eq!(payload.quorum_hash, QuorumHash([0xAA; 32]));
    assert_eq!(payload.sig, BlsSignature([0xBB; 96]));
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_mnhf_version_bit_zero() {
    let data = make_payload(1, 0);
    let mut cur = Cursor::new(&data);
    let payload = MNHFTxPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode version_bit 0");
    assert_eq!(payload.version_bit, 0);
  }

  #[test]
  fn decode_mnhf_eof() {
    let data = make_payload(1, 3);
    let mut cur = Cursor::new(&data[..10]);
    assert!(MNHFTxPayload::consensus_decode(&mut cur).is_err());
  }
}
