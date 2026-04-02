//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Script classification and address derivation.

use ripemd::Ripemd160;
use sha2::{Digest, Sha256};

const HASH160_LEN: usize = 20;
const P2PKH_SCRIPT_LEN: usize = 25;
const P2SH_SCRIPT_LEN: usize = 23;
const P2PK_COMPRESSED_SCRIPT_LEN: usize = 35;
const P2PK_UNCOMPRESSED_SCRIPT_LEN: usize = 67;
const P2PK_COMPRESSED_KEY_LEN: usize = 33;
const P2PK_UNCOMPRESSED_KEY_LEN: usize = 65;
const OP_DUP: u8 = 0x76;
const OP_HASH160: u8 = 0xa9;
const OP_PUSH_HASH160: u8 = 0x14;
const OP_EQUALVERIFY: u8 = 0x88;
const OP_EQUAL: u8 = 0x87;
const OP_CHECKSIG: u8 = 0xac;
const OP_RETURN: u8 = 0x6a;
const OP_PUSH_COMPRESSED_PUBKEY: u8 = 0x21;
const OP_PUSH_UNCOMPRESSED_PUBKEY: u8 = 0x41;

/// Known output script patterns that the daemon exposes publicly.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ScriptKind {
  /// Pay-to-public-key-hash.
  P2pkh,
  /// Pay-to-script-hash.
  P2sh,
  /// Pay-to-public-key.
  P2pk,
  /// Provably unspendable `OP_RETURN`.
  OpReturn,
  /// Unrecognized or unsupported script.
  Unknown,
}

/// Classify a scriptPubKey by its leading pattern.
pub fn classify_script(script: &[u8]) -> ScriptKind {
  let len = script.len();
  if len == P2PKH_SCRIPT_LEN
    && script[0] == OP_DUP
    && script[1] == OP_HASH160
    && script[2] == OP_PUSH_HASH160
    && script[P2PKH_SCRIPT_LEN - 2] == OP_EQUALVERIFY
    && script[P2PKH_SCRIPT_LEN - 1] == OP_CHECKSIG
  {
    return ScriptKind::P2pkh;
  }

  if len == P2SH_SCRIPT_LEN
    && script[0] == OP_HASH160
    && script[1] == OP_PUSH_HASH160
    && script[P2SH_SCRIPT_LEN - 1] == OP_EQUAL
  {
    return ScriptKind::P2sh;
  }

  if (len == P2PK_COMPRESSED_SCRIPT_LEN
    && script[0] == OP_PUSH_COMPRESSED_PUBKEY
    && script[P2PK_COMPRESSED_SCRIPT_LEN - 1] == OP_CHECKSIG)
    || (len == P2PK_UNCOMPRESSED_SCRIPT_LEN
      && script[0] == OP_PUSH_UNCOMPRESSED_PUBKEY
      && script[P2PK_UNCOMPRESSED_SCRIPT_LEN - 1] == OP_CHECKSIG)
  {
    return ScriptKind::P2pk;
  }

  if !script.is_empty() && script[0] == OP_RETURN {
    return ScriptKind::OpReturn;
  }

  ScriptKind::Unknown
}

/// Derive a Base58Check address from a scriptPubKey.
pub fn derive_address(script: &[u8], p2pkh_version: u8, p2sh_version: u8) -> Option<String> {
  match classify_script(script) {
    ScriptKind::P2pkh => {
      crate::util::encode_p2pkh_hash160(&script[3..(3 + HASH160_LEN)], p2pkh_version)
    }
    ScriptKind::P2sh => {
      crate::util::encode_p2sh_hash160(&script[2..(2 + HASH160_LEN)], p2sh_version)
    }
    ScriptKind::P2pk => {
      let pubkey = if script.len() == P2PK_COMPRESSED_SCRIPT_LEN {
        &script[1..(1 + P2PK_COMPRESSED_KEY_LEN)]
      } else {
        &script[1..(1 + P2PK_UNCOMPRESSED_KEY_LEN)]
      };
      let sha = Sha256::digest(pubkey);
      let hash160 = Ripemd160::digest(sha);
      crate::util::encode_p2pkh_hash160(hash160.as_slice(), p2pkh_version)
    }
    ScriptKind::OpReturn | ScriptKind::Unknown => None,
  }
}

#[cfg(test)]
mod tests {
  use super::{classify_script, derive_address, ScriptKind};

  #[test]
  fn classifies_p2pkh_script() {
    let script = [
      0x76, 0xa9, 0x14, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
      0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x88, 0xac,
    ];
    assert_eq!(classify_script(&script), ScriptKind::P2pkh);
  }

  #[test]
  fn derives_dash_p2sh_address() {
    let script = [
      0xa9, 0x14, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24,
      0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x24, 0x87,
    ];
    assert_eq!(
      derive_address(&script, 76, 16),
      Some("7VhkNn2LJ9YE35ZGbWkfPjKisrCFT7ovqy".to_owned())
    );
  }
}
