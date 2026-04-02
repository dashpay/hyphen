//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CAssetLockPayload (type 8): L1 to Platform.

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::prelude::Vec;
use crate::types::tx::{decode_vec, TxOut};

const MAX_CREDIT_OUTPUTS: usize = 100;

/// CAssetLockPayload: L1-to-Platform (type 8).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CAssetLockPayload {
  pub version: u8,
  /// Platform credit allocations.
  pub credit_outputs: Vec<TxOut>,
}

impl Decode for CAssetLockPayload {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = u8::consensus_decode(cursor)?;
    let credit_outputs = decode_vec::<TxOut>(cursor, MAX_CREDIT_OUTPUTS)?;

    Ok(Self {
      version,
      credit_outputs,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::types::common::Script;

  fn make_payload(num_outputs: u8) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(1); // version = 1
    buf.push(num_outputs);
    for i in 0..num_outputs {
      buf.extend_from_slice(&(1000i64 * i64::from(i + 1)).to_le_bytes());
      buf.push(0x03);
      buf.extend_from_slice(&[0x76, 0xA9, 0x14]);
    }
    buf
  }

  #[test]
  fn decode_asset_lock_single_output() {
    let data = make_payload(1);
    let mut cur = Cursor::new(&data);
    let payload = CAssetLockPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode single output");
    assert_eq!(payload.version, 1);
    assert_eq!(payload.credit_outputs.len(), 1);
    assert_eq!(payload.credit_outputs[0].value, 1000);
    assert_eq!(
      payload.credit_outputs[0].script_pubkey,
      Script(crate::prelude::vec![0x76, 0xA9, 0x14])
    );
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_asset_lock_multiple_outputs() {
    let data = make_payload(3);
    let mut cur = Cursor::new(&data);
    let payload = CAssetLockPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode multiple outputs");
    assert_eq!(payload.credit_outputs.len(), 3);
    assert_eq!(payload.credit_outputs[0].value, 1000);
    assert_eq!(payload.credit_outputs[1].value, 2000);
    assert_eq!(payload.credit_outputs[2].value, 3000);
  }

  #[test]
  fn decode_asset_lock_empty_outputs() {
    let data = [1u8, 0x00];
    let mut cur = Cursor::new(&data);
    let payload = CAssetLockPayload::consensus_decode(&mut cur);
    let payload = payload.expect("should decode empty outputs");
    assert!(payload.credit_outputs.is_empty());
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_asset_lock_eof() {
    let data = make_payload(2);
    let mut cur = Cursor::new(&data[..5]);
    assert!(CAssetLockPayload::consensus_decode(&mut cur).is_err());
  }
}
