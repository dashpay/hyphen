//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Utility helpers for chain-derived value formatting.

use crate::prelude::{String, Vec};
use crate::types::common::KeyId;

const HASH160_LEN: usize = 20;

fn encode_base58_check(prefix: u8, hash160: &[u8]) -> Option<String> {
  if hash160.len() != HASH160_LEN {
    return None;
  }
  let mut payload = Vec::with_capacity(HASH160_LEN + 1);
  payload.push(prefix);
  payload.extend_from_slice(hash160);
  Some(bs58::encode(payload).with_check().into_string())
}

/// Encode a 20-byte public-key hash as a Base58Check P2PKH address.
pub fn encode_p2pkh_hash160(hash160: &[u8], prefix: u8) -> Option<String> {
  encode_base58_check(prefix, hash160)
}

/// Encode a Dash `KeyId` as a Base58Check P2PKH address.
pub fn encode_key_id_address(key_id: &KeyId, prefix: u8) -> String {
  debug_assert_eq!(key_id.as_bytes().len(), HASH160_LEN);
  let mut payload = Vec::with_capacity(HASH160_LEN + 1);
  payload.push(prefix);
  payload.extend_from_slice(key_id.as_bytes());
  bs58::encode(payload).with_check().into_string()
}

/// Encode a 20-byte script hash as a Base58Check P2SH address.
pub fn encode_p2sh_hash160(hash160: &[u8], prefix: u8) -> Option<String> {
  encode_base58_check(prefix, hash160)
}

#[cfg(test)]
mod tests {
  use super::{encode_key_id_address, encode_p2pkh_hash160, encode_p2sh_hash160};
  use crate::types::common::KeyId;

  #[test]
  fn encode_key_id_address_uses_supplied_prefix() {
    let key_id = KeyId([0x42; 20]);
    let address = encode_key_id_address(&key_id, 76);
    assert_eq!(address, "XgjBof7tWXH8jNXY4rRb1rdNUmDK3aNNNK");
  }

  #[test]
  fn encode_p2pkh_hash160_rejects_wrong_length() {
    let address = encode_p2pkh_hash160(&[0x42; 19], 76);
    assert_eq!(address, None);
  }

  #[test]
  fn encode_p2sh_hash160_uses_supplied_prefix() {
    let address = encode_p2sh_hash160(&[0x24; 20], 16);
    assert_eq!(
      address,
      Some("7VhkNn2LJ9YE35ZGbWkfPjKisrCFT7ovqy".to_owned())
    );
  }
}
