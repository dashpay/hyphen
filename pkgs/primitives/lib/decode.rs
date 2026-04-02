//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Consensus decoding trait and impls.

use crate::cursor::Cursor;
use crate::error::DecodeError;

/// Decode a value from consensus wire format.
pub trait Decode: Sized {
  /// Read one value from the cursor.
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError>;
}

impl Decode for u8 {
  #[inline]
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(cursor.read_u8()?)
  }
}

impl Decode for u16 {
  #[inline]
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(cursor.read_u16_le()?)
  }
}

impl Decode for u32 {
  #[inline]
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(cursor.read_u32_le()?)
  }
}

impl Decode for u64 {
  #[inline]
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(cursor.read_u64_le()?)
  }
}

impl Decode for i16 {
  #[inline]
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(cursor.read_i16_le()?)
  }
}

impl Decode for i32 {
  #[inline]
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(cursor.read_i32_le()?)
  }
}

impl Decode for i64 {
  #[inline]
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(cursor.read_i64_le()?)
  }
}

/// Zero is `false`, any non-zero is `true`.
impl Decode for bool {
  #[inline]
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let byte = cursor.read_u8()?;
    Ok(byte != 0)
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::error::ReadError;

  #[test]
  fn decode_u8() {
    let mut cur = Cursor::new(&[0x42]);
    assert_eq!(u8::consensus_decode(&mut cur), Ok(0x42));
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_u8_eof() {
    let mut cur = Cursor::new(&[]);
    assert!(u8::consensus_decode(&mut cur).is_err());
  }

  #[test]
  fn decode_u16() {
    let mut cur = Cursor::new(&[0x34, 0x12]);
    assert_eq!(u16::consensus_decode(&mut cur), Ok(0x1234));
  }

  #[test]
  fn decode_u16_eof() {
    let mut cur = Cursor::new(&[0x01]);
    assert!(u16::consensus_decode(&mut cur).is_err());
  }

  #[test]
  fn decode_u32() {
    let mut cur = Cursor::new(&[0xEF, 0xBE, 0xAD, 0xDE]);
    assert_eq!(u32::consensus_decode(&mut cur), Ok(0xDEADBEEF));
  }

  #[test]
  fn decode_u32_eof() {
    let mut cur = Cursor::new(&[0x01, 0x02]);
    assert!(u32::consensus_decode(&mut cur).is_err());
  }

  #[test]
  fn decode_u64() {
    let mut cur = Cursor::new(&[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
    assert_eq!(u64::consensus_decode(&mut cur), Ok(0x0102030405060708));
  }

  #[test]
  fn decode_u64_eof() {
    let mut cur = Cursor::new(&[0x01; 5]);
    assert!(u64::consensus_decode(&mut cur).is_err());
  }

  #[test]
  fn decode_i16_positive() {
    let mut cur = Cursor::new(&[0x39, 0x05]);
    assert_eq!(i16::consensus_decode(&mut cur), Ok(1337));
  }

  #[test]
  fn decode_i16_negative() {
    let mut cur = Cursor::new(&[0xFF, 0xFF]);
    assert_eq!(i16::consensus_decode(&mut cur), Ok(-1));
  }

  #[test]
  fn decode_i32_positive() {
    let mut cur = Cursor::new(&[0xA0, 0x86, 0x01, 0x00]);
    assert_eq!(i32::consensus_decode(&mut cur), Ok(100_000));
  }

  #[test]
  fn decode_i32_negative() {
    let mut cur = Cursor::new(&[0x00, 0xFF, 0xFF, 0xFF]);
    assert_eq!(i32::consensus_decode(&mut cur), Ok(-256));
  }

  #[test]
  fn decode_i64_positive() {
    let mut cur = Cursor::new(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(i64::consensus_decode(&mut cur), Ok(1));
  }

  #[test]
  fn decode_i64_negative() {
    let mut cur = Cursor::new(&[0xFF; 8]);
    assert_eq!(i64::consensus_decode(&mut cur), Ok(-1));
  }

  #[test]
  fn decode_bool_false() {
    let mut cur = Cursor::new(&[0x00]);
    assert_eq!(bool::consensus_decode(&mut cur), Ok(false));
  }

  #[test]
  fn decode_bool_true_one() {
    let mut cur = Cursor::new(&[0x01]);
    assert_eq!(bool::consensus_decode(&mut cur), Ok(true));
  }

  #[test]
  fn decode_bool_true_nonzero() {
    let mut cur = Cursor::new(&[0xFF]);
    assert_eq!(bool::consensus_decode(&mut cur), Ok(true));
  }

  #[test]
  fn decode_bool_eof() {
    let mut cur = Cursor::new(&[]);
    assert!(bool::consensus_decode(&mut cur).is_err());
  }

  #[test]
  fn error_conversion_read_to_decode() {
    let read_err = ReadError::UnexpectedEof {
      needed: 4,
      available: 0,
    };
    let decode_err: DecodeError = read_err.clone().into();
    assert_eq!(decode_err, DecodeError::Read(read_err));
  }

  #[test]
  fn sequential_decode_multiple_types() {
    let data = [0x42, 0xEF, 0xBE, 0xAD, 0xDE, 0x01];
    let mut cur = Cursor::new(&data);
    assert_eq!(u8::consensus_decode(&mut cur), Ok(0x42));
    assert_eq!(u32::consensus_decode(&mut cur), Ok(0xDEADBEEF));
    assert_eq!(bool::consensus_decode(&mut cur), Ok(true));
    assert!(cur.is_empty());
  }
}
