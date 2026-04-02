//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Transaction and special-tx payload types.

pub mod asset_lock;
pub mod asset_unlock;
pub mod coinbase;
pub mod mnhf;
pub mod proregtx;
pub mod proupregtx;
pub mod prouprevtx;
pub mod proupservtx;
pub mod quorum;
pub mod transaction;

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::PayloadError;
use crate::prelude::Vec;
use crate::types::tx::TxType;

use asset_lock::CAssetLockPayload;
use asset_unlock::CAssetUnlockPayload;
use coinbase::CCbTx;
use mnhf::MNHFTxPayload;
use proregtx::CProRegTx;
use proupregtx::CProUpRegTx;
use prouprevtx::CProUpRevTx;
use proupservtx::CProUpServTx;
use quorum::CFinalCommitmentTxPayload;

/// Dispatch enum for special tx payloads.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum SpecialPayload {
  /// ProRegTx (type 1).
  ProviderRegister(CProRegTx),
  /// ProUpServTx (type 2).
  ProviderUpdateService(CProUpServTx),
  /// ProUpRegTx (type 3).
  ProviderUpdateRegistrar(CProUpRegTx),
  /// ProUpRevTx (type 4).
  ProviderUpdateRevoke(CProUpRevTx),
  /// CCbTx (type 5).
  Coinbase(CCbTx),
  /// CFinalCommitmentTxPayload (type 6).
  QuorumCommitment(CFinalCommitmentTxPayload),
  /// MNHFTxPayload (type 7).
  MnhfSignal(MNHFTxPayload),
  /// CAssetLockPayload (type 8).
  AssetLock(CAssetLockPayload),
  /// CAssetUnlockPayload (type 9).
  AssetUnlock(CAssetUnlockPayload),
  /// Unrecognized type, kept as raw bytes.
  Unknown { tx_type: TxType, data: Vec<u8> },
}

/// Decode payload, rejecting trailing bytes.
fn decode_payload_exact<T: Decode>(data: &[u8]) -> Result<T, PayloadError> {
  let mut cursor = Cursor::new(data);
  let payload = T::consensus_decode(&mut cursor)?;
  if !cursor.is_empty() {
    return Err(PayloadError::Decode(
      crate::error::DecodeError::InvalidValue {
        field: "extra_payload",
        reason: "trailing bytes after payload decode",
      },
    ));
  }
  Ok(payload)
}

impl SpecialPayload {
  /// Dispatch by type code to typed payload.
  pub fn decode(tx_type: TxType, data: &[u8]) -> Result<Self, PayloadError> {
    match tx_type {
      TxType::Normal => Err(PayloadError::UnknownTxType(0)),
      TxType::ProviderRegister => Ok(Self::ProviderRegister(decode_payload_exact(data)?)),
      TxType::ProviderUpdateService => Ok(Self::ProviderUpdateService(decode_payload_exact(data)?)),
      TxType::ProviderUpdateRegistrar => {
        Ok(Self::ProviderUpdateRegistrar(decode_payload_exact(data)?))
      }
      TxType::ProviderUpdateRevoke => Ok(Self::ProviderUpdateRevoke(decode_payload_exact(data)?)),
      TxType::Coinbase => Ok(Self::Coinbase(decode_payload_exact(data)?)),
      TxType::QuorumCommitment => Ok(Self::QuorumCommitment(decode_payload_exact(data)?)),
      TxType::MnhfSignal => Ok(Self::MnhfSignal(decode_payload_exact(data)?)),
      TxType::AssetLock => Ok(Self::AssetLock(decode_payload_exact(data)?)),
      TxType::AssetUnlock => Ok(Self::AssetUnlock(decode_payload_exact(data)?)),
      TxType::Unknown(t) => Err(PayloadError::UnknownTxType(t)),
    }
  }
}
