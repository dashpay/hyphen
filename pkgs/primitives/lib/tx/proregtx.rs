//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CProRegTx registration payload (type 1).

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::prelude::Vec;
use crate::types::common::{BlsPublicKey, InputsHash, KeyId, PlatformNodeId, Script};
use crate::types::sptx::NetInfo;
use crate::types::tx::{MnType, OutPoint};

const MAX_VCH_SIG_SIZE: usize = 256;

/// CProRegTx -- register a new masternode (type 1).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CProRegTx {
  /// 1=LegacyBLS, 2=BasicBLS, 3=ExtAddr.
  pub version: u16,
  pub mn_type: MnType,
  /// Reserved, always 0.
  pub mode: u16,
  pub collateral_outpoint: OutPoint,
  /// Legacy CService or extended NetInfo.
  pub net_info: NetInfo,
  pub key_id_owner: KeyId,
  pub pub_key_operator: BlsPublicKey,
  pub key_id_voting: KeyId,
  /// Basis points (0-10000).
  pub operator_reward: u16,
  pub script_payout: Script,
  pub inputs_hash: InputsHash,
  /// Evo only.
  pub platform_node_id: Option<PlatformNodeId>,
  /// Evo + version < 3 only.
  pub platform_p2p_port: Option<u16>,
  /// Evo + version < 3 only.
  pub platform_http_port: Option<u16>,
  /// Owner ECDSA sig (variable-length).
  pub vch_sig: Vec<u8>,
}

impl Decode for CProRegTx {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = u16::consensus_decode(cursor)?;
    let mn_type = MnType::from_u16(u16::consensus_decode(cursor)?);
    let mode = u16::consensus_decode(cursor)?;
    let collateral_outpoint = OutPoint::consensus_decode(cursor)?;

    let net_info = if version >= 3 {
      NetInfo::decode_extended(cursor)?
    } else {
      NetInfo::decode_legacy(cursor)?
    };

    let key_id_owner = KeyId::consensus_decode(cursor)?;
    let pub_key_operator = BlsPublicKey::consensus_decode(cursor)?;
    let key_id_voting = KeyId::consensus_decode(cursor)?;
    let operator_reward = u16::consensus_decode(cursor)?;
    let script_payout = Script::consensus_decode(cursor)?;
    let inputs_hash = InputsHash::consensus_decode(cursor)?;

    let (platform_node_id, platform_p2p_port, platform_http_port) = if mn_type == MnType::Evo {
      let node_id = PlatformNodeId::consensus_decode(cursor)?;
      if version < 3 {
        let p2p = u16::consensus_decode(cursor)?;
        let http = u16::consensus_decode(cursor)?;
        (Some(node_id), Some(p2p), Some(http))
      } else {
        (Some(node_id), None, None)
      }
    } else {
      (None, None, None)
    };

    let vch_sig = cursor.read_compact_bytes(MAX_VCH_SIG_SIZE)?;

    Ok(Self {
      version,
      mn_type,
      mode,
      collateral_outpoint,
      net_info,
      key_id_owner,
      pub_key_operator,
      key_id_voting,
      operator_reward,
      script_payout,
      inputs_hash,
      platform_node_id,
      platform_p2p_port,
      platform_http_port,
      vch_sig,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::types::common::TxHash;

  fn make_common_prefix(version: u16, mn_type: u16) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&version.to_le_bytes());
    buf.extend_from_slice(&mn_type.to_le_bytes());
    buf.extend_from_slice(&0u16.to_le_bytes()); // mode = 0
                                                // collateral_outpoint: hash(32) + index(4)
    buf.extend_from_slice(&[0x11; 32]);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
  }

  fn append_keys_and_payout(buf: &mut Vec<u8>) {
    buf.extend_from_slice(&[0xAA; 20]); // key_id_owner
    buf.extend_from_slice(&[0xBB; 48]); // pub_key_operator
    buf.extend_from_slice(&[0xCC; 20]); // key_id_voting
    buf.extend_from_slice(&100u16.to_le_bytes()); // operator_reward
    buf.push(0x00); // empty script_payout
    buf.extend_from_slice(&[0xDD; 32]); // inputs_hash
  }

