//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Fixed-size byte newtypes for hashes, keys,
//! and scripts.

use core::fmt;
use core::hash::{Hash, Hasher};

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;

/// Newtype over `[u8; N]` with hex Debug
/// and consensus Decode.
macro_rules! fixed_bytes_newtype {
  ($(#[$meta:meta])* $name:ident, $size:expr) => {
    $(#[$meta])*
    #[derive(Clone, Copy, PartialEq, Eq)]
    pub struct $name(pub [u8; $size]);

    impl fmt::Debug for $name {
      fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}(", stringify!($name))?;
        for byte in &self.0 {
          write!(f, "{byte:02x}")?;
        }
        write!(f, ")")
      }
    }

    impl Hash for $name {
      fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.hash(state);
      }
    }

    impl Decode for $name {
      #[inline]
      fn consensus_decode(
        cursor: &mut Cursor<'_>,
      ) -> Result<Self, DecodeError> {
        Ok(Self(cursor.read_array::<$size>()?))
      }
    }

    impl From<[u8; $size]> for $name {
      #[inline]
      fn from(bytes: [u8; $size]) -> Self {
        Self(bytes)
      }
    }

    impl $name {
      #[inline]
      pub fn as_bytes(&self) -> &[u8] {
        &self.0
      }
    }

    #[cfg(feature = "serde")]
    impl serde::Serialize for $name {
      fn serialize<S: serde::Serializer>(
        &self,
        serializer: S,
      ) -> Result<S::Ok, S::Error> {
        if serializer.is_human_readable() {
          use core::fmt::Write;
          let mut hex =
            crate::prelude::String::with_capacity($size * 2);
          for byte in &self.0 {
            let _ = write!(hex, "{byte:02x}");
          }
          serializer.serialize_str(&hex)
        } else {
          serializer.serialize_bytes(&self.0)
        }
      }
    }
  };
}

fixed_bytes_newtype!(
  /// 32-byte X11 block header hash.
  BlockHash,
  32
);

fixed_bytes_newtype!(
  /// 32-byte double-SHA256 transaction hash.
  TxHash,
  32
);

fixed_bytes_newtype!(
  /// 32-byte Merkle tree root hash.
  MerkleRoot,
  32
);

fixed_bytes_newtype!(
  /// 32-byte LE unsigned 256-bit integer.
  Uint256,
  32
);

fixed_bytes_newtype!(
  /// 32-byte hash of serialized tx inputs.
  InputsHash,
  32
);

fixed_bytes_newtype!(
  /// 32-byte LLMQ quorum identifier hash.
  QuorumHash,
  32
);

fixed_bytes_newtype!(
  /// 32-byte quorum verification vector hash.
  QuorumVvecHash,
  32
);

fixed_bytes_newtype!(
  /// 20-byte public key hash identifier.
  KeyId,
  20
);

fixed_bytes_newtype!(
  /// 20-byte Evo masternode platform node ID.
  PlatformNodeId,
  20
);

fixed_bytes_newtype!(
  /// 48-byte BLS public key.
  BlsPublicKey,
  48
);

fixed_bytes_newtype!(
  /// 96-byte BLS signature.
  BlsSignature,
  96
);

const MAX_SCRIPT_SIZE: usize = 10_000;

/// CompactSize-prefixed variable-length script.
#[cfg(feature = "alloc")]
#[derive(Clone, PartialEq, Eq)]
pub struct Script(pub crate::prelude::Vec<u8>);

#[cfg(feature = "alloc")]
impl fmt::Debug for Script {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    write!(f, "Script({} bytes: ", self.0.len())?;
    for byte in &self.0 {
      write!(f, "{byte:02x}")?;
    }
    write!(f, ")")
  }
}

#[cfg(feature = "alloc")]
impl Hash for Script {
  fn hash<H: Hasher>(&self, state: &mut H) {
    self.0.hash(state);
  }
}

#[cfg(feature = "alloc")]
impl Decode for Script {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let bytes = cursor.read_compact_bytes(MAX_SCRIPT_SIZE)?;
    Ok(Self(bytes))
  }
}

#[cfg(all(feature = "alloc", feature = "serde"))]
impl serde::Serialize for Script {
  fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    if serializer.is_human_readable() {
      use core::fmt::Write;
      let mut hex = crate::prelude::String::with_capacity(self.0.len() * 2);
      for byte in &self.0 {
        let _ = write!(hex, "{byte:02x}");
      }
      serializer.serialize_str(&hex)
    } else {
      serializer.serialize_bytes(&self.0)
    }
  }
}

#[cfg(feature = "alloc")]
impl Script {
  #[inline]
  pub fn as_bytes(&self) -> &[u8] {
    &self.0
  }

