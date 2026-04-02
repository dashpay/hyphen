//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Special transaction payload support types.

use crate::cursor::Cursor;
use crate::error::DecodeError;
use crate::prelude::Vec;

const LEGACY_CSERVICE_SIZE: usize = 18;
const MAX_EXT_NET_INFO_PURPOSES: u64 = 8;
const MAX_ENTRIES_PER_PURPOSE: u64 = 8;
const MAX_DOMAIN_LENGTH: usize = 256;
const MAX_DYNBITSET_BITS: u64 = 1024;

/// LLMQ type (quorum size/threshold).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum LlmqType {
  /// 50 members, 60% threshold.
  Llmq50_60,
  /// 400 members, 60% threshold.
  Llmq400_60,
  /// 400 members, 85% threshold.
  Llmq400_85,
  /// 100 members, 67% threshold.
  Llmq100_67,
  /// 60 members, 75% threshold.
  Llmq60_75,
  /// 25 members, 67% threshold.
  Llmq25_67,
  /// Regtest quorum.
  LlmqTest,
  /// Devnet quorum.
  LlmqDevnet,
  /// Test v17-era quorum.
  LlmqTestV17,
  /// Test InstantSend quorum.
  LlmqTestInstantsend,
  /// Test Platform quorum.
  LlmqTestPlatform,
  /// Devnet Platform quorum.
  LlmqDevnetPlatform,
  /// Unrecognized type code.
  Unknown(u8),
}

impl LlmqType {
  /// Converts a raw byte.
  #[inline]
  pub fn from_u8(val: u8) -> Self {
    match val {
      1 => Self::Llmq50_60,
      2 => Self::Llmq400_60,
      3 => Self::Llmq400_85,
      4 => Self::Llmq100_67,
      5 => Self::Llmq60_75,
      6 => Self::Llmq25_67,
      100 => Self::LlmqTest,
      101 => Self::LlmqDevnet,
      102 => Self::LlmqTestV17,
      104 => Self::LlmqTestInstantsend,
      106 => Self::LlmqTestPlatform,
      107 => Self::LlmqDevnetPlatform,
      other => Self::Unknown(other),
    }
  }
}

/// LSB-first dynamic bitset.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct DynBitset {
  pub num_bits: u64,
  pub data: Vec<u8>,
}

impl DynBitset {
  /// Decodes from cursor.
  pub fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let num_bits = cursor.read_compact_size()?;
    if num_bits > MAX_DYNBITSET_BITS {
      return Err(DecodeError::InvalidValue {
        field: "dynbitset_size",
        reason: "dynamic bitset exceeds maximum allowed bit count",
      });
    }
    let byte_len = num_bits.div_ceil(8) as usize;
    let data = cursor.read_bytes(byte_len)?.to_vec();
    Ok(Self { num_bits, data })
  }

  /// Bit at index, `None` if out of range.
  #[inline]
  pub fn get(&self, index: u64) -> Option<bool> {
    if index >= self.num_bits {
      return None;
    }
    let byte_idx = (index / 8) as usize;
    let bit_idx = (index % 8) as u32;
    Some(self.data[byte_idx] & (1u8 << bit_idx) != 0)
  }

  /// Counts set bits.
  pub fn count_ones(&self) -> u64 {
    if self.num_bits == 0 {
      return 0;
    }
    let full_bytes = (self.num_bits / 8) as usize;
    let remaining_bits = (self.num_bits % 8) as u32;

    let mut count: u64 = 0;
    for &byte in &self.data[..full_bytes] {
      count += u64::from(byte.count_ones());
    }
    if remaining_bits > 0 {
      let mask = (1u8 << remaining_bits) - 1;
      count += u64::from((self.data[full_bytes] & mask).count_ones());
    }
    count
  }

  /// Iterates set bit indices.
  pub fn iter_set_bits(&self) -> DynBitsetIterator<'_> {
    DynBitsetIterator {
      bitset: self,
      current: 0,
    }
  }
}

/// Iterator over set bit positions.
#[derive(Debug)]
pub struct DynBitsetIterator<'a> {
  bitset: &'a DynBitset,
  current: u64,
}

