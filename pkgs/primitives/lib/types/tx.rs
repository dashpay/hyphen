//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Transaction wire format types.

use crate::cursor::Cursor;
use crate::decode::Decode;
use crate::error::DecodeError;
use crate::prelude::Vec;
use crate::types::common::{Script, TxHash};

const MAX_TX_INPUTS: usize = 100_000;
const MAX_TX_OUTPUTS: usize = 100_000;
const MAX_EXTRA_PAYLOAD: usize = 100_000;

/// Dash transaction type code (upper 16 bits).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum TxType {
  /// Standard transaction, type 0.
  Normal,
  /// Masternode registration, type 1.
  ProviderRegister,
  /// Service address update, type 2.
  ProviderUpdateService,
  /// Registrar key update, type 3.
  ProviderUpdateRegistrar,
  /// Masternode revocation, type 4.
  ProviderUpdateRevoke,
  /// Coinbase with MN list commit, type 5.
  Coinbase,
  /// LLMQ final commitment, type 6.
  QuorumCommitment,
  /// MN hard-fork signal, type 7.
  MnhfSignal,
  /// Funds locked to Platform, type 8.
  AssetLock,
  /// Funds withdrawn from Platform, type 9.
  AssetUnlock,
  /// Unrecognized type for forward compat.
  Unknown(u16),
}

impl TxType {
  /// Converts a raw 16-bit type code.
  #[inline]
  pub fn from_u16(val: u16) -> Self {
    match val {
      0 => Self::Normal,
      1 => Self::ProviderRegister,
      2 => Self::ProviderUpdateService,
      3 => Self::ProviderUpdateRegistrar,
      4 => Self::ProviderUpdateRevoke,
      5 => Self::Coinbase,
      6 => Self::QuorumCommitment,
      7 => Self::MnhfSignal,
      8 => Self::AssetLock,
      9 => Self::AssetUnlock,
      other => Self::Unknown(other),
    }
  }

  /// Returns the raw 16-bit type code.
  #[inline]
  pub fn to_u16(self) -> u16 {
    match self {
      Self::Normal => 0,
      Self::ProviderRegister => 1,
      Self::ProviderUpdateService => 2,
      Self::ProviderUpdateRegistrar => 3,
      Self::ProviderUpdateRevoke => 4,
      Self::Coinbase => 5,
      Self::QuorumCommitment => 6,
      Self::MnhfSignal => 7,
      Self::AssetLock => 8,
      Self::AssetUnlock => 9,
      Self::Unknown(v) => v,
    }
  }
}

/// Masternode type in provider registration.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub enum MnType {
  /// 1000 DASH collateral, type 0.
  Regular,
  /// 4000 DASH collateral (Evo), type 1.
  Evo,
  /// Unrecognized type for forward compat.
  Unknown(u16),
}

impl MnType {
  /// Converts a raw 16-bit MN type code.
  #[inline]
  pub fn from_u16(val: u16) -> Self {
    match val {
      0 => Self::Regular,
      1 => Self::Evo,
      other => Self::Unknown(other),
    }
  }
}

/// Reference to a previous transaction output.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct OutPoint {
  pub hash: TxHash,
  pub index: u32,
}

impl Decode for OutPoint {
  #[inline]
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(Self {
      hash: TxHash::consensus_decode(cursor)?,
      index: u32::consensus_decode(cursor)?,
    })
  }
}

/// Transaction input spending a previous output.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TxIn {
  pub prevout: OutPoint,
  pub script_sig: Script,
  pub sequence: u32,
}

impl Decode for TxIn {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(Self {
      prevout: OutPoint::consensus_decode(cursor)?,
      script_sig: Script::consensus_decode(cursor)?,
      sequence: u32::consensus_decode(cursor)?,
    })
  }
}

/// Transaction output creating a spendable coin.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
#[cfg_attr(feature = "serde", derive(serde::Serialize))]
pub struct TxOut {
  /// Amount in duffs.
  pub value: i64,
  pub script_pubkey: Script,
}

impl Decode for TxOut {
  fn consensus_decode(cursor: &mut Cursor<'_>) -> Result<Self, DecodeError> {
    Ok(Self {
      value: i64::consensus_decode(cursor)?,
      script_pubkey: Script::consensus_decode(cursor)?,
    })
  }
}

/// Decodes a CompactSize-prefixed vector.
pub(crate) fn decode_vec<T: Decode>(
  cursor: &mut Cursor<'_>,
  max_count: usize,
) -> Result<Vec<T>, DecodeError> {
  let count = cursor.read_compact_size()?;
  if count > max_count as u64 {
    return Err(DecodeError::InvalidValue {
      field: "vector length",
      reason: "exceeds maximum allowed count",
    });
  }
  let count = count as usize;
  let mut items = Vec::with_capacity(count);
  for _ in 0..count {
    items.push(T::consensus_decode(cursor)?);
  }
  Ok(items)
}

/// Decodes a complete Dash transaction.
#[expect(clippy::type_complexity)]
pub(crate) fn decode_transaction(
  cursor: &mut Cursor<'_>,
) -> Result<(i16, TxType, Vec<TxIn>, Vec<TxOut>, u32, Vec<u8>), DecodeError> {
  let n_version = i32::consensus_decode(cursor)?;
  let version = (n_version & 0xFFFF) as i16;
  #[expect(clippy::cast_sign_loss)]
  let tx_type = TxType::from_u16(((n_version >> 16) & 0xFFFF) as u16);

  let inputs = decode_vec::<TxIn>(cursor, MAX_TX_INPUTS)?;
  let outputs = decode_vec::<TxOut>(cursor, MAX_TX_OUTPUTS)?;
  let lock_time = u32::consensus_decode(cursor)?;

  let extra_payload = if version >= 3 && tx_type != TxType::Normal {
    cursor.read_compact_bytes(MAX_EXTRA_PAYLOAD)?
  } else {
    Vec::new()
  };

  Ok((version, tx_type, inputs, outputs, lock_time, extra_payload))
}
