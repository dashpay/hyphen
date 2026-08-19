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

impl HyDbAmount {
  /// Convert an unsigned satoshi count from a decoder into the signed width
  /// amounts are stored at.
  ///
  /// # Errors
  ///
  /// Returns `Decode` above `i64::MAX`. No real amount reaches it — the whole
  /// money supply is some three thousand times smaller — but a decoder reading
  /// a malformed transaction can produce one, and an `as i64` cast would hand
  /// on a negative amount that then flows into every accumulation that sums it.
  pub fn from_sat(sats: u64) -> Result<Self, HyDbError> {
    i64::try_from(sats)
      .map(Self)
      .map_err(|_| HyDbError::Decode(format!("amount {sats} duffs exceeds the storable range")))
  }

  /// Sum amounts, refusing an overflow rather than wrapping into a negative
  /// total.
  ///
  /// The release profile enables neither overflow checks nor panics on
  /// arithmetic, so a plain `sum()` wraps silently.
  ///
  /// # Errors
  ///
  /// Returns `Decode` when the running total would overflow.
  pub fn try_sum(amounts: impl IntoIterator<Item = Self>) -> Result<Self, HyDbError> {
    let mut total: i64 = 0;
    for amount in amounts {
      total = total.checked_add(amount.0).ok_or_else(|| {
        HyDbError::Decode("summing amounts overflowed the storable range".to_owned())
      })?;
    }
    Ok(Self(total))
  }
}

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
///
/// The inner field is private so every narrowing runs through
/// [`Self::as_i32`]. A bare `as i32` on a height that arrived over the wire
/// wraps large values into negative sentinels, which the depth bounds below
/// then read as a shallow, admissible span.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct HyDbHeight(u32);

impl HyDbHeight {
  /// Wrap a height that is already in range, such as one counted up from
  /// zero or read back through [`Self::try_from_i32`].
  pub const fn new(height: u32) -> Self {
    Self(height)
  }

  /// Read the height for arithmetic and formatting.
  pub const fn get(self) -> u32 {
    self.0
  }

  /// Build from a stored `INTEGER`, rejecting negative sentinels: absence is
  /// `Option<HyDbHeight>`, never a negative height (an unchecked `as u32`
  /// would silently turn `-1` into ~4.29 billion).
  ///
  /// # Errors
  ///
  /// Returns `Decode` when `height` is negative.
  pub fn try_from_i32(height: i32) -> Result<Self, HyDbError> {
    u32::try_from(height)
      .map(Self)
      .map_err(|_| HyDbError::Decode(format!("negative height {height} crossed the boundary")))
  }

  /// Narrow to the `INTEGER` width the projection stores heights at.
  ///
  /// # Errors
  ///
  /// Returns `Decode` when the height exceeds `i32::MAX`, which no real
  /// chain reaches but a wire-supplied target can claim.
  pub fn as_i32(self) -> Result<i32, HyDbError> {
    i32::try_from(self.0).map_err(|_| {
      HyDbError::Decode(format!(
        "height {} exceeds the stored INTEGER range",
        self.0
      ))
    })
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

  #[test]
  fn amount_conversion_refuses_what_a_cast_would_negate() {
    assert_eq!(HyDbAmount::from_sat(0).ok(), Some(HyDbAmount(0)));
    assert_eq!(
      HyDbAmount::from_sat(i64::MAX as u64).ok(),
      Some(HyDbAmount(i64::MAX))
    );
    // The values `as i64` turns into a negative amount, which then sums into
    // the supply and every balance that touches the output.
    for wide in [u64::MAX, 1 << 63, i64::MAX as u64 + 1] {
      assert!(
        HyDbAmount::from_sat(wide).is_err(),
        "amount {wide} must not convert"
      );
    }
  }

  #[test]
  fn summing_amounts_refuses_an_overflow() {
    assert_eq!(
      HyDbAmount::try_sum([HyDbAmount(2), HyDbAmount(3)]).ok(),
      Some(HyDbAmount(5))
    );
    assert!(HyDbAmount::try_sum([]).is_ok());
    assert!(
      HyDbAmount::try_sum([HyDbAmount(i64::MAX), HyDbAmount(1)]).is_err(),
      "a total past the range must refuse rather than wrap negative"
    );
  }

  #[test]
  fn height_narrowing_refuses_what_a_cast_would_wrap() {
    // The three values a bare `as i32` turns into a negative floor, which
    // every depth bound downstream then reads as an admissible span.
    for wide in [u32::MAX, 0x8000_0000, i32::MAX as u32 + 1] {
      assert!(
        HyDbHeight::new(wide).as_i32().is_err(),
        "height {wide} must not narrow"
      );
    }
    assert_eq!(HyDbHeight::new(0).as_i32().ok(), Some(0));
    assert_eq!(
      HyDbHeight::new(i32::MAX as u32).as_i32().ok(),
      Some(i32::MAX)
    );
  }

  #[test]
  fn height_round_trips_through_the_stored_width() {
    for h in [0_i32, 1, 860_000, i32::MAX] {
      assert_eq!(
        HyDbHeight::try_from_i32(h)
          .and_then(HyDbHeight::as_i32)
          .ok(),
        Some(h)
      );
    }
    assert!(HyDbHeight::try_from_i32(-1).is_err());
  }
}