impl Iterator for DynBitsetIterator<'_> {
  type Item = u64;

  fn next(&mut self) -> Option<u64> {
    while self.current < self.bitset.num_bits {
      let idx = self.current;
      self.current += 1;
      if self.bitset.get(idx) == Some(true) {
        return Some(idx);
      }
    }
    None
  }
}

/// ADDRv2 network type.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum NetworkType {
  /// Ipv4, 4 bytes.
  Ipv4,
  /// Ipv6, 16 bytes.
  Ipv6,
  /// Tor v3, 32 bytes.
  TorV3,
  /// I2P, 32 bytes.
  I2P,
  /// Cjdns, 16 bytes.
  Cjdns,
  /// Unrecognized type code.
  Unknown(u8),
}

impl NetworkType {
  /// Converts an ADDRv2 ID byte.
  #[inline]
  pub fn from_addrv2_id(id: u8) -> Self {
    match id {
      0x01 => Self::Ipv4,
      0x02 => Self::Ipv6,
      0x04 => Self::TorV3,
      0x05 => Self::I2P,
      0x06 => Self::Cjdns,
      other => Self::Unknown(other),
    }
  }
}

/// Network address with port.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CService {
  pub network: NetworkType,
  pub addr: Vec<u8>,
  pub port: u16,
}

const IPV4_MAPPED_PREFIX: [u8; 12] = [0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF];

impl CService {
  /// Decodes from ADDRv1 (18 bytes).
  pub fn decode_addrv1(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let addr16 = cursor.read_array::<16>()?;
    let port = cursor.read_u16_be()?;

    if addr16[..12] == IPV4_MAPPED_PREFIX {
      Ok(Self {
        network: NetworkType::Ipv4,
        addr: addr16[12..16].to_vec(),
        port,
      })
    } else {
      Ok(Self {
        network: NetworkType::Ipv6,
        addr: addr16.to_vec(),
        port,
      })
    }
  }

  /// Decodes from ADDRv2 (BIP155).
  pub fn decode_addrv2(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let net_id = cursor.read_u8()?;
    let network = NetworkType::from_addrv2_id(net_id);
    let addr_len = cursor.read_compact_size()? as usize;
    if addr_len > 512 {
      return Err(DecodeError::InvalidValue {
        field: "addrv2_addr_len",
        reason: "address length exceeds maximum allowed size",
      });
    }
    let addr = cursor.read_bytes(addr_len)?.to_vec();
    let port = cursor.read_u16_be()?;

    Ok(Self {
      network,
      addr,
      port,
    })
  }
}

/// Purpose of a NetInfo entry.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum NetInfoPurpose {
  /// Core P2P address.
  CoreP2P,
  /// Platform P2P address.
  PlatformP2P,
  /// Platform HTTPS endpoint.
  PlatformHttps,
  /// Unrecognized purpose.
  Unknown(u8),
}

impl NetInfoPurpose {
  /// Converts a raw purpose byte.
  #[inline]
  pub fn from_u8(val: u8) -> Self {
    match val {
      0 => Self::CoreP2P,
      1 => Self::PlatformP2P,
      2 => Self::PlatformHttps,
      other => Self::Unknown(other),
    }
  }
}

/// Single NetInfo entry.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum NetInfoEntry {
  /// ADDRv2 service address.
  Service(CService),
  /// Domain name with port.
  Domain { name: Vec<u8>, port: u16 },
  /// Invalid/empty placeholder.
  Invalid,
}

impl NetInfoEntry {
  /// Decodes one entry.
  pub fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let entry_type = cursor.read_u8()?;
    match entry_type {
      0x01 => {
        let svc = CService::decode_addrv2(cursor)?;
        Ok(Self::Service(svc))
      }
      0x02 => {
        let name_len = cursor.read_compact_size()? as usize;
        if name_len > MAX_DOMAIN_LENGTH {
          return Err(DecodeError::InvalidValue {
            field: "net_info_domain_length",
            reason: "domain name exceeds maximum allowed length",
          });
        }
        let name = cursor.read_bytes(name_len)?.to_vec();
        let port = cursor.read_u16_be()?;
        Ok(Self::Domain { name, port })
      }
      0xFF => Ok(Self::Invalid),
      _ => Err(DecodeError::InvalidValue {
        field: "net_info_entry_type",
        reason: "unrecognized NetInfoEntry type byte",
      }),
    }
  }
}

