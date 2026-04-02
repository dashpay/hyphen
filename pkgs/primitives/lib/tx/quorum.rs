//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! LLMQ final commitment payload (type 6).

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::types::common::{BlsPublicKey, BlsSignature, QuorumHash, QuorumVvecHash};
use crate::types::sptx::{DynBitset, LlmqType};

/// DKG session output for one LLMQ.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CFinalCommitment {
  /// 1=legacy, 2=+indexed, 3=basic, 4=basic+idx.
  pub version: u16,
  pub llmq_type: LlmqType,
  pub quorum_hash: QuorumHash,
  /// Present for indexed versions (2, 4).
  pub quorum_index: Option<i16>,
  pub signers: DynBitset,
  pub valid_members: DynBitset,
  pub quorum_public_key: BlsPublicKey,
  pub quorum_vvec_hash: QuorumVvecHash,
  /// Threshold signature over commitment.
  pub quorum_sig: BlsSignature,
  /// Aggregated per-member signature.
  pub members_sig: BlsSignature,
}

impl CFinalCommitment {
  #[inline]
  pub fn is_indexed(&self) -> bool {
    self.version == 2 || self.version == 4
  }
}

impl Decode for CFinalCommitment {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = u16::consensus_decode(cursor)?;
    let llmq_type = LlmqType::from_u8(cursor.read_u8()?);
    let quorum_hash = QuorumHash::consensus_decode(cursor)?;

    let quorum_index = if version == 2 || version == 4 {
      Some(i16::consensus_decode(cursor)?)
    } else {
      None
    };

    let signers = DynBitset::consensus_decode(cursor)?;
    let valid_members = DynBitset::consensus_decode(cursor)?;
    let quorum_public_key = BlsPublicKey::consensus_decode(cursor)?;
    let quorum_vvec_hash = QuorumVvecHash::consensus_decode(cursor)?;
    let quorum_sig = BlsSignature::consensus_decode(cursor)?;
    let members_sig = BlsSignature::consensus_decode(cursor)?;

    Ok(Self {
      version,
      llmq_type,
      quorum_hash,
      quorum_index,
      signers,
      valid_members,
      quorum_public_key,
      quorum_vvec_hash,
      quorum_sig,
      members_sig,
    })
  }
}

/// Tx-level wrapper for CFinalCommitment (type 6).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CFinalCommitmentTxPayload {
  pub version: u16,
  pub height: u32,
  pub commitment: CFinalCommitment,
}

impl Decode for CFinalCommitmentTxPayload {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = u16::consensus_decode(cursor)?;
    let height = u32::consensus_decode(cursor)?;
    let commitment = CFinalCommitment::consensus_decode(cursor)?;

    Ok(Self {
      version,
      height,
      commitment,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::prelude::Vec;

  fn make_payload(commitment_version: u16, num_members: u8) -> Vec<u8> {
    let byte_len = ((num_members as usize) + 7) / 8;
    let mut buf = Vec::new();

    buf.extend_from_slice(&1u16.to_le_bytes()); // payload version
    buf.extend_from_slice(&1000u32.to_le_bytes()); // height

    buf.extend_from_slice(&commitment_version.to_le_bytes());
    buf.push(0x01); // llmq_type = LLMQ_50_60
    buf.extend_from_slice(&[0xAA; 32]); // quorum_hash

    if commitment_version == 2 || commitment_version == 4 {
      buf.extend_from_slice(&42i16.to_le_bytes());
    }

    // signers bitset
    buf.push(num_members);
    buf.extend(core::iter::repeat(0xFF).take(byte_len));

    // valid_members bitset
    buf.push(num_members);
    buf.extend(core::iter::repeat(0xAA).take(byte_len));

    buf.extend_from_slice(&[0xBB; 48]); // quorum_public_key
    buf.extend_from_slice(&[0xCC; 32]); // quorum_vvec_hash
    buf.extend_from_slice(&[0xDD; 96]); // quorum_sig
    buf.extend_from_slice(&[0xEE; 96]); // members_sig
    buf
  }

  #[test]
  fn decode_quorum_v1_non_indexed() {
    let data = make_payload(1, 50);
    let mut cur = Cursor::new(&data);
    let payload = CFinalCommitmentTxPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v1 non-indexed");
    assert_eq!(payload.version, 1);
    assert_eq!(payload.height, 1000);
    assert_eq!(payload.commitment.version, 1);
    assert_eq!(payload.commitment.llmq_type, LlmqType::Llmq50_60);
    assert_eq!(payload.commitment.quorum_hash, QuorumHash([0xAA; 32]));
    assert!(payload.commitment.quorum_index.is_none());
    assert_eq!(payload.commitment.signers.num_bits, 50);
    assert_eq!(payload.commitment.signers.data.len(), 7);
    assert_eq!(payload.commitment.valid_members.num_bits, 50);
    assert_eq!(
      payload.commitment.quorum_public_key,
      BlsPublicKey([0xBB; 48])
    );
    assert_eq!(
      payload.commitment.quorum_vvec_hash,
      QuorumVvecHash([0xCC; 32])
    );
    assert_eq!(payload.commitment.quorum_sig, BlsSignature([0xDD; 96]));
    assert_eq!(payload.commitment.members_sig, BlsSignature([0xEE; 96]));
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_quorum_v2_indexed() {
    let data = make_payload(2, 50);
    let mut cur = Cursor::new(&data);
    let payload = CFinalCommitmentTxPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v2 indexed");
    assert_eq!(payload.commitment.version, 2);
    assert_eq!(payload.commitment.quorum_index, Some(42));
    assert!(payload.commitment.is_indexed());
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_quorum_v3_basic_non_indexed() {
    let data = make_payload(3, 100);
    let mut cur = Cursor::new(&data);
    let payload = CFinalCommitmentTxPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v3 basic");
    assert_eq!(payload.commitment.version, 3);
    assert!(payload.commitment.quorum_index.is_none());
    assert!(!payload.commitment.is_indexed());
    assert_eq!(payload.commitment.signers.num_bits, 100);
    assert_eq!(payload.commitment.signers.data.len(), 13);
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_quorum_v4_basic_indexed() {
    let data = make_payload(4, 25);
    let mut cur = Cursor::new(&data);
    let payload = CFinalCommitmentTxPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v4 indexed");
    assert_eq!(payload.commitment.version, 4);
    assert_eq!(payload.commitment.quorum_index, Some(42));
    assert!(payload.commitment.is_indexed());
    assert_eq!(payload.commitment.signers.num_bits, 25);
    assert_eq!(payload.commitment.signers.data.len(), 4);
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_quorum_eof() {
    let data = make_payload(1, 50);
    let mut cur = Cursor::new(&data[..20]);
    assert!(CFinalCommitmentTxPayload::consensus_decode(&mut cur).is_err());
  }
}
