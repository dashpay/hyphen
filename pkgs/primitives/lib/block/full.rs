//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Full block: header + transaction vector.

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::{BlockError, DecodeError, TxError};
use crate::prelude::Vec;

use super::header::BlockHeader;
use crate::tx::transaction::Transaction;

const MAX_BLOCK_TXS: usize = 100_000;

/// Header + ordered transaction vector.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct Block {
  pub header: BlockHeader,
  pub transactions: Vec<Transaction>,
}

impl Block {
  /// Wraps tx failures with the failing index.
  pub fn consensus_decode_block(cursor: &mut Cursor<'_>) -> Result<Self, BlockError> {
    let header = BlockHeader::consensus_decode(cursor)?;

    let tx_count = cursor.read_compact_size()?;
    if tx_count > MAX_BLOCK_TXS as u64 {
      return Err(BlockError::Decode(DecodeError::InvalidValue {
        field: "transaction count",
        reason: "exceeds maximum allowed transactions per block",
      }));
    }

    let mut transactions = Vec::with_capacity(tx_count as usize);
    for i in 0..tx_count as usize {
      let tx = Transaction::consensus_decode_tx(cursor).map_err(|e: TxError| BlockError::Tx {
        index: i,
        source: e,
      })?;
      transactions.push(tx);
    }

    Ok(Self {
      header,
      transactions,
    })
  }

  /// Decode one raw serialized block and reject trailing bytes.
  pub fn from_bytes(raw: &[u8]) -> Result<Self, BlockError> {
    let mut cursor = Cursor::new(raw);
    let block = Self::consensus_decode_block(&mut cursor)?;
    if !cursor.is_empty() {
      return Err(BlockError::Decode(DecodeError::InvalidValue {
        field: "block",
        reason: "trailing bytes after block decode",
      }));
    }
    Ok(block)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::types::common::{BlockHash, MerkleRoot};
  use crate::types::tx::TxType;

  fn make_minimal_block() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&4i32.to_le_bytes());
    buf.extend_from_slice(&[0xAA; 32]);
    buf.extend_from_slice(&[0xBB; 32]);
    buf.extend_from_slice(&1_700_000_000u32.to_le_bytes());
    buf.extend_from_slice(&0x1A0FFFFFu32.to_le_bytes());
    buf.extend_from_slice(&42u32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&1i32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&[0x00; 32]);
    buf.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    buf.push(0x04);
    buf.extend_from_slice(&[0x01, 0x02, 0x03, 0x04]);
    buf.extend_from_slice(&0xFFFFFFFFu32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&50_0000_0000i64.to_le_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
  }

  #[test]
  fn decode_minimal_block() {
    let data = make_minimal_block();
    let mut cur = Cursor::new(&data);
    let block = Block::consensus_decode_block(&mut cur);
    let block = block.expect("should decode minimal block");
    assert_eq!(block.header.version, 4);
    assert_eq!(block.header.prev_hash, BlockHash([0xAA; 32]));
    assert_eq!(block.header.merkle_root, MerkleRoot([0xBB; 32]));
    assert_eq!(block.header.time, 1_700_000_000);
    assert_eq!(block.header.nonce, 42);
    assert_eq!(block.transactions.len(), 1);
    let tx = &block.transactions[0];
    assert_eq!(tx.version, 1);
    assert_eq!(tx.tx_type, TxType::Normal);
    assert_eq!(tx.inputs.len(), 1);
    assert_eq!(tx.outputs.len(), 1);
    assert!(tx.extra_payload.is_empty());
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_block_eof_in_transactions() {
    let data = make_minimal_block();
    let truncated = &data[..81];
    let mut cur = Cursor::new(truncated);
    assert!(Block::consensus_decode_block(&mut cur).is_err());
  }

  #[test]
  fn decode_block_empty_buffer() {
    let mut cur = Cursor::new(&[]);
    assert!(Block::consensus_decode_block(&mut cur).is_err());
  }
}
