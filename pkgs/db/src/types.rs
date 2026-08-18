//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared value types crossing the storage boundary. All are `HyDb`-prefixed
//! and kept short (<= 18 chars). Newtypes wrap the primitive representations so
//! callers cannot confuse a block hash for a txid or a height for an amount.

use core::fmt;

use serde::{Deserialize, Serialize};
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgTypeInfo, PgValueRef};
use sqlx::{Decode, Postgres, Type};

use crate::HyDbError;

/// An address string in its canonical base58/bech32 form. `transparent` lets it
/// bind and decode as its inner `TEXT` directly.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize, sqlx::Type)]
#[sqlx(transparent)]
pub struct HyDbAddress(pub String);

/// Output script kind — mirrors the `address_type` Postgres enum.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "address_type", rename_all = "snake_case")]
pub enum HyDbAddrType {
  P2pkh,
  P2sh,
  P2pk,
  Unknown,
}

/// A signed satoshi-denominated amount (duffs, on Dash). Signed to represent
/// deltas as well as balances.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct HyDbAmount(pub i64);

/// An opaque keyset-pagination cursor. Encodes the composite sort key of the
/// last row returned; callers pass it back verbatim to fetch the next page and
/// never construct or inspect it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct HyDbCursor(pub String);

/// A 32-byte hash — block hash, txid, or protx_hash. Stored as `bytea` in
/// Postgres; the 32-byte length invariant is enforced here, not by the column.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HyDbHash(pub [u8; 32]);

impl HyDbHash {
  pub fn as_bytes(&self) -> &[u8; 32] {
    &self.0
  }

  /// Build from a byte slice, erroring if it is not exactly 32 bytes.
  pub fn from_slice(bytes: &[u8]) -> Result<Self, HyDbError> {
    let arr: [u8; 32] = bytes
      .try_into()
      .map_err(|_| HyDbError::Decode(format!("expected 32-byte hash, got {}", bytes.len())))?;
    Ok(Self(arr))
  }
}

impl fmt::Debug for HyDbHash {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    // Display txid/hash conventionally: reversed, hex. Matches explorer UIs.
    for b in self.0.iter().rev() {
      write!(f, "{b:02x}")?;
    }
    Ok(())
  }
}

// Decodes from a `bytea` column, enforcing the 32-byte invariant at the seam so
// `FromRow` can build a `HyDbHash` field directly.
impl Type<Postgres> for HyDbHash {
  fn type_info() -> PgTypeInfo {
    <&[u8] as Type<Postgres>>::type_info()
  }

  fn compatible(ty: &PgTypeInfo) -> bool {
    <&[u8] as Type<Postgres>>::compatible(ty)
  }
}

impl<'r> Decode<'r, Postgres> for HyDbHash {
  fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
    let bytes = <&[u8] as Decode<Postgres>>::decode(value)?;
    Ok(Self::from_slice(bytes)?)
  }
}

/// A block height. Canonical chain position; negative sentinels never cross the
/// boundary — absence is `Option<HyDbHeight>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HyDbHeight(pub u32);

impl HyDbHeight {
  /// Build from a stored `INTEGER`, rejecting negative sentinels: absence is
  /// `Option<HyDbHeight>`, never a negative height (an unchecked `as u32`
  /// would silently turn `-1` into ~4.29 billion).
  pub fn try_from_i32(height: i32) -> Result<Self, HyDbError> {
    u32::try_from(height)
      .map(Self)
      .map_err(|_| HyDbError::Decode(format!("negative height {height} crossed the boundary")))
  }
}

// Heights are stored as `INTEGER` (they always fit); decode reads the `i32` and
// widens, so `FromRow` can build a `HyDbHeight` field directly. Negative values
// are rejected — see [`HyDbHeight::try_from_i32`].
impl Type<Postgres> for HyDbHeight {
  fn type_info() -> PgTypeInfo {
    <i32 as Type<Postgres>>::type_info()
  }