/// Purpose-keyed NetInfo map.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct ExtendedNetInfo {
  pub version: u8,
  pub entries: Vec<(NetInfoPurpose, Vec<NetInfoEntry>)>,
}

impl ExtendedNetInfo {
  /// Decodes from cursor.
  pub fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = cursor.read_u8()?;
    let map_count = cursor.read_compact_size()?;
    if map_count > MAX_EXT_NET_INFO_PURPOSES {
      return Err(DecodeError::InvalidValue {
        field: "ext_net_info_map_count",
        reason: "too many purpose entries in ExtNetInfo",
      });
    }

    let mut entries = Vec::with_capacity(map_count as usize);
    for _ in 0..map_count {
      let purpose = NetInfoPurpose::from_u8(cursor.read_u8()?);
      let entry_count = cursor.read_compact_size()?;
      if entry_count > MAX_ENTRIES_PER_PURPOSE {
        return Err(DecodeError::InvalidValue {
          field: "ext_net_info_entry_count",
          reason: "too many entries per purpose in ExtNetInfo",
        });
      }
      let mut purpose_entries = Vec::with_capacity(entry_count as usize);
      for _ in 0..entry_count {
        purpose_entries.push(NetInfoEntry::consensus_decode(cursor)?);
      }
      entries.push((purpose, purpose_entries));
    }

    Ok(Self { version, entries })
  }
}

/// Masternode network info.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum NetInfo {
  /// ADDRv1 CService.
  Legacy(CService),
  /// Versioned purpose-keyed map.
  Extended(ExtendedNetInfo),
}

impl NetInfo {
  /// Decodes legacy (ADDRv1).
  pub fn decode_legacy(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let svc = CService::decode_addrv1(cursor)?;
    Ok(Self::Legacy(svc))
  }

  /// Decodes extended format.
  pub fn decode_extended(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let ext = ExtendedNetInfo::consensus_decode(cursor)?;
    Ok(Self::Extended(ext))
  }
}

/// Skips legacy NetInfo as raw bytes.
pub fn skip_legacy_net_info(cursor: &mut Cursor<'_>) -> Result<Vec<u8>, DecodeError> {
  let bytes = cursor.read_bytes(LEGACY_CSERVICE_SIZE)?;
  Ok(bytes.to_vec())
}

