//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! 80-byte block header decoding.

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::types::common::{BlockHash, MerkleRoot};

/// Fixed 80-byte block header.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct BlockHeader {
  pub version: i32,
  pub prev_hash: BlockHash,
  pub merkle_root: MerkleRoot,
  /// Unix epoch seconds.
  pub time: u32,
  /// Compact difficulty target.
  pub bits: u32,
  pub nonce: u32,
}

impl Decode for BlockHeader {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(Self {
      version: i32::consensus_decode(cursor)?,
      prev_hash: BlockHash::consensus_decode(cursor)?,
      merkle_root: MerkleRoot::consensus_decode(cursor)?,
      time: u32::consensus_decode(cursor)?,
      bits: u32::consensus_decode(cursor)?,
      nonce: u32::consensus_decode(cursor)?,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn make_test_header() -> [u8; 80] {
    let mut buf = [0u8; 80];
    buf[0..4].copy_from_slice(&4i32.to_le_bytes());
    buf[4..36].fill(0xAA);
    buf[36..68].fill(0xBB);
    buf[68..72].copy_from_slice(&1_700_000_000u32.to_le_bytes());
    buf[72..76].copy_from_slice(&0x1A0FFFFFu32.to_le_bytes());
    buf[76..80].copy_from_slice(&42u32.to_le_bytes());
    buf
  }

  #[test]
  fn decode_block_header_known_80_bytes() {
    let buf = make_test_header();
    let mut cur = Cursor::new(&buf);
    let header = BlockHeader::consensus_decode(&mut cur);
    let header = header.expect("should decode valid header");
    assert_eq!(header.version, 4);
    assert_eq!(header.prev_hash, BlockHash([0xAA; 32]));
    assert_eq!(header.merkle_root, MerkleRoot([0xBB; 32]));
    assert_eq!(header.time, 1_700_000_000);
    assert_eq!(header.bits, 0x1A0FFFFF);
    assert_eq!(header.nonce, 42);
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_block_header_consumes_exactly_80_bytes() {
    let mut buf = [0u8; 84];
    buf[..80].copy_from_slice(&make_test_header());
    buf[80..84].copy_from_slice(&[0xDE, 0xAD, 0xBE, 0xEF]);
    let mut cur = Cursor::new(&buf);
    let _ = BlockHeader::consensus_decode(&mut cur).expect("should decode");
    assert_eq!(cur.remaining(), 4);
    assert_eq!(cur.position(), 80);
  }

  #[test]
  fn decode_block_header_eof_mid_header() {
    let buf = [0u8; 40];
    let mut cur = Cursor::new(&buf);
    assert!(BlockHeader::consensus_decode(&mut cur).is_err());
  }

  #[test]
  fn decode_block_header_empty_buffer() {
    let mut cur = Cursor::new(&[]);
    assert!(BlockHeader::consensus_decode(&mut cur).is_err());
  }
}