  fn make_v1_regular() -> Vec<u8> {
    let mut buf = make_common_prefix(1, 0);
    buf.extend_from_slice(&[0x00; 18]); // legacy NetInfo
    append_keys_and_payout(&mut buf);
    buf.push(0x03);
    buf.extend_from_slice(&[0xEE; 3]);
    buf
  }

  fn make_v2_evo() -> Vec<u8> {
    let mut buf = make_common_prefix(2, 1);
    buf.extend_from_slice(&[0x00; 18]); // legacy NetInfo
    append_keys_and_payout(&mut buf);
    buf.extend_from_slice(&[0xFF; 20]); // platform_node_id
    buf.extend_from_slice(&8001u16.to_le_bytes());
    buf.extend_from_slice(&8002u16.to_le_bytes());
    buf.push(0x00); // empty sig
    buf
  }

  fn make_v3_evo() -> Vec<u8> {
    let mut buf = make_common_prefix(3, 1);
    // Extended NetInfo: version(1) + empty map
    buf.push(0x01);
    buf.push(0x00);
    append_keys_and_payout(&mut buf);
    buf.extend_from_slice(&[0xFF; 20]); // platform_node_id
    buf.push(0x00); // empty sig
    buf
  }

  #[test]
  fn decode_proregtx_v1_regular() {
    let data = make_v1_regular();
    let mut cur = Cursor::new(&data);
    let payload = CProRegTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v1 regular");
    assert_eq!(payload.version, 1);
    assert_eq!(payload.mn_type, MnType::Regular);
    assert_eq!(payload.mode, 0);
    assert_eq!(payload.collateral_outpoint.hash, TxHash([0x11; 32]));
    assert_eq!(payload.collateral_outpoint.index, 0);
    assert!(matches!(payload.net_info, NetInfo::Legacy(_)));
    assert_eq!(payload.key_id_owner, KeyId([0xAA; 20]));
    assert_eq!(payload.pub_key_operator, BlsPublicKey([0xBB; 48]));
    assert_eq!(payload.key_id_voting, KeyId([0xCC; 20]));
    assert_eq!(payload.operator_reward, 100);
    assert_eq!(payload.inputs_hash, InputsHash([0xDD; 32]));
    assert!(payload.platform_node_id.is_none());
    assert!(payload.platform_p2p_port.is_none());
    assert_eq!(payload.vch_sig, &[0xEE; 3]);
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_proregtx_v2_evo() {
    let data = make_v2_evo();
    let mut cur = Cursor::new(&data);
    let payload = CProRegTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v2 evo");
    assert_eq!(payload.version, 2);
    assert_eq!(payload.mn_type, MnType::Evo);
    assert_eq!(payload.platform_node_id, Some(PlatformNodeId([0xFF; 20])));
    assert_eq!(payload.platform_p2p_port, Some(8001));
    assert_eq!(payload.platform_http_port, Some(8002));
    assert!(payload.vch_sig.is_empty());
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_proregtx_v3_evo() {
    let data = make_v3_evo();
    let mut cur = Cursor::new(&data);
    let payload = CProRegTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v3 evo");
    assert_eq!(payload.version, 3);
    assert_eq!(payload.mn_type, MnType::Evo);
    assert!(matches!(payload.net_info, NetInfo::Extended(_)));
    assert_eq!(payload.platform_node_id, Some(PlatformNodeId([0xFF; 20])));
    assert!(payload.platform_p2p_port.is_none());
    assert!(payload.platform_http_port.is_none());
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_proregtx_eof() {
    let data = make_v1_regular();
    let mut cur = Cursor::new(&data[..10]);
    assert!(CProRegTx::consensus_decode(&mut cur).is_err());
  }
}
