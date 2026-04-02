//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Full transaction decoding.

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::{DecodeError, TxError};
use crate::prelude::Vec;
use crate::types::tx::{decode_transaction, TxIn, TxOut, TxType};

/// Decoded transaction. Type code packed in
/// upper 16 bits of the wire version field.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Transaction {
  /// Lower 16 bits of nVersion.
  pub version: i16,
  /// Upper 16 bits of nVersion.
  pub tx_type: TxType,
  pub inputs: Vec<TxIn>,
  pub outputs: Vec<TxOut>,
  pub lock_time: u32,
  /// Raw special-tx payload (empty if Normal).
  pub extra_payload: Vec<u8>,
}

impl Transaction {
  pub fn consensus_decode_tx(cursor: &mut Cursor<'_>) -> Result<Self, TxError> {
    let (version, tx_type, inputs, outputs, lock_time, extra_payload) = decode_transaction(cursor)?;
    Ok(Self {
      version,
      tx_type,
      inputs,
      outputs,
      lock_time,
      extra_payload,
    })
  }

  /// Decode one raw serialized transaction and reject trailing bytes.
  pub fn from_bytes(raw: &[u8]) -> Result<Self, TxError> {
    let mut cursor = Cursor::new(raw);
    let tx = Self::consensus_decode_tx(&mut cursor)?;
    if !cursor.is_empty() {
      return Err(TxError::Decode(DecodeError::InvalidValue {
        field: "transaction",
        reason: "trailing bytes after transaction decode",
      }));
    }
    Ok(tx)
  }

  /// Decode extra_payload into a typed struct.
  pub fn payload<T: Decode>(&self) -> Result<T, DecodeError> {
    let mut cursor = Cursor::new(&self.extra_payload);
    T::consensus_decode(&mut cursor)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::prelude::vec;

  fn make_normal_tx() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1i32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&[0x00; 32]);
    buf.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&0i64.to_le_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
  }

  fn make_special_tx() -> Vec<u8> {
    let mut buf = Vec::new();
    let n_version: i32 = 3 | (5 << 16);
    buf.extend_from_slice(&n_version.to_le_bytes());
    buf.push(0x00);
    buf.push(0x00);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.push(0x04);
    buf.extend_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    buf
  }

  #[test]
  fn decode_normal_transaction() {
    let data = make_normal_tx();
    let mut cur = Cursor::new(&data);
    let tx = Transaction::consensus_decode_tx(&mut cur);
    let tx = tx.expect("should decode normal tx");
    assert_eq!(tx.version, 1);
    assert_eq!(tx.tx_type, TxType::Normal);
    assert_eq!(tx.inputs.len(), 1);
    assert_eq!(tx.outputs.len(), 1);
    assert_eq!(tx.lock_time, 0);
    assert!(tx.extra_payload.is_empty());
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_special_transaction_with_payload() {
    let data = make_special_tx();
    let mut cur = Cursor::new(&data);
    let tx = Transaction::consensus_decode_tx(&mut cur);
    let tx = tx.expect("should decode special tx");
    assert_eq!(tx.version, 3);
    assert_eq!(tx.tx_type, TxType::Coinbase);
    assert_eq!(tx.extra_payload, vec![0xDE, 0xAD, 0xBE, 0xEF]);
    assert!(cur.is_empty());
  }

  #[test]
  fn version_packing_round_trip() {
    let n_version: i32 = 3 | (9 << 16);
    let version = (n_version & 0xFFFF) as i16;
    #[expect(clippy::cast_sign_loss)]
    let tx_type = TxType::from_u16(((n_version >> 16) & 0xFFFF) as u16);
    assert_eq!(version, 3);
    assert_eq!(tx_type, TxType::AssetUnlock);
  }

  #[test]
  fn normal_tx_version_3_no_payload() {
    let mut buf = Vec::new();
    let n_version: i32 = 3;
    buf.extend_from_slice(&n_version.to_le_bytes());
    buf.push(0x00);
    buf.push(0x00);
    buf.extend_from_slice(&0u32.to_le_bytes());

    let mut cur = Cursor::new(&buf);
    let tx = Transaction::consensus_decode_tx(&mut cur);
    let tx = tx.expect("should decode");
    assert_eq!(tx.version, 3);
    assert_eq!(tx.tx_type, TxType::Normal);
    assert!(tx.extra_payload.is_empty());
  }
}