  fn compatible(ty: &PgTypeInfo) -> bool {
    <i32 as Type<Postgres>>::compatible(ty)
  }
}

impl<'r> Decode<'r, Postgres> for HyDbHeight {
  fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
    Ok(Self::try_from_i32(<i32 as Decode<Postgres>>::decode(
      value,
    )?)?)
  }
}

/// A transaction outpoint: which output of which transaction. Packs to the
/// 36-byte `outpoint` key (`txid || int4send(vout)`) used by `spends`/`utxo_set`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct HyDbOutpoint {
  pub txid: HyDbHash,
  pub vout: u32,
}

impl HyDbOutpoint {
  /// Unpack from the 36-byte key.
  pub fn from_packed(bytes: &[u8]) -> Result<Self, HyDbError> {
    if bytes.len() != 36 {
      return Err(HyDbError::Decode(format!(
        "expected 36-byte outpoint, got {}",
        bytes.len()
      )));
    }
    let txid = HyDbHash::from_slice(&bytes[..32])?;
    let mut v = [0u8; 4];
    v.copy_from_slice(&bytes[32..]);
    Ok(Self {
      txid,
      vout: u32::from_be_bytes(v),
    })
  }

  /// Pack into the 36-byte big-endian key: txid(32) || vout(4 BE).
  pub fn packed(&self) -> [u8; 36] {
    let mut out = [0u8; 36];
    out[..32].copy_from_slice(&self.txid.0);
    out[32..].copy_from_slice(&self.vout.to_be_bytes());
    out
  }
}

/// One page of a keyset-paginated result.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HyDbPage<T> {
  pub items: Vec<T>,
  /// Cursor for the next page, or `None` when this is the last page.
  pub next_cursor: Option<HyDbCursor>,
}

impl<T> HyDbPage<T> {
  /// A terminal page with no continuation.
  pub fn last(items: Vec<T>) -> Self {
    Self {
      items,
      next_cursor: None,
    }
  }

  pub fn new(items: Vec<T>, next_cursor: Option<HyDbCursor>) -> Self {
    Self { items, next_cursor }
  }
}

/// Transaction kind — mirrors the `tx_type` Postgres enum. The `sqlx::Type`
/// mapping lets `query!` decode the enum column directly.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "tx_type", rename_all = "snake_case")]
pub enum HyDbTxType {
  Normal,
  ProviderRegister,
  ProviderUpdateService,
  ProviderUpdateRegistrar,
  ProviderUpdateRevoke,
  Coinbase,
  QuorumCommitment,
  AssetLock,
  AssetUnlock,
  Mnhf,
}

/// Validator kind — mirrors the `validator_type` Postgres enum. Stored
/// already-normalized, so filtering is a plain equality.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, sqlx::Type)]
#[serde(rename_all = "snake_case")]
#[sqlx(type_name = "validator_type", rename_all = "snake_case")]
pub enum HyDbVlType {
  Regular,
  Evonode,
  Unknown,
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn outpoint_roundtrips_through_packed_key() {
    let op = HyDbOutpoint {
      txid: HyDbHash([7u8; 32]),
      vout: 0x0102_0304,
    };
    let packed = op.packed();
    assert_eq!(&packed[32..], &[0x01, 0x02, 0x03, 0x04]);
    assert_eq!(HyDbOutpoint::from_packed(&packed).ok(), Some(op));
  }

  #[test]
  fn hash_from_slice_rejects_wrong_length() {
    assert!(HyDbHash::from_slice(&[0u8; 31]).is_err());
    assert!(HyDbHash::from_slice(&[0u8; 32]).is_ok());
  }

  #[test]
  fn hash_debug_is_reversed_hex() {
    let mut bytes = [0u8; 32];
    bytes[0] = 0xab;
    let s = format!("{:?}", HyDbHash(bytes));
    assert!(s.ends_with("ab"));
    assert_eq!(s.len(), 64);
  }
}
