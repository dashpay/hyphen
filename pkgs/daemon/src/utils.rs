//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared byte-order helpers for public hash identifiers.

/// The byte order used by one hash-like identifier in memory or storage.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HashEndian {
  Ser,
  View,
}

impl HashEndian {
  const fn other(self) -> Self {
    match self {
      Self::Ser => Self::View,
      Self::View => Self::Ser,
    }
  }
}

/// One decoded hash identifier with both serialized and display byte orders.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HashLookup {
  pub ser_endian: Vec<u8>,
  pub view_endian: Vec<u8>,
}

impl HashLookup {
  /// Decode one 64-char hex identifier and materialize both byte orders.
  ///
  /// The raw hex string is interpreted as view order. The serialized order is
  /// derived by reversing those bytes. Callers that need to accept either
  /// input order can try both fields against the backing store.
  pub fn from_hex(hash_hex: &str) -> Result<Self, hex::FromHexError> {
    let view_endian = hex::decode(hash_hex)?;
    Ok(Self {
      ser_endian: ser_endian(&view_endian, HashEndian::View),
      view_endian,
    })
  }

  pub fn bytes(&self, endian: HashEndian) -> &[u8] {
    match endian {
      HashEndian::Ser => &self.ser_endian,
      HashEndian::View => &self.view_endian,
    }
  }

  pub fn ordered_candidates(&self, preferred: HashEndian) -> (Vec<u8>, Option<Vec<u8>>) {
    let primary = self.bytes(preferred).to_vec();
    let secondary = if self.ser_endian == self.view_endian {
      None
    } else {
      Some(self.bytes(preferred.other()).to_vec())
    };
    (primary, secondary)
  }
}

/// Convert one byte slice to serialized order.
pub fn ser_endian(bytes: &[u8], stored_as: HashEndian) -> Vec<u8> {
  reorder(bytes, stored_as, HashEndian::Ser)
}

/// Convert one byte slice to display order.
pub fn view_endian(bytes: &[u8], stored_as: HashEndian) -> Vec<u8> {
  reorder(bytes, stored_as, HashEndian::View)
}

/// Encode one byte slice as a public display-order hex string.
pub fn view_hex(bytes: &[u8], stored_as: HashEndian) -> String {
  hex::encode(view_endian(bytes, stored_as))
}

/// Decode one hex string into the requested byte order.
pub fn decode_hex_to(hash_hex: &str, target: HashEndian) -> Result<Vec<u8>, hex::FromHexError> {
  let bytes = hex::decode(hash_hex)?;
  Ok(reorder(&bytes, HashEndian::View, target))
}

fn reorder(bytes: &[u8], from: HashEndian, to: HashEndian) -> Vec<u8> {
  if from == to {
    return bytes.to_vec();
  }
  let mut reordered = bytes.to_vec();
  reordered.reverse();
  reordered
}

#[cfg(test)]
mod tests {
  use super::{decode_hex_to, view_hex, HashEndian, HashLookup};

  #[test]
  fn hash_lookup_materializes_both_orders() {
    let lookup = HashLookup::from_hex("00112233").expect("hex should decode");
    assert_eq!(lookup.view_endian, vec![0x00, 0x11, 0x22, 0x33]);
    assert_eq!(lookup.ser_endian, vec![0x33, 0x22, 0x11, 0x00]);
  }

  #[test]
  fn view_and_ser_hex_follow_declared_storage_order() {
    let bytes = vec![0x33, 0x22, 0x11, 0x00];
    assert_eq!(view_hex(&bytes, HashEndian::Ser), "00112233");
    assert_eq!(view_hex(&bytes, HashEndian::View), "33221100");
  }

  #[test]
  fn decode_hex_to_reorders_when_needed() {
    assert_eq!(
      decode_hex_to("00112233", HashEndian::Ser).expect("hex should decode"),
      vec![0x33, 0x22, 0x11, 0x00]
    );
  }

  #[test]
  fn ordered_candidates_include_opposite_endian() {
    let lookup = HashLookup::from_hex("00112233").expect("hex should decode");
    let (primary, secondary) = lookup.ordered_candidates(HashEndian::Ser);
    assert_eq!(primary, vec![0x33, 0x22, 0x11, 0x00]);
    assert_eq!(secondary, Some(vec![0x00, 0x11, 0x22, 0x33]));
  }
}