  #[inline]
  pub fn len(&self) -> usize {
    self.0.len()
  }

  #[inline]
  pub fn is_empty(&self) -> bool {
    self.0.is_empty()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn decode_block_hash() {
    let bytes = [0xABu8; 32];
    let mut cur = Cursor::new(&bytes);
    let hash = BlockHash::consensus_decode(&mut cur);
    assert_eq!(hash, Ok(BlockHash(bytes)));
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_tx_hash() {
    let bytes = [0xCDu8; 32];
    let mut cur = Cursor::new(&bytes);
    let hash = TxHash::consensus_decode(&mut cur);
    assert_eq!(hash, Ok(TxHash(bytes)));
  }

  #[test]
  fn decode_key_id_20_bytes() {
    let bytes = [0x42u8; 20];
    let mut cur = Cursor::new(&bytes);
    let key = KeyId::consensus_decode(&mut cur);
    assert_eq!(key, Ok(KeyId(bytes)));
  }

  #[test]
  fn decode_bls_public_key_48_bytes() {
    let bytes = [0x11u8; 48];
    let mut cur = Cursor::new(&bytes);
    let key = BlsPublicKey::consensus_decode(&mut cur);
    assert_eq!(key, Ok(BlsPublicKey(bytes)));
  }

  #[test]
  fn decode_bls_signature_96_bytes() {
    let bytes = [0x22u8; 96];
    let mut cur = Cursor::new(&bytes);
    let sig = BlsSignature::consensus_decode(&mut cur);
    assert_eq!(sig, Ok(BlsSignature(bytes)));
  }

  #[test]
  fn block_hash_eof() {
    let bytes = [0x00u8; 31];
    let mut cur = Cursor::new(&bytes);
    assert!(BlockHash::consensus_decode(&mut cur).is_err());
  }

  #[test]
  fn newtype_from_array() {
    let arr = [0xFFu8; 32];
    let hash = BlockHash::from(arr);
    assert_eq!(hash.as_bytes(), &arr);
  }

  #[test]
  fn newtype_debug_shows_hex() {
    let hash = BlockHash([
      0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0A, 0x0B, 0x0C, 0x0D, 0x0E, 0x0F,
      0x10, 0x11, 0x12, 0x13, 0x14, 0x15, 0x16, 0x17, 0x18, 0x19, 0x1A, 0x1B, 0x1C, 0x1D, 0x1E,
      0x1F, 0x20,
    ]);
    let dbg = format!("{hash:?}");
    assert!(dbg.starts_with("BlockHash("));
    assert!(dbg.contains("0102030405060708"));
    assert!(dbg.ends_with(')'));
  }

  #[test]
  fn newtype_copy_semantics() {
    let a = BlockHash([0xAA; 32]);
    let b = a;
    assert_eq!(a, b);
  }

  // -- Script --

  #[test]
  fn decode_script_empty() {
    let mut cur = Cursor::new(&[0x00]);
    let script = Script::consensus_decode(&mut cur);
    let script = script.expect("should decode");
    assert!(script.is_empty());
    assert_eq!(script.len(), 0);
  }

  #[test]
  fn decode_script_small() {
    let mut cur = Cursor::new(&[0x03, 0x76, 0xA9, 0x14]);
    let script = Script::consensus_decode(&mut cur);
    let script = script.expect("should decode");
    assert_eq!(script.as_bytes(), &[0x76, 0xA9, 0x14]);
    assert_eq!(script.len(), 3);
  }

  #[test]
  fn decode_script_max_size_boundary() {
    let mut buf = crate::prelude::Vec::new();
    buf.push(0xFD);
    buf.extend_from_slice(&10000u16.to_le_bytes());
    buf.extend(core::iter::repeat(0xAA).take(10_000));

    let mut cur = Cursor::new(&buf);
    let script = Script::consensus_decode(&mut cur);
    let script = script.expect("should decode at max size");
    assert_eq!(script.len(), 10_000);
  }

  #[test]
  fn decode_script_exceeds_max_size() {
    let mut buf = crate::prelude::Vec::new();
    buf.push(0xFD);
    buf.extend_from_slice(&10001u16.to_le_bytes());
    buf.extend(core::iter::repeat(0xAA).take(10_001));

    let mut cur = Cursor::new(&buf);
    let result = Script::consensus_decode(&mut cur);
    assert_eq!(
      result,
      Err(DecodeError::Read(
        crate::error::ReadError::CompactSizeOverflow(10001)
      ))
    );
  }

  #[test]
  fn script_debug_shows_hex() {
    let script = Script(crate::prelude::vec![0x76, 0xA9, 0x14]);
    let dbg = format!("{script:?}");
    assert!(dbg.starts_with("Script(3 bytes: "));
    assert!(dbg.contains("76a914"));
  }
}
