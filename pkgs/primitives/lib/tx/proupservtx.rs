//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CProUpServTx service-update payload (type 2).

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::types::common::{BlsSignature, InputsHash, PlatformNodeId, Script, TxHash};
use crate::types::sptx::NetInfo;
use crate::types::tx::MnType;

/// CProUpServTx -- update MN service addr (type 2).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct CProUpServTx {
  /// 1=LegacyBLS, 2=BasicBLS, 3=ExtAddr.
  pub version: u16,
  /// v2+ only; defaults to Regular.
  pub mn_type: MnType,
  pub pro_tx_hash: TxHash,
  /// Legacy CService or extended NetInfo.
  pub net_info: NetInfo,
  pub script_operator_payout: Script,
  pub inputs_hash: InputsHash,
  /// Evo only.
  pub platform_node_id: Option<PlatformNodeId>,
  /// Evo + version < 3 only.
  pub platform_p2p_port: Option<u16>,
  /// Evo + version < 3 only.
  pub platform_http_port: Option<u16>,
  /// Operator BLS signature.
  pub sig: BlsSignature,
}

impl Decode for CProUpServTx {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    let version = u16::consensus_decode(cursor)?;

    let mn_type = if version >= 2 {
      MnType::from_u16(u16::consensus_decode(cursor)?)
    } else {
      MnType::Regular
    };

    let pro_tx_hash = TxHash::consensus_decode(cursor)?;

    let net_info = if version >= 3 {
      NetInfo::decode_extended(cursor)?
    } else {
      NetInfo::decode_legacy(cursor)?
    };

    let script_operator_payout = Script::consensus_decode(cursor)?;
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

    let sig = BlsSignature::consensus_decode(cursor)?;

    Ok(Self {
      version,
      mn_type,
      pro_tx_hash,
      net_info,
      script_operator_payout,
      inputs_hash,
      platform_node_id,
      platform_p2p_port,
      platform_http_port,
      sig,
    })
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::prelude::Vec;

  fn make_v1_regular() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1u16.to_le_bytes());
    // no mn_type for v1
    buf.extend_from_slice(&[0x11; 32]); // pro_tx_hash
    buf.extend_from_slice(&[0x00; 18]); // legacy NetInfo
    buf.push(0x00); // empty script_operator_payout
    buf.extend_from_slice(&[0x22; 32]); // inputs_hash
    buf.extend_from_slice(&[0x33; 96]); // sig
    buf
  }

  fn make_v2_evo() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&2u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // mn_type = Evo
    buf.extend_from_slice(&[0x11; 32]); // pro_tx_hash
    buf.extend_from_slice(&[0x00; 18]); // legacy NetInfo
    buf.push(0x00); // empty script_operator_payout
    buf.extend_from_slice(&[0x22; 32]); // inputs_hash
    buf.extend_from_slice(&[0x44; 20]); // platform_node_id
    buf.extend_from_slice(&8001u16.to_le_bytes());
    buf.extend_from_slice(&8002u16.to_le_bytes());
    buf.extend_from_slice(&[0x33; 96]); // sig
    buf
  }

  fn make_v3_evo_ext() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&3u16.to_le_bytes());
    buf.extend_from_slice(&1u16.to_le_bytes()); // mn_type = Evo
    buf.extend_from_slice(&[0x11; 32]); // pro_tx_hash
                                        // Extended NetInfo: version(1) + empty map
    buf.push(0x01);
    buf.push(0x00);
    buf.push(0x00); // empty script_operator_payout
    buf.extend_from_slice(&[0x22; 32]); // inputs_hash
    buf.extend_from_slice(&[0x44; 20]); // platform_node_id
    buf.extend_from_slice(&[0x33; 96]); // sig
    buf
  }

  #[test]
  fn decode_proupservtx_v1_regular() {
    let data = make_v1_regular();
    let mut cur = Cursor::new(&data);
    let payload = CProUpServTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v1 regular");
    assert_eq!(payload.version, 1);
    assert_eq!(payload.mn_type, MnType::Regular);
    assert_eq!(payload.pro_tx_hash, TxHash([0x11; 32]));
    assert!(matches!(payload.net_info, NetInfo::Legacy(_)));
    assert_eq!(payload.inputs_hash, InputsHash([0x22; 32]));
    assert!(payload.platform_node_id.is_none());
    assert!(payload.platform_p2p_port.is_none());
    assert!(payload.platform_http_port.is_none());
    assert_eq!(payload.sig, BlsSignature([0x33; 96]));
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_proupservtx_v2_evo() {
    let data = make_v2_evo();
    let mut cur = Cursor::new(&data);
    let payload = CProUpServTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v2 evo");
    assert_eq!(payload.version, 2);
    assert_eq!(payload.mn_type, MnType::Evo);
    assert_eq!(payload.platform_node_id, Some(PlatformNodeId([0x44; 20])));
    assert_eq!(payload.platform_p2p_port, Some(8001));
    assert_eq!(payload.platform_http_port, Some(8002));
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_proupservtx_v3_evo_no_ports() {
    let data = make_v3_evo_ext();
    let mut cur = Cursor::new(&data);
    let payload = CProUpServTx::consensus_decode(&mut cur);
    let payload = payload.expect("should decode v3 evo");
    assert_eq!(payload.version, 3);
    assert_eq!(payload.mn_type, MnType::Evo);
    assert!(matches!(payload.net_info, NetInfo::Extended(_)));
    assert_eq!(payload.platform_node_id, Some(PlatformNodeId([0x44; 20])));
    assert!(payload.platform_p2p_port.is_none());
    assert!(payload.platform_http_port.is_none());
    assert!(cur.is_empty());
  }

  #[test]
  fn decode_proupservtx_eof() {
    let data = make_v1_regular();
    let mut cur = Cursor::new(&data[..10]);
    assert!(CProUpServTx::consensus_decode(&mut cur).is_err());
  }
}