/// Skips extended NetInfo, returns raw bytes.
pub fn skip_extended_net_info(cursor: &mut Cursor<'_>) -> Result<Vec<u8>, DecodeError> {
  let start = cursor.position();

  let _ = cursor.read_u8()?;

  let map_count = cursor.read_compact_size()?;
  if map_count > MAX_EXT_NET_INFO_PURPOSES {
    return Err(DecodeError::InvalidValue {
      field: "ext_net_info_map_count",
      reason: "too many purpose entries in ExtNetInfo",
    });
  }

  for _ in 0..map_count {
    let _ = cursor.read_u8()?;

    let entry_count = cursor.read_compact_size()?;
    if entry_count > MAX_ENTRIES_PER_PURPOSE {
      return Err(DecodeError::InvalidValue {
        field: "ext_net_info_entry_count",
        reason: "too many entries per purpose in ExtNetInfo",
      });
    }

    for _ in 0..entry_count {
      let entry_type = cursor.read_u8()?;
      match entry_type {
        0x01 => {
          let _ = cursor.read_u8()?;
          let addr_len = cursor.read_compact_size()? as usize;
          let _ = cursor.read_bytes(addr_len)?;
          let _ = cursor.read_bytes(2)?;
        }
        0x02 => {
          let domain_len = cursor.read_compact_size()? as usize;
          if domain_len > MAX_DOMAIN_LENGTH {
            return Err(DecodeError::InvalidValue {
              field: "net_info_domain_length",
              reason: "domain name exceeds maximum allowed length",
            });
          }
          let _ = cursor.read_bytes(domain_len)?;
          let _ = cursor.read_bytes(2)?;
        }
        0xFF => {}
        _ => {
          return Err(DecodeError::InvalidValue {
            field: "net_info_entry_type",
            reason: "unrecognized NetInfoEntry type byte",
          });
        }
      }
    }
  }

  Ok(cursor.consumed_since(start).to_vec())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn skip_legacy_ipv4_mapped() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&[0x00; 10]);
    buf.extend_from_slice(&[0xFF, 0xFF]);
    buf.extend_from_slice(&[192, 168, 1, 1]);
    buf.extend_from_slice(&[0x27, 0x0F]);
    assert_eq!(buf.len(), 18);

    let mut cur = Cursor::new(&buf);
    let blob = skip_legacy_net_info(&mut cur).expect("should skip");
    assert_eq!(blob.len(), 18);
    assert_eq!(blob, buf);
    assert!(cur.is_empty());
  }

  #[test]
  fn skip_legacy_ipv6() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&[0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    buf.extend_from_slice(&[0x27, 0x0F]);
    assert_eq!(buf.len(), 18);

    let mut cur = Cursor::new(&buf);
    let blob = skip_legacy_net_info(&mut cur).expect("should skip");
    assert_eq!(blob.len(), 18);
    assert!(cur.is_empty());
  }

  #[test]
  fn skip_legacy_eof() {
    let buf = [0u8; 17];
    let mut cur = Cursor::new(&buf);
    assert!(skip_legacy_net_info(&mut cur).is_err());
  }

  #[test]
  fn skip_legacy_consumed_count() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&[0x00; 18]);
    buf.push(0xFF);
    let mut cur = Cursor::new(&buf);
    let blob = skip_legacy_net_info(&mut cur).expect("should skip");
    assert_eq!(blob.len(), 18);
    assert_eq!(cur.remaining(), 1);
  }

  fn make_ext_net_info_ipv4() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.push(0x01); // version
    buf.push(0x01); // map: 1 entry
    buf.push(0x00); // purpose: CORE_P2P
    buf.push(0x01); // vector: 1 entry
    buf.push(0x01); // entry type: Service
    buf.push(0x01); // network_id: Ipv4
    buf.push(0x04); // addr len: 4
    buf.extend_from_slice(&[192, 168, 1, 1]);
    buf.extend_from_slice(&[0x27, 0x0F]);
    buf
  }

  #[test]
  fn skip_extended_ipv4_service() {
    let data = make_ext_net_info_ipv4();
    let expected_len = data.len();
    let mut cur = Cursor::new(&data);
    let blob = skip_extended_net_info(&mut cur).expect("should skip");
    assert_eq!(blob.len(), expected_len);
    assert_eq!(blob, data);
    assert!(cur.is_empty());
  }

  #[test]
  fn skip_extended_ipv6_service() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x00);
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x02);
    buf.push(0x10);
    buf.extend_from_slice(&[0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    buf.extend_from_slice(&[0x1F, 0x41]);

    let expected_len = buf.len();
    let mut cur = Cursor::new(&buf);
    let blob = skip_extended_net_info(&mut cur).expect("should skip");
    assert_eq!(blob.len(), expected_len);
    assert!(cur.is_empty());
  }

  #[test]
  fn skip_extended_domain_entry() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x02);
    buf.push(0x01);
    buf.push(0x02);
    buf.push(0x0B);
    buf.extend_from_slice(b"example.com");
    buf.extend_from_slice(&[0x01, 0xBB]);

    let expected_len = buf.len();
    let mut cur = Cursor::new(&buf);
    let blob = skip_extended_net_info(&mut cur).expect("should skip");
    assert_eq!(blob.len(), expected_len);
    assert!(cur.is_empty());
  }

  #[test]
  fn skip_extended_multiple_purposes() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x02);

    buf.push(0x00);
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x04);
    buf.extend_from_slice(&[10, 0, 0, 1]);
    buf.extend_from_slice(&[0x27, 0x0F]);

    buf.push(0x02);
    buf.push(0x01);
    buf.push(0x02);
    buf.push(0x07);
    buf.extend_from_slice(b"foo.com");
    buf.extend_from_slice(&[0x00, 0x50]);

    let expected_len = buf.len();
    let mut cur = Cursor::new(&buf);
    let blob = skip_extended_net_info(&mut cur).expect("should skip");
    assert_eq!(blob.len(), expected_len);
    assert!(cur.is_empty());
  }

  #[test]
  fn skip_extended_empty_map() {
    let buf = [0x01, 0x00];
    let mut cur = Cursor::new(&buf);
    let blob = skip_extended_net_info(&mut cur).expect("should skip empty");
    assert_eq!(blob.len(), 2);
    assert!(cur.is_empty());
  }

  #[test]
  fn skip_extended_invalid_entry_type() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x00);
    buf.push(0x01);
    buf.push(0x03);

    let mut cur = Cursor::new(&buf);
    assert!(skip_extended_net_info(&mut cur).is_err());
  }

  #[test]
  fn skip_extended_eof_mid_service() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x00);
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x04);
    buf.extend_from_slice(&[192, 168]);

    let mut cur = Cursor::new(&buf);
    assert!(skip_extended_net_info(&mut cur).is_err());
  }

  #[test]
  fn skip_extended_invalid_entry_0xff() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x00);
    buf.push(0x01);
    buf.push(0xFF);

    let expected_len = buf.len();
    let mut cur = Cursor::new(&buf);
    let blob = skip_extended_net_info(&mut cur).expect("should skip");
    assert_eq!(blob.len(), expected_len);
    assert!(cur.is_empty());
  }

  #[test]
  fn dynbitset_get_known_bits() {
    let data = [0x08, 0xB2];
    let mut cur = Cursor::new(&data);
    let bs = DynBitset::consensus_decode(&mut cur).expect("should decode");
    assert_eq!(bs.num_bits, 8);
    assert_eq!(bs.get(0), Some(false));
    assert_eq!(bs.get(1), Some(true));
    assert_eq!(bs.get(2), Some(false));
    assert_eq!(bs.get(3), Some(false));
    assert_eq!(bs.get(4), Some(true));
    assert_eq!(bs.get(5), Some(true));
    assert_eq!(bs.get(6), Some(false));
    assert_eq!(bs.get(7), Some(true));
  }

  #[test]
  fn dynbitset_get_oob_returns_none() {
    let data = [0x04, 0x0F];
    let mut cur = Cursor::new(&data);
    let bs = DynBitset::consensus_decode(&mut cur).expect("should decode");
    assert_eq!(bs.get(3), Some(true));
    assert_eq!(bs.get(4), None);
    assert_eq!(bs.get(100), None);
  }

  #[test]
  fn dynbitset_count_ones_byte_aligned() {
    let data = [0x08, 0xFF];
    let mut cur = Cursor::new(&data);
    let bs = DynBitset::consensus_decode(&mut cur).expect("should decode");
    assert_eq!(bs.count_ones(), 8);
  }

  #[test]
  fn dynbitset_count_ones_non_aligned() {
    let data = [0x05, 0xFF];
    let mut cur = Cursor::new(&data);
    let bs = DynBitset::consensus_decode(&mut cur).expect("should decode");
    assert_eq!(bs.count_ones(), 5);
  }

  #[test]
  fn dynbitset_count_ones_empty() {
    let data = [0x00];
    let mut cur = Cursor::new(&data);
    let bs = DynBitset::consensus_decode(&mut cur).expect("should decode");
    assert_eq!(bs.count_ones(), 0);
  }

  #[test]
  fn dynbitset_iter_set_bits() {
    let data = [0x08, 0xB2];
    let mut cur = Cursor::new(&data);
    let bs = DynBitset::consensus_decode(&mut cur).expect("should decode");
    let set_bits: Vec<u64> = bs.iter_set_bits().collect();
    assert_eq!(set_bits, vec![1, 4, 5, 7]);
  }

  #[test]
  fn dynbitset_iter_set_bits_all_set() {
    let data = [0x04, 0x0F];
    let mut cur = Cursor::new(&data);
    let bs = DynBitset::consensus_decode(&mut cur).expect("should decode");
    let set_bits: Vec<u64> = bs.iter_set_bits().collect();
    assert_eq!(set_bits, vec![0, 1, 2, 3]);
  }

  #[test]
  fn dynbitset_iter_set_bits_none_set() {
    let data = [0x08, 0x00];
    let mut cur = Cursor::new(&data);
    let bs = DynBitset::consensus_decode(&mut cur).expect("should decode");
    let set_bits: Vec<u64> = bs.iter_set_bits().collect();
    assert!(set_bits.is_empty());
  }

  #[test]
  fn dynbitset_multi_byte() {
    let data = [0x0A, 0xFF, 0x03];
    let mut cur = Cursor::new(&data);
    let bs = DynBitset::consensus_decode(&mut cur).expect("should decode");
    assert_eq!(bs.count_ones(), 10);
    assert_eq!(bs.num_bits, 10);
    let set_bits: Vec<u64> = bs.iter_set_bits().collect();
    assert_eq!(set_bits, vec![0, 1, 2, 3, 4, 5, 6, 7, 8, 9]);
  }

  #[test]
  fn cservice_addrv1_ipv4() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
    buf.extend_from_slice(&[192, 168, 1, 1]);
    buf.extend_from_slice(&[0x27, 0x0F]);

    let mut cur = Cursor::new(&buf);
    let svc = CService::decode_addrv1(&mut cur).expect("should decode Ipv4");
    assert_eq!(svc.network, NetworkType::Ipv4);
    assert_eq!(svc.addr, &[192, 168, 1, 1]);
    assert_eq!(svc.port, 9999);
    assert!(cur.is_empty());
  }

  #[test]
  fn cservice_addrv1_ipv6() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&[0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    buf.extend_from_slice(&[0x1F, 0x41]);

    let mut cur = Cursor::new(&buf);
    let svc = CService::decode_addrv1(&mut cur).expect("should decode Ipv6");
    assert_eq!(svc.network, NetworkType::Ipv6);
    assert_eq!(svc.addr.len(), 16);
    assert_eq!(svc.addr[0], 0x20);
    assert_eq!(svc.addr[1], 0x01);
    assert_eq!(svc.port, 8001);
    assert!(cur.is_empty());
  }

  #[test]
  fn cservice_addrv1_port_be_verification() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
    buf.extend_from_slice(&[127, 0, 0, 1]);
    buf.extend_from_slice(&[0x00, 0x50]);

    let mut cur = Cursor::new(&buf);
    let svc = CService::decode_addrv1(&mut cur).expect("should decode");
    assert_eq!(svc.port, 80);
  }

  #[test]
  fn cservice_addrv1_eof() {
    let buf = [0u8; 17];
    let mut cur = Cursor::new(&buf);
    assert!(CService::decode_addrv1(&mut cur).is_err());
  }

  #[test]
  fn cservice_addrv2_ipv4() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x04);
    buf.extend_from_slice(&[192, 168, 1, 1]);
    buf.extend_from_slice(&[0x27, 0x0F]);

    let mut cur = Cursor::new(&buf);
    let svc = CService::decode_addrv2(&mut cur).expect("should decode Ipv4");
    assert_eq!(svc.network, NetworkType::Ipv4);
    assert_eq!(svc.addr, &[192, 168, 1, 1]);
    assert_eq!(svc.port, 9999);
    assert!(cur.is_empty());
  }

  #[test]
  fn cservice_addrv2_ipv6() {
    let mut buf = Vec::new();
    buf.push(0x02);
    buf.push(0x10);
    buf.extend_from_slice(&[0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    buf.extend_from_slice(&[0x1F, 0x41]);

    let mut cur = Cursor::new(&buf);
    let svc = CService::decode_addrv2(&mut cur).expect("should decode Ipv6");
    assert_eq!(svc.network, NetworkType::Ipv6);
    assert_eq!(svc.addr.len(), 16);
    assert_eq!(svc.port, 8001);
    assert!(cur.is_empty());
  }

  #[test]
  fn cservice_addrv2_torv3() {
    let mut buf = Vec::new();
    buf.push(0x04);
    buf.push(0x20);
    buf.extend_from_slice(&[0xAA; 32]);
    buf.extend_from_slice(&[0x00, 0x50]);

    let mut cur = Cursor::new(&buf);
    let svc = CService::decode_addrv2(&mut cur).expect("should decode TorV3");
    assert_eq!(svc.network, NetworkType::TorV3);
    assert_eq!(svc.addr.len(), 32);
    assert_eq!(svc.port, 80);
    assert!(cur.is_empty());
  }

  #[test]
  fn cservice_addrv2_unknown_network() {
    let mut buf = Vec::new();
    buf.push(0x99);
    buf.push(0x02);
    buf.extend_from_slice(&[0xBB, 0xCC]);
    buf.extend_from_slice(&[0x01, 0xBB]);

    let mut cur = Cursor::new(&buf);
    let svc = CService::decode_addrv2(&mut cur).expect("should decode unknown");
    assert_eq!(svc.network, NetworkType::Unknown(0x99));
    assert_eq!(svc.addr, &[0xBB, 0xCC]);
    assert_eq!(svc.port, 443);
  }

  #[test]
  fn cservice_addrv2_eof() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x04);
    buf.extend_from_slice(&[192, 168]);

    let mut cur = Cursor::new(&buf);
    assert!(CService::decode_addrv2(&mut cur).is_err());
  }

  #[test]
  fn netinfo_legacy_ipv4() {
    let mut buf = Vec::new();
    buf.extend_from_slice(&[0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xFF, 0xFF]);
    buf.extend_from_slice(&[10, 0, 0, 1]);
    buf.extend_from_slice(&[0x27, 0x0F]);

    let mut cur = Cursor::new(&buf);
    let ni = NetInfo::decode_legacy(&mut cur).expect("should decode legacy");
    match ni {
      NetInfo::Legacy(svc) => {
        assert_eq!(svc.network, NetworkType::Ipv4);
        assert_eq!(svc.addr, &[10, 0, 0, 1]);
        assert_eq!(svc.port, 9999);
      }
      NetInfo::Extended(_) => panic!("expected Legacy"),
    }
    assert!(cur.is_empty());
  }

  #[test]
  fn netinfo_extended_single_service() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x00);
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x04);
    buf.extend_from_slice(&[192, 168, 1, 1]);
    buf.extend_from_slice(&[0x27, 0x0F]);

    let mut cur = Cursor::new(&buf);
    let ni = NetInfo::decode_extended(&mut cur).expect("should decode ext");
    match ni {
      NetInfo::Extended(ext) => {
        assert_eq!(ext.version, 1);
        assert_eq!(ext.entries.len(), 1);
        assert_eq!(ext.entries[0].0, NetInfoPurpose::CoreP2P);
        assert_eq!(ext.entries[0].1.len(), 1);
        match &ext.entries[0].1[0] {
          NetInfoEntry::Service(svc) => {
            assert_eq!(svc.network, NetworkType::Ipv4);
            assert_eq!(svc.port, 9999);
          }
          _ => panic!("expected Service entry"),
        }
      }
      NetInfo::Legacy(_) => panic!("expected Extended"),
    }
    assert!(cur.is_empty());
  }

  #[test]
  fn netinfo_extended_with_domain() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x02);
    buf.push(0x01);
    buf.push(0x02);
    buf.push(0x07);
    buf.extend_from_slice(b"foo.com");
    buf.extend_from_slice(&[0x01, 0xBB]);

    let mut cur = Cursor::new(&buf);
    let ni = NetInfo::decode_extended(&mut cur).expect("should decode domain");
    match ni {
      NetInfo::Extended(ext) => {
        assert_eq!(ext.entries[0].0, NetInfoPurpose::PlatformHttps);
        match &ext.entries[0].1[0] {
          NetInfoEntry::Domain { name, port } => {
            assert_eq!(name, b"foo.com");
            assert_eq!(*port, 443);
          }
          _ => panic!("expected Domain entry"),
        }
      }
      NetInfo::Legacy(_) => panic!("expected Extended"),
    }
  }

  #[test]
  fn netinfo_extended_multiple_purposes() {
    let mut buf = Vec::new();
    buf.push(0x01);
    buf.push(0x02);
    // CORE_P2P with Ipv4
    buf.push(0x00);
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x04);
    buf.extend_from_slice(&[10, 0, 0, 1]);
    buf.extend_from_slice(&[0x27, 0x0F]);
    // PLATFORM_P2P with Ipv6
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x01);
    buf.push(0x02);
    buf.push(0x10);
    buf.extend_from_slice(&[0x20, 0x01, 0x0D, 0xB8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
    buf.extend_from_slice(&[0x1F, 0x41]);

    let mut cur = Cursor::new(&buf);
    let ni = NetInfo::decode_extended(&mut cur).expect("should decode");
    match ni {
      NetInfo::Extended(ext) => {
        assert_eq!(ext.entries.len(), 2);
        assert_eq!(ext.entries[0].0, NetInfoPurpose::CoreP2P);
        assert_eq!(ext.entries[1].0, NetInfoPurpose::PlatformP2P);
      }
      NetInfo::Legacy(_) => panic!("expected Extended"),
    }
    assert!(cur.is_empty());
  }
}
