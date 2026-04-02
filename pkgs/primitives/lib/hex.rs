//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Hex string to byte-vector conversion.

#[cfg(feature = "alloc")]
use crate::prelude::Vec;
use core::fmt;

/// Hex decoding errors.
#[derive(Clone, PartialEq, Eq)]
pub enum HexError {
  /// Input has odd length.
  OddLength,

  /// Non-hex character encountered.
  InvalidChar { index: usize, ch: char },
}

impl fmt::Debug for HexError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::OddLength => write!(f, "OddLength"),
      Self::InvalidChar { index, ch } => f
        .debug_struct("InvalidChar")
        .field("index", index)
        .field("ch", ch)
        .finish(),
    }
  }
}

impl fmt::Display for HexError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::OddLength => {
        write!(f, "hex string has odd length, cannot form complete bytes")
      }
      Self::InvalidChar { index, ch } => {
        write!(f, "invalid hex character '{ch}' at position {index}")
      }
    }
  }
}

#[cfg(feature = "std")]
impl std::error::Error for HexError {}

/// Single hex char to its 4-bit value.
#[inline]
fn hex_val(ch: u8) -> Option<u8> {
  match ch {
    b'0'..=b'9' => Some(ch - b'0'),
    b'a'..=b'f' => Some(ch - b'a' + 10),
    b'A'..=b'F' => Some(ch - b'A' + 10),
    _ => None,
  }
}

/// Decode a hex string into raw bytes.
#[cfg(feature = "alloc")]
pub fn decode_hex(s: &str) -> Result<Vec<u8>, HexError> {
  let bytes = s.as_bytes();
  if !bytes.len().is_multiple_of(2) {
    return Err(HexError::OddLength);
  }

  let mut out = Vec::with_capacity(bytes.len() / 2);
  for i in (0..bytes.len()).step_by(2) {
    let hi = hex_val(bytes[i]).ok_or(HexError::InvalidChar {
      index: i,
      ch: bytes[i] as char,
    })?;
    let lo = hex_val(bytes[i + 1]).ok_or(HexError::InvalidChar {
      index: i + 1,
      ch: bytes[i + 1] as char,
    })?;
    out.push((hi << 4) | lo);
  }
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn decode_known_hex_string() {
    let result = decode_hex("48656c6c6f");
    assert_eq!(result, Ok(vec![0x48, 0x65, 0x6c, 0x6c, 0x6f]));
  }

  #[test]
  fn decode_uppercase_hex() {
    let result = decode_hex("DEADBEEF");
    assert_eq!(result, Ok(vec![0xDE, 0xAD, 0xBE, 0xEF]));
  }

  #[test]
  fn decode_mixed_case_hex() {
    let result = decode_hex("DeAdBeEf");
    assert_eq!(result, Ok(vec![0xDE, 0xAD, 0xBE, 0xEF]));
  }

  #[test]
  fn decode_empty_string() {
    let result = decode_hex("");
    assert_eq!(result, Ok(vec![]));
  }

  #[test]
  fn decode_single_byte() {
    let result = decode_hex("ff");
    assert_eq!(result, Ok(vec![0xFF]));
  }

  #[test]
  fn reject_odd_length() {
    let result = decode_hex("ABC");
    assert_eq!(result, Err(HexError::OddLength));
  }

  #[test]
  fn reject_odd_length_single_char() {
    let result = decode_hex("A");
    assert_eq!(result, Err(HexError::OddLength));
  }

  #[test]
  fn reject_invalid_char_at_start() {
    let result = decode_hex("GG");
    assert_eq!(result, Err(HexError::InvalidChar { index: 0, ch: 'G' }));
  }

  #[test]
  fn reject_invalid_char_mid_string() {
    let result = decode_hex("AB__CD");
    assert_eq!(result, Err(HexError::InvalidChar { index: 2, ch: '_' }));
  }

  #[test]
  fn reject_space_in_hex() {
    let result = decode_hex("AB CD");
    assert_eq!(result, Err(HexError::OddLength));
  }

  #[test]
  fn all_zero_bytes() {
    let result = decode_hex("000000");
    assert_eq!(result, Ok(vec![0x00, 0x00, 0x00]));
  }

  #[test]
  fn all_ff_bytes() {
    let result = decode_hex("ffffff");
    assert_eq!(result, Ok(vec![0xFF, 0xFF, 0xFF]));
  }
}
