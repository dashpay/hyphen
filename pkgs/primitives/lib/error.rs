//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Layered error types for decoding.

use core::fmt;

/// Byte-cursor read errors (leaf level).
#[derive(Clone, PartialEq, Eq)]
pub enum ReadError {
  /// Buffer too short for the read.
  UnexpectedEof { needed: usize, available: usize },

  /// CompactSize uses non-minimal encoding.
  NonCanonicalCompactSize,

  /// Decoded CompactSize exceeds platform limit.
  CompactSizeOverflow(u64),
}

impl fmt::Debug for ReadError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::UnexpectedEof { needed, available } => f
        .debug_struct("UnexpectedEof")
        .field("needed", needed)
        .field("available", available)
        .finish(),
      Self::NonCanonicalCompactSize => write!(f, "NonCanonicalCompactSize"),
      Self::CompactSizeOverflow(val) => {
        write!(f, "CompactSizeOverflow({val})")
      }
    }
  }
}

impl fmt::Display for ReadError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::UnexpectedEof { needed, available } => {
        write!(
          f,
          "unexpected end of buffer: needed {needed} bytes but only {available} available"
        )
      }
      Self::NonCanonicalCompactSize => {
        write!(f, "non-canonical CompactSize encoding")
      }
      Self::CompactSizeOverflow(val) => {
        write!(f, "CompactSize value {val} exceeds maximum allowed size")
      }
    }
  }
}

#[cfg(feature = "std")]
impl std::error::Error for ReadError {}

// ---------------------------------------------------------------------------
// Layer 1: DecodeError
// ---------------------------------------------------------------------------

/// Consensus structure decode errors.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum DecodeError {
  /// Byte-level read failed.
  Read(ReadError),

  /// Field value is structurally invalid.
  InvalidValue {
    field: &'static str,
    reason: &'static str,
  },
}

impl From<ReadError> for DecodeError {
  #[inline]
  fn from(e: ReadError) -> Self {
    Self::Read(e)
  }
}

impl fmt::Debug for DecodeError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Read(e) => write!(f, "Read({e:?})"),
      Self::InvalidValue { field, reason } => f
        .debug_struct("InvalidValue")
        .field("field", field)
        .field("reason", reason)
        .finish(),
    }
  }
}

impl fmt::Display for DecodeError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Read(e) => write!(f, "decode read error: {e}"),
      Self::InvalidValue { field, reason } => {
        write!(f, "invalid value for field '{field}': {reason}")
      }
    }
  }
}

#[cfg(feature = "std")]
impl std::error::Error for DecodeError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Read(e) => Some(e),
      Self::InvalidValue { .. } => None,
    }
  }
}

// ---------------------------------------------------------------------------
// Layer 2: TxError
// ---------------------------------------------------------------------------

/// Transaction-level decode errors.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum TxError {
  /// Consensus decode of a tx field failed.
  Decode(DecodeError),
}

impl From<DecodeError> for TxError {
  #[inline]
  fn from(e: DecodeError) -> Self {
    Self::Decode(e)
  }
}

impl From<ReadError> for TxError {
  #[inline]
  fn from(e: ReadError) -> Self {
    Self::Decode(DecodeError::Read(e))
  }
}

impl fmt::Debug for TxError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Decode(e) => write!(f, "Decode({e:?})"),
    }
  }
}

impl fmt::Display for TxError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Decode(e) => write!(f, "transaction decode error: {e}"),
    }
  }
}

#[cfg(feature = "std")]
impl std::error::Error for TxError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Decode(e) => Some(e),
    }
  }
}

// ---------------------------------------------------------------------------
// Layer 2: BlockError
// ---------------------------------------------------------------------------

/// Block-level decode errors.
#[derive(Clone, PartialEq, Eq)]
#[non_exhaustive]
pub enum BlockError {
  /// Block-level field decode failed.
  Decode(DecodeError),

  /// A transaction in the block failed.
  Tx {
    /// Zero-based tx index.
    index: usize,
    source: TxError,
  },
}

impl From<DecodeError> for BlockError {
  #[inline]
  fn from(e: DecodeError) -> Self {
    Self::Decode(e)
  }
}

impl From<ReadError> for BlockError {
  #[inline]
  fn from(e: ReadError) -> Self {
    Self::Decode(DecodeError::Read(e))
  }
}

impl fmt::Debug for BlockError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Decode(e) => write!(f, "Decode({e:?})"),
      Self::Tx { index, source } => f
        .debug_struct("Tx")
        .field("index", index)
        .field("source", source)
        .finish(),
    }
  }
}

impl fmt::Display for BlockError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Decode(e) => write!(f, "block decode error: {e}"),
      Self::Tx { index, source } => {
        write!(f, "block transaction {index} error: {source}")
      }
    }
  }
}

#[cfg(feature = "std")]
impl std::error::Error for BlockError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Decode(e) => Some(e),
      Self::Tx { source, .. } => Some(source),
    }
  }
}

// ---------------------------------------------------------------------------
// Layer 3: PayloadError
// ---------------------------------------------------------------------------

/// Special-transaction payload decode errors.
#[derive(Clone, PartialEq, Eq)]
pub enum PayloadError {
  /// Payload field decode failed.
  Decode(DecodeError),

  /// Unknown special transaction type code.
  UnknownTxType(u16),

  /// Payload version not supported.
  UnsupportedVersion { payload: &'static str, version: u16 },
}

impl From<DecodeError> for PayloadError {
  #[inline]
  fn from(e: DecodeError) -> Self {
    Self::Decode(e)
  }
}

impl From<ReadError> for PayloadError {
  #[inline]
  fn from(e: ReadError) -> Self {
    Self::Decode(DecodeError::Read(e))
  }
}

impl fmt::Debug for PayloadError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Decode(e) => write!(f, "Decode({e:?})"),
      Self::UnknownTxType(t) => write!(f, "UnknownTxType({t})"),
      Self::UnsupportedVersion { payload, version } => f
        .debug_struct("UnsupportedVersion")
        .field("payload", payload)
        .field("version", version)
        .finish(),
    }
  }
}

impl fmt::Display for PayloadError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Decode(e) => write!(f, "payload decode error: {e}"),
      Self::UnknownTxType(t) => {
        write!(f, "unknown special transaction type: {t}")
      }
      Self::UnsupportedVersion { payload, version } => {
        write!(
          f,
          "unsupported version {version} for payload type '{payload}'"
        )
      }
    }
  }
}

#[cfg(feature = "std")]
impl std::error::Error for PayloadError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Decode(e) => Some(e),
      Self::UnknownTxType(_) => None,
      Self::UnsupportedVersion { .. } => None,
    }
  }
}
