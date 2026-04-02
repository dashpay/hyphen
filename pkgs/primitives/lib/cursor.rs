//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//
// Contains code adapted from rust-bitcoin, released under CC0 1.0 Universal
// See accompanying file LICENSE.CC0 or https://creativecommons.org/publicdomain/zero/1.0
//

//! Zero-copy byte cursor for consensus decoding.

use crate::error::ReadError;

/// Zero-copy, little-endian byte cursor.
#[derive(Debug)]
pub struct Cursor<'a> {
  data: &'a [u8],
  pos: usize,
}

impl<'a> Cursor<'a> {
  /// Wrap a byte slice at position zero.
  #[inline]
  pub fn new(data: &'a [u8]) -> Self {
    Self { data, pos: 0 }
  }

  /// Bytes remaining after current position.
  #[inline]
  pub fn remaining(&self) -> usize {
    self.data.len() - self.pos
  }

  /// True when fully consumed.
  #[inline]
  pub fn is_empty(&self) -> bool {
    self.pos >= self.data.len()
  }

  /// Current byte offset from start.
  #[inline]
  pub fn position(&self) -> usize {
    self.pos
  }

  /// Slice of bytes consumed since `start_pos`.
  #[inline]
  pub fn consumed_since(&self, start_pos: usize) -> &'a [u8] {
    &self.data[start_pos..self.pos]
  }

  /// Read one byte.
  #[inline]
  pub fn read_u8(&mut self) -> Result<u8, ReadError> {
    if self.remaining() < 1 {
      return Err(ReadError::UnexpectedEof {
        needed: 1,
        available: self.remaining(),
      });
    }
    let val = self.data[self.pos];
    self.pos += 1;
    Ok(val)
  }

  /// Read a little-endian `u16`.
  #[inline]
  pub fn read_u16_le(&mut self) -> Result<u16, ReadError> {
    let bytes = self.read_array::<2>()?;
    Ok(u16::from_le_bytes(bytes))
  }

  /// Read a big-endian `u16` (network ports).
  #[inline]
  pub fn read_u16_be(&mut self) -> Result<u16, ReadError> {
    let bytes = self.read_array::<2>()?;
    Ok(u16::from_be_bytes(bytes))
  }

  /// Read a little-endian `u32`.
  #[inline]
  pub fn read_u32_le(&mut self) -> Result<u32, ReadError> {
    let bytes = self.read_array::<4>()?;
    Ok(u32::from_le_bytes(bytes))
  }

  /// Read a little-endian `u64`.
  #[inline]
  pub fn read_u64_le(&mut self) -> Result<u64, ReadError> {
    let bytes = self.read_array::<8>()?;
    Ok(u64::from_le_bytes(bytes))
  }

  /// Read a little-endian `i16`.
  #[inline]
  pub fn read_i16_le(&mut self) -> Result<i16, ReadError> {
    let bytes = self.read_array::<2>()?;
    Ok(i16::from_le_bytes(bytes))
  }

  /// Read a little-endian `i32`.
  #[inline]
  pub fn read_i32_le(&mut self) -> Result<i32, ReadError> {
    let bytes = self.read_array::<4>()?;
    Ok(i32::from_le_bytes(bytes))
  }

  /// Read a little-endian `i64`.
  #[inline]
  pub fn read_i64_le(&mut self) -> Result<i64, ReadError> {
    let bytes = self.read_array::<8>()?;
    Ok(i64::from_le_bytes(bytes))
  }

  /// Read `n` bytes as a zero-copy slice.
  #[inline]
  pub fn read_bytes(&mut self, n: usize) -> Result<&'a [u8], ReadError> {
    if self.remaining() < n {
      return Err(ReadError::UnexpectedEof {
        needed: n,
        available: self.remaining(),
      });
    }
    let slice = &self.data[self.pos..self.pos + n];
    self.pos += n;
    Ok(slice)
  }

  /// Read `N` bytes into a fixed-size array.
  #[inline]
  pub fn read_array<const N: usize>(&mut self) -> Result<[u8; N], ReadError> {
    let slice = self.read_bytes(N)?;
    let mut arr = [0u8; N];
    arr.copy_from_slice(slice);
    Ok(arr)
  }

  /// Read a CompactSize varint.
  ///
  /// Enforces canonical minimal encoding.
  pub fn read_compact_size(&mut self) -> Result<u64, ReadError> {
    let first = self.read_u8()?;
    match first {
      0..=0xFC => Ok(u64::from(first)),
      0xFD => {
        let val = u64::from(self.read_u16_le()?);
        if val < 0xFD {
          return Err(ReadError::NonCanonicalCompactSize);
        }
        Ok(val)
      }
      0xFE => {
        let val = u64::from(self.read_u32_le()?);
        if val < 0x10000 {
          return Err(ReadError::NonCanonicalCompactSize);
        }
        Ok(val)
      }
      0xFF => {
        let val = self.read_u64_le()?;
        if val < 0x100000000 {
          return Err(ReadError::NonCanonicalCompactSize);
        }
        Ok(val)
      }
    }
  }

  /// Read CompactSize-prefixed bytes, capped.
  #[cfg(feature = "alloc")]
  pub fn read_compact_bytes(
    &mut self,
    max_len: usize,
  ) -> Result<crate::prelude::Vec<u8>, ReadError> {
    let len = self.read_compact_size()?;
    if len > max_len as u64 {
      return Err(ReadError::CompactSizeOverflow(len));
    }
    let len = len as usize;
    let slice = self.read_bytes(len)?;
    Ok(slice.to_vec())
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn new_cursor_starts_at_position_zero() {
    let cur = Cursor::new(&[0xAA, 0xBB]);
    assert_eq!(cur.position(), 0);
    assert_eq!(cur.remaining(), 2);
    assert!(!cur.is_empty());
  }

  #[test]
  fn empty_cursor_reports_empty() {
    let cur = Cursor::new(&[]);
    assert!(cur.is_empty());
    assert_eq!(cur.remaining(), 0);
    assert_eq!(cur.position(), 0);
  }

  #[test]
  fn position_advances_after_reads() {
    let mut cur = Cursor::new(&[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(cur.position(), 0);
    let _ = cur.read_u8();
    assert_eq!(cur.position(), 1);
    assert_eq!(cur.remaining(), 3);
    let _ = cur.read_u16_le();
    assert_eq!(cur.position(), 3);
    assert_eq!(cur.remaining(), 1);
  }

  #[test]
  fn cursor_becomes_empty_after_full_read() {
    let mut cur = Cursor::new(&[0xFF]);
    let _ = cur.read_u8();
    assert!(cur.is_empty());
    assert_eq!(cur.remaining(), 0);
  }

  #[test]
  fn read_u8_returns_correct_value() {
    let mut cur = Cursor::new(&[0x42]);
    assert_eq!(cur.read_u8(), Ok(0x42));
  }

  #[test]
  fn read_u8_eof_on_empty() {
    let mut cur = Cursor::new(&[]);
    assert_eq!(
      cur.read_u8(),
      Err(ReadError::UnexpectedEof {
        needed: 1,
        available: 0
      })
    );
  }

  #[test]
  fn read_u16_le_known_value() {
    let mut cur = Cursor::new(&[0x02, 0x01]);
    assert_eq!(cur.read_u16_le(), Ok(0x0102));
  }

  #[test]
  fn read_u16_le_eof_one_byte_short() {
    let mut cur = Cursor::new(&[0xAA]);
    assert_eq!(
      cur.read_u16_le(),
      Err(ReadError::UnexpectedEof {
        needed: 2,
        available: 1
      })
    );
  }

  #[test]
  fn read_u32_le_known_value() {
    let mut cur = Cursor::new(&[0xEF, 0xBE, 0xAD, 0xDE]);
    assert_eq!(cur.read_u32_le(), Ok(0xDEADBEEF));
  }

  #[test]
  fn read_u32_le_eof_three_bytes_short() {
    let mut cur = Cursor::new(&[0xAA]);
    assert_eq!(
      cur.read_u32_le(),
      Err(ReadError::UnexpectedEof {
        needed: 4,
        available: 1
      })
    );
  }

  #[test]
  fn read_u64_le_known_value() {
    let mut cur = Cursor::new(&[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]);
    assert_eq!(cur.read_u64_le(), Ok(0x0102030405060708));
  }

  #[test]
  fn read_u64_le_eof() {
    let mut cur = Cursor::new(&[0x01, 0x02, 0x03]);
    assert_eq!(
      cur.read_u64_le(),
      Err(ReadError::UnexpectedEof {
        needed: 8,
        available: 3
      })
    );
  }

  #[test]
  fn read_i16_le_positive() {
    let mut cur = Cursor::new(&[0x39, 0x05]);
    assert_eq!(cur.read_i16_le(), Ok(1337));
  }

  #[test]
  fn read_i16_le_negative() {
    let mut cur = Cursor::new(&[0xFF, 0xFF]);
    assert_eq!(cur.read_i16_le(), Ok(-1));
  }

  #[test]
  fn read_i16_le_eof() {
    let mut cur = Cursor::new(&[0x01]);
    assert!(cur.read_i16_le().is_err());
  }

  #[test]
  fn read_i32_le_positive() {
    let mut cur = Cursor::new(&[0xA0, 0x86, 0x01, 0x00]);
    assert_eq!(cur.read_i32_le(), Ok(100_000));
  }

  #[test]
  fn read_i32_le_negative() {
    let mut cur = Cursor::new(&[0x00, 0xFF, 0xFF, 0xFF]);
    assert_eq!(cur.read_i32_le(), Ok(-256));
  }

  #[test]
  fn read_i32_le_eof() {
    let mut cur = Cursor::new(&[0x01, 0x02]);
    assert!(cur.read_i32_le().is_err());
  }

  #[test]
  fn read_i64_le_positive() {
    let mut cur = Cursor::new(&[0x01, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(cur.read_i64_le(), Ok(1));
  }

  #[test]
  fn read_i64_le_negative() {
    let mut cur = Cursor::new(&[0xFF; 8]);
    assert_eq!(cur.read_i64_le(), Ok(-1));
  }

  #[test]
  fn read_i64_le_eof() {
    let mut cur = Cursor::new(&[0x01; 7]);
    assert!(cur.read_i64_le().is_err());
  }

  #[test]
  fn read_bytes_returns_correct_slice() {
    let data = [0xAA, 0xBB, 0xCC, 0xDD];
    let mut cur = Cursor::new(&data);
    let _ = cur.read_u8();
    let slice = cur.read_bytes(2);
    assert_eq!(slice, Ok(&[0xBB, 0xCC][..]));
    assert_eq!(cur.position(), 3);
  }

  #[test]
  fn read_bytes_zero_length() {
    let mut cur = Cursor::new(&[0x01]);
    let slice = cur.read_bytes(0);
    assert_eq!(slice, Ok(&[][..]));
    assert_eq!(cur.position(), 0);
  }

  #[test]
  fn read_bytes_eof() {
    let mut cur = Cursor::new(&[0x01, 0x02]);
    assert_eq!(
      cur.read_bytes(3),
      Err(ReadError::UnexpectedEof {
        needed: 3,
        available: 2
      })
    );
  }

  #[test]
  fn read_array_returns_correct_fixed_array() {
    let mut cur = Cursor::new(&[0x01, 0x02, 0x03, 0x04]);
    let arr = cur.read_array::<4>();
    assert_eq!(arr, Ok([0x01, 0x02, 0x03, 0x04]));
    assert!(cur.is_empty());
  }

  #[test]
  fn read_array_partial_read_preserves_position() {
    let mut cur = Cursor::new(&[0xAA, 0xBB, 0xCC, 0xDD, 0xEE]);
    let _ = cur.read_u8();
    let arr = cur.read_array::<2>();
    assert_eq!(arr, Ok([0xBB, 0xCC]));
    assert_eq!(cur.position(), 3);
    assert_eq!(cur.remaining(), 2);
  }

  #[test]
  fn read_array_eof() {
    let mut cur = Cursor::new(&[0x01, 0x02]);
    assert!(cur.read_array::<4>().is_err());
  }

  #[test]
  fn sequential_reads_consume_entire_buffer() {
    let mut buf = Vec::new();
    buf.push(0xFF);
    buf.extend_from_slice(&0x0102u16.to_le_bytes());
    buf.extend_from_slice(&0xDEADBEEFu32.to_le_bytes());
    buf.push(0x42);

    let mut cur = Cursor::new(&buf);
    assert_eq!(cur.read_u8(), Ok(0xFF));
    assert_eq!(cur.read_u16_le(), Ok(0x0102));
    assert_eq!(cur.read_u32_le(), Ok(0xDEADBEEF));
    assert_eq!(cur.read_u8(), Ok(0x42));
    assert!(cur.is_empty());
  }

  #[test]
  fn failed_read_does_not_advance_position() {
    let mut cur = Cursor::new(&[0x01]);
    let pos_before = cur.position();
    let result = cur.read_u32_le();
    assert!(result.is_err());
    assert_eq!(cur.position(), pos_before);
  }

  // -- CompactSize canonical boundary values --

  #[test]
  fn compact_size_zero() {
    let mut cur = Cursor::new(&[0x00]);
    assert_eq!(cur.read_compact_size(), Ok(0));
  }

  #[test]
  fn compact_size_max_single_byte() {
    let mut cur = Cursor::new(&[0xFC]);
    assert_eq!(cur.read_compact_size(), Ok(252));
  }

  #[test]
  fn compact_size_min_three_byte() {
    let mut cur = Cursor::new(&[0xFD, 0xFD, 0x00]);
    assert_eq!(cur.read_compact_size(), Ok(253));
  }

  #[test]
  fn compact_size_max_three_byte() {
    let mut cur = Cursor::new(&[0xFD, 0xFF, 0xFF]);
    assert_eq!(cur.read_compact_size(), Ok(0xFFFF));
  }

  #[test]
  fn compact_size_min_five_byte() {
    let mut cur = Cursor::new(&[0xFE, 0x00, 0x00, 0x01, 0x00]);
    assert_eq!(cur.read_compact_size(), Ok(0x10000));
  }

  #[test]
  fn compact_size_max_five_byte() {
    let mut cur = Cursor::new(&[0xFE, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(cur.read_compact_size(), Ok(0xFFFFFFFF));
  }

  #[test]
  fn compact_size_min_nine_byte() {
    let mut cur = Cursor::new(&[0xFF, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00]);
    assert_eq!(cur.read_compact_size(), Ok(0x100000000));
  }

  #[test]
  fn compact_size_large_nine_byte() {
    let mut cur = Cursor::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF]);
    assert_eq!(cur.read_compact_size(), Ok(u64::MAX));
  }

  // -- CompactSize non-canonical rejection --

  #[test]
  fn compact_size_rejects_zero_in_three_byte() {
    let mut cur = Cursor::new(&[0xFD, 0x00, 0x00]);
    assert_eq!(
      cur.read_compact_size(),
      Err(ReadError::NonCanonicalCompactSize)
    );
  }

  #[test]
  fn compact_size_rejects_252_in_three_byte() {
    let mut cur = Cursor::new(&[0xFD, 0xFC, 0x00]);
    assert_eq!(
      cur.read_compact_size(),
      Err(ReadError::NonCanonicalCompactSize)
    );
  }

  #[test]
  fn compact_size_rejects_small_in_five_byte() {
    let mut cur = Cursor::new(&[0xFE, 0xFF, 0xFF, 0x00, 0x00]);
    assert_eq!(
      cur.read_compact_size(),
      Err(ReadError::NonCanonicalCompactSize)
    );
  }

  #[test]
  fn compact_size_rejects_zero_in_five_byte() {
    let mut cur = Cursor::new(&[0xFE, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(
      cur.read_compact_size(),
      Err(ReadError::NonCanonicalCompactSize)
    );
  }

  #[test]
  fn compact_size_rejects_small_in_nine_byte() {
    let mut cur = Cursor::new(&[0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(
      cur.read_compact_size(),
      Err(ReadError::NonCanonicalCompactSize)
    );
  }

  #[test]
  fn compact_size_rejects_zero_in_nine_byte() {
    let mut cur = Cursor::new(&[0xFF, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00]);
    assert_eq!(
      cur.read_compact_size(),
      Err(ReadError::NonCanonicalCompactSize)
    );
  }

  // -- CompactSize EOF conditions --

  #[test]
  fn compact_size_eof_empty() {
    let mut cur = Cursor::new(&[]);
    assert!(cur.read_compact_size().is_err());
  }

  #[test]
  fn compact_size_eof_mid_three_byte() {
    let mut cur = Cursor::new(&[0xFD, 0x01]);
    assert_eq!(
      cur.read_compact_size(),
      Err(ReadError::UnexpectedEof {
        needed: 2,
        available: 1
      })
    );
  }

  #[test]
  fn compact_size_eof_mid_five_byte() {
    let mut cur = Cursor::new(&[0xFE, 0x01, 0x02]);
    assert_eq!(
      cur.read_compact_size(),
      Err(ReadError::UnexpectedEof {
        needed: 4,
        available: 2
      })
    );
  }

  #[test]
  fn compact_size_eof_mid_nine_byte() {
    let mut cur = Cursor::new(&[0xFF, 0x01, 0x02, 0x03, 0x04, 0x05]);
    assert_eq!(
      cur.read_compact_size(),
      Err(ReadError::UnexpectedEof {
        needed: 8,
        available: 5
      })
    );
  }

  // -- read_compact_bytes --

  #[test]
  fn compact_bytes_reads_correct_data() {
    let mut cur = Cursor::new(&[0x03, 0xAA, 0xBB, 0xCC]);
    let result = cur.read_compact_bytes(100);
    assert_eq!(result, Ok(vec![0xAA, 0xBB, 0xCC]));
    assert!(cur.is_empty());
  }

  #[test]
  fn compact_bytes_zero_length() {
    let mut cur = Cursor::new(&[0x00]);
    let result = cur.read_compact_bytes(100);
    assert_eq!(result, Ok(vec![]));
  }

  #[test]
  fn compact_bytes_exceeds_max_len() {
    let mut cur = Cursor::new(&[0x05, 0x01, 0x02, 0x03, 0x04, 0x05]);
    assert_eq!(
      cur.read_compact_bytes(3),
      Err(ReadError::CompactSizeOverflow(5))
    );
  }

  #[test]
  fn compact_bytes_eof_in_data() {
    let mut cur = Cursor::new(&[0x05, 0x01, 0x02, 0x03]);
    assert_eq!(
      cur.read_compact_bytes(100),
      Err(ReadError::UnexpectedEof {
        needed: 5,
        available: 3
      })
    );
  }
}
