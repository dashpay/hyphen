//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Governance wire parsing and canonical hash helpers.

use crate::cursor::Cursor;
use crate::hash::double_sha256;

/// Governance wire parsing errors.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GovError {
  /// One raw governance payload could not be decoded.
  Decode(crate::error::ReadError),
}

impl core::fmt::Display for GovError {
  fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    match self {
      Self::Decode(error) => write!(f, "{error}"),
    }
  }
}

#[cfg(feature = "std")]
impl std::error::Error for GovError {}

/// Read the parent object hash from one raw governance vote.
///
/// The returned bytes follow RPC/display order.
pub fn vote_parent_hash(raw_vote: &[u8]) -> Result<[u8; 32], GovError> {
  let mut cursor = Cursor::new(raw_vote);
  let _ = cursor.read_array::<32>().map_err(GovError::Decode)?;
  let _ = cursor.read_u32_le().map_err(GovError::Decode)?;
  let mut parent_hash = cursor.read_array::<32>().map_err(GovError::Decode)?;
  parent_hash.reverse();
  Ok(parent_hash)
}

/// Compute the canonical governance object hash from one raw governance object.
///
/// The returned bytes follow RPC/display order.
pub fn object_hash(raw_object: &[u8]) -> Result<[u8; 32], GovError> {
  let mut cursor = Cursor::new(raw_object);
  let parent_hash = cursor.read_array::<32>().map_err(GovError::Decode)?;
  let revision = cursor.read_i32_le().map_err(GovError::Decode)?;
  let object_time = cursor.read_i64_le().map_err(GovError::Decode)?;
  let _ = cursor.read_array::<32>().map_err(GovError::Decode)?;
  let data = cursor
    .read_compact_bytes(1_000_000)
    .map_err(GovError::Decode)?;
  let _ = cursor.read_i32_le().map_err(GovError::Decode)?;
  let masternode_hash = cursor.read_array::<32>().map_err(GovError::Decode)?;
  let _ = cursor.read_u32_le().map_err(GovError::Decode)?;
  let sig = cursor
    .read_compact_bytes(10_000)
    .map_err(GovError::Decode)?;

  let data_hex = hex::encode(&data);
  let mut hash_input = crate::prelude::Vec::new();
  hash_input.extend_from_slice(&parent_hash);
  hash_input.extend_from_slice(&revision.to_le_bytes());
  hash_input.extend_from_slice(&object_time.to_le_bytes());
  write_compact_size(&mut hash_input, data_hex.len() as u64);
  hash_input.extend_from_slice(data_hex.as_bytes());
  hash_input.extend_from_slice(&masternode_hash);
  hash_input.push(0);
  hash_input.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
  write_compact_size(&mut hash_input, sig.len() as u64);
  hash_input.extend_from_slice(&sig);

  let mut hash = double_sha256(&hash_input);
  hash.reverse();
  Ok(hash)
}

fn write_compact_size(buf: &mut crate::prelude::Vec<u8>, value: u64) {
  match value {
    0..=252 => buf.push(value as u8),
    253..=0xFFFF => {
      buf.push(253);
      buf.extend_from_slice(&(value as u16).to_le_bytes());
    }
    0x1_0000..=0xFFFF_FFFF => {
      buf.push(254);
      buf.extend_from_slice(&(value as u32).to_le_bytes());
    }
    _ => {
      buf.push(255);
      buf.extend_from_slice(&value.to_le_bytes());
    }
  }
}

#[cfg(test)]
mod tests {
  use super::{object_hash, vote_parent_hash};

  #[test]
  fn vote_parent_hash_reads_display_order() {
    let mut raw_vote = Vec::new();
    raw_vote.extend_from_slice(&[0x55; 32]);
    raw_vote.extend_from_slice(&7u32.to_le_bytes());
    raw_vote.extend((0u8..32).collect::<Vec<_>>().iter().rev().copied());
    raw_vote.extend_from_slice(&1i32.to_le_bytes());
    raw_vote.extend_from_slice(&2i32.to_le_bytes());
    raw_vote.extend_from_slice(&3i64.to_le_bytes());
    raw_vote.push(0);
    assert_eq!(
      hex::encode(vote_parent_hash(&raw_vote).expect("parent hash should decode")),
      "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
    );
  }

  #[test]
  fn object_hash_matches_dash_rules() {
    let data_hex = "7b226e616d65223a2270726f706f73616c227d";
    let data = hex::decode(data_hex).expect("object data hex should decode");
    let signature = vec![0xaa, 0xbb, 0xcc];

    let mut raw = Vec::new();
    raw.extend_from_slice(&[0u8; 32]);
    raw.extend_from_slice(&1i32.to_le_bytes());
    raw.extend_from_slice(&123i64.to_le_bytes());
    raw.extend_from_slice(&[0u8; 32]);
    super::write_compact_size(&mut raw, data.len() as u64);
    raw.extend_from_slice(&data);
    raw.extend_from_slice(&1i32.to_le_bytes());
    raw.extend_from_slice(&[0x11; 32]);
    raw.extend_from_slice(&7u32.to_le_bytes());
    super::write_compact_size(&mut raw, signature.len() as u64);
    raw.extend_from_slice(&signature);

    assert_eq!(
      hex::encode(object_hash(&raw).expect("object hash should decode")),
      "e3b703644367c38fba1012dc2e549876dda0203ea3c7cde44846b3fe2c84236f"
    );
  }
}
