//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CProUpRevTx revocation payload (type 4).

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::types::common::{BlsSignature, InputsHash, TxHash};

/// Revocation reason code.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum RevocationReason {
  NotSpecified,
  KeyCompromise,
  ChangeOfKeys,
  ViolationOfService,
  /// Forward-compat catch-all.
  Unknown(u16),
}

impl RevocationReason {
  #[inline]
  pub fn from_u16(val: u16) -> Self {
    match val {
      0 => Self::NotSpecified,
      1 => Self::KeyCompromise,
      2 => Self::ChangeOfKeys,
      3 => Self::ViolationOfService,
      other => Self::Unknown(other),
    }
  }
}

/// CProUpRevTx -- revoke a masternode (type 4).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CProUpRevTx {
  /// 1=LegacyBLS, 2=BasicBLS.
  pub version: u16,
  pub pro_tx_hash: TxHash,
  pub reason: RevocationReason,
  pub inputs_hash: InputsHash,
  /// Operator BLS signature.
  pub sig: BlsSignature,
}

impl Decode for CProUpRevTx {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = u16::consensus_decode(cursor)?;
    let pro_tx_hash = TxHash::consensus_decode(cursor)?;
    let reason_raw = u16::consensus_decode(cursor)?;
    let reason = RevocationReason::from_u16(reason_raw);
    let inputs_hash = InputsHash::consensus_decode(cursor)?;
    let sig = BlsSignature::consensus_decode(cursor)?;

    Ok(Self {
      version,
      pro_tx_hash,
      reason,
      inputs_hash,
      sig,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::prelude::Vec;

  fn make_payload(version: u16, reason: u16) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&version.to_le_bytes());
    buf.extend_from_slice(&[0x11; 32]); // pro_tx_hash
    buf.extend_from_slice(&reason.to_le_bytes());
    buf.extend_from_slice(&[0x22; 32]); // inputs_hash
    buf.extend_from_slice(&[0x33; 96]); // sig
    buf
  }

  #[test]
  fn decode_prouprevtx_v1() {
    let data = make_payload(1, 0);
    let mut cur = Cursor::new(&data);
    let payload = CProUpRevTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v1");
    assert_eq!(payload.version, 1);
    assert_eq!(payload.pro_tx_hash, TxHash([0x11; 32]));
    assert_eq!(payload.reason, RevocationReason::NotSpecified);
    assert_eq!(payload.inputs_hash, InputsHash([0x22; 32]));
    assert_eq!(payload.sig, BlsSignature([0x33; 96]));
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_prouprevtx_v2_key_compromise() {
    let data = make_payload(2, 1);
    let mut cur = Cursor::new(&data);
    let payload = CProUpRevTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v2");
    assert_eq!(payload.version, 2);
    assert_eq!(payload.reason, RevocationReason::KeyCompromise);
  }

  #[test]
  fn decode_prouprevtx_unknown_reason() {
    let data = make_payload(1, 99);
    let mut cur = Cursor::new(&data);
    let payload = CProUpRevTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode unknown reason");
    assert_eq!(payload.reason, RevocationReason::Unknown(99));
  }

  #[test]
  fn decode_prouprevtx_eof() {
    let data = make_payload(1, 0);
    let mut cur = Cursor::new(&data[..10]);
    assert!(CProUpRevTx::consensus_decode(&mut cur).is_err());
  }
}
