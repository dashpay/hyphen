//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CProUpRegTx registrar-update payload (type 3).

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::prelude::Vec;
use crate::types::common::{BlsPublicKey, InputsHash, KeyId, Script, TxHash};

const MAX_VCH_SIG_SIZE: usize = 256;

/// CProUpRegTx -- update MN keys/payout (type 3).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CProUpRegTx {
  /// 1=LegacyBLS, 2=BasicBLS.
  pub version: u16,
  pub pro_tx_hash: TxHash,
  /// Reserved, always 0.
  pub mode: u16,
  pub pub_key_operator: BlsPublicKey,
  pub key_id_voting: KeyId,
  pub script_payout: Script,
  pub inputs_hash: InputsHash,
  /// Owner ECDSA sig (variable-length).
  pub vch_sig: Vec<u8>,
}

impl Decode for CProUpRegTx {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = u16::consensus_decode(cursor)?;
    let pro_tx_hash = TxHash::consensus_decode(cursor)?;
    let mode = u16::consensus_decode(cursor)?;
    let pub_key_operator = BlsPublicKey::consensus_decode(cursor)?;
    let key_id_voting = KeyId::consensus_decode(cursor)?;
    let script_payout = Script::consensus_decode(cursor)?;
    let inputs_hash = InputsHash::consensus_decode(cursor)?;
    let vch_sig = cursor.read_compact_bytes(MAX_VCH_SIG_SIZE)?;

    Ok(Self {
      version,
      pro_tx_hash,
      mode,
      pub_key_operator,
      key_id_voting,
      script_payout,
      inputs_hash,
      vch_sig,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::prelude::Vec;

  fn make_payload(version: u16) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&version.to_le_bytes());
    buf.extend_from_slice(&[0x11; 32]); // pro_tx_hash
    buf.extend_from_slice(&0u16.to_le_bytes()); // mode = 0
    buf.extend_from_slice(&[0x22; 48]); // pub_key_operator
    buf.extend_from_slice(&[0x33; 20]); // key_id_voting
                                        // script_payout: CS(3) + 3 bytes
    buf.push(0x03);
    buf.extend_from_slice(&[0x76, 0xA9, 0x14]);
    buf.extend_from_slice(&[0x44; 32]); // inputs_hash
                                        // vch_sig: CS(3) + 3 bytes
    buf.push(0x03);
    buf.extend_from_slice(&[0xDE, 0xAD, 0xBE]);
    buf
  }

  #[test]
  fn decode_proupregtx_v1() {
    let data = make_payload(1);
    let mut cur = Cursor::new(&data);
    let payload = CProUpRegTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v1");
    assert_eq!(payload.version, 1);
    assert_eq!(payload.pro_tx_hash, TxHash([0x11; 32]));
    assert_eq!(payload.mode, 0);
    assert_eq!(payload.pub_key_operator, BlsPublicKey([0x22; 48]));
    assert_eq!(payload.key_id_voting, KeyId([0x33; 20]));
    assert_eq!(payload.script_payout.as_bytes(), &[0x76, 0xA9, 0x14]);
    assert_eq!(payload.inputs_hash, InputsHash([0x44; 32]));
    assert_eq!(payload.vch_sig, &[0xDE, 0xAD, 0xBE]);
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_proupregtx_v2() {
    let data = make_payload(2);
    let mut cur = Cursor::new(&data);
    let payload = CProUpRegTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v2");
    assert_eq!(payload.version, 2);
    assert_eq!(payload.pub_key_operator, BlsPublicKey([0x22; 48]));
  }

  #[test]
  fn decode_proupregtx_empty_sig() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1u16.to_le_bytes());
    buf.extend_from_slice(&[0x11; 32]); // pro_tx_hash
    buf.extend_from_slice(&0u16.to_le_bytes()); // mode
    buf.extend_from_slice(&[0x22; 48]); // pub_key_operator
    buf.extend_from_slice(&[0x33; 20]); // key_id_voting
    buf.push(0x00); // empty script_payout
    buf.extend_from_slice(&[0x44; 32]); // inputs_hash
    buf.push(0x00); // empty vch_sig

    let mut cur = Cursor::new(&buf);
    let payload = CProUpRegTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode empty sig");
    assert!(payload.vch_sig.is_empty());
    assert!(payload.script_payout.is_empty());
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_proupregtx_eof() {
    let data = make_payload(1);
    let mut cur = Cursor::new(&data[..10]);
    assert!(CProUpRegTx::consensus_decode(&mut cur).is_err());
  }
}
