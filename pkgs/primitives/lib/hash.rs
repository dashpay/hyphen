//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Canonical chain identity and compact-target helpers.

use crate::cursor::Cursor;
use crate::error::DecodeError;
use crate::tx::transaction::Transaction;
use crate::types::common::{BlockHash, TxHash};
use sha2::{Digest, Sha256};

const BLOCK_HEADER_LEN: usize = 80;

/// Hashing errors for raw chain payload helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HashError {
  /// The provided raw block or header is shorter than 80 bytes.
  HeaderTooShort { len: usize },
  /// One CompactSize or primitive read failed.
  Decode(DecodeError),
  /// One transaction in the raw block could not be decoded.
  Tx(crate::error::TxError),
}

impl core::fmt::Display for HashError {
  fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    match self {
      Self::HeaderTooShort { len } => {
        write!(
          f,
          "block header requires {BLOCK_HEADER_LEN} bytes, found {len}"
        )
      }
      Self::Decode(error) => write!(f, "{error}"),
      Self::Tx(error) => write!(f, "{error}"),
    }
  }
}

#[cfg(feature = "std")]
impl std::error::Error for HashError {}

/// Compute the canonical double-SHA256 digest of one transaction-like payload.
pub fn double_sha256(data: &[u8]) -> [u8; 32] {
  let first = Sha256::digest(data);
  let second = Sha256::digest(first);
  let mut result = [0u8; 32];
  result.copy_from_slice(&second);
  result
}

/// Compute the canonical transaction hash for serialized transaction bytes.
pub fn tx_hash(raw_tx: &[u8]) -> TxHash {
  TxHash(double_sha256(raw_tx))
}

/// Compute the canonical Dash X11 block hash from the 80-byte header prefix.
#[cfg(feature = "std")]
pub fn block_hash_from_header_bytes(header: &[u8]) -> Result<BlockHash, HashError> {
  if header.len() < BLOCK_HEADER_LEN {
    return Err(HashError::HeaderTooShort { len: header.len() });
  }
  Ok(BlockHash(rs_x11_hash::get_x11_hash(
    &header[..BLOCK_HEADER_LEN],
  )))
}

/// Compute the canonical Dash X11 block hash from raw serialized block bytes.
#[cfg(feature = "std")]
pub fn block_hash(raw_block: &[u8]) -> Result<BlockHash, HashError> {
  block_hash_from_header_bytes(raw_block)
}

/// Walk raw block bytes and return `(start, end)` offsets for each transaction.
pub fn tx_byte_ranges(raw_block: &[u8]) -> Result<crate::prelude::Vec<(usize, usize)>, HashError> {
  let mut cursor = Cursor::new(raw_block);
  if cursor.read_bytes(BLOCK_HEADER_LEN).is_err() {
    return Err(HashError::HeaderTooShort {
      len: raw_block.len(),
    });
  }

  let tx_count = cursor
    .read_compact_size()
    .map_err(DecodeError::from)
    .map_err(HashError::Decode)? as usize;
  let mut ranges = crate::prelude::Vec::with_capacity(tx_count);
  for _ in 0..tx_count {
    let start = cursor.position();
    Transaction::consensus_decode_tx(&mut cursor).map_err(HashError::Tx)?;
    ranges.push((start, cursor.position()));
  }
  Ok(ranges)
}

/// Convert compact `bits` encoding into a floating-point difficulty value.
pub fn bits_to_difficulty(bits: u32) -> f64 {
  let exponent = (bits >> 24) & 0xFF;
  let mantissa = bits & 0x007F_FFFF;
  if mantissa == 0 {
    return 0.0;
  }
  let genesis_target = 0x00FFFF_u64 as f64 * 256.0_f64.powi(0x1d - 3);
  let target = mantissa as f64 * 256.0_f64.powi(exponent as i32 - 3);
  genesis_target / target
}

#[cfg(test)]
mod tests {
  use super::{bits_to_difficulty, block_hash_from_header_bytes, tx_byte_ranges, tx_hash};

  #[test]
  fn tx_hash_matches_known_double_sha256() {
    let txid = tx_hash(&[1u8, 2, 3, 4]);
    assert_eq!(
      hex::encode(txid.0),
      "8de472e2399610baaa7f84840547cd409434e31f5d3bd71e4d947f283874f9c0"
    );
  }

  #[cfg(feature = "std")]
  #[test]
  fn block_hash_matches_known_x11_header_hash() {
    let header = hex::decode(
      "040000002e3df23eec5cd6a86edd509539028e2c3a3dc05315eb28f2baa43218ca080000b3a56d65316ffdb006163240a4380e94a4c2d8c0f0b3b2c1ddc486fae15ed065ba968054ffff7f2002000000",
    )
    .expect("header hex should decode");
    let hash = block_hash_from_header_bytes(&header).expect("x11 hash should compute");
    assert_eq!(
      hex::encode(hash.0),
      "000739d9da507b3acb949f21fe10ad424abbad5b4c46789285b05fe36df5c5b0"
    );
  }

  #[test]
  fn tx_byte_ranges_walk_minimal_block() {
    let mut raw = Vec::new();
    raw.extend_from_slice(&[0u8; 80]);
    raw.push(1);
    raw.extend_from_slice(&1i32.to_le_bytes());
    raw.push(0);
    raw.push(0);
    raw.extend_from_slice(&0u32.to_le_bytes());
    let ranges = tx_byte_ranges(&raw).expect("tx ranges should decode");
    assert_eq!(ranges, vec![(81, raw.len())]);
  }

  #[test]
  fn bits_to_difficulty_handles_zero_mantissa() {
    assert_eq!(bits_to_difficulty(0), 0.0);
  }
}
