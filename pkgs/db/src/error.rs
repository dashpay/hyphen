//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The crate error type. The daemon never sees `sqlx::Error`; every storage
//! failure is surfaced as a [`HyDbError`], preserving the SQLSTATE code so the
//! caller can classify transient vs permanent without touching sqlx.

use core::fmt;
use std::error::Error as StdError;

/// The result alias returned by every fallible `hyphen-db` operation.
pub type HyDbResult<T> = Result<T, HyDbError>;

/// Every way a storage operation can fail, as seen from outside the crate.
#[derive(Debug)]
pub enum HyDbError {
  /// Connection URL or pool sizing was invalid.
  Config(String),
  /// A statement reached PostgreSQL and failed; `code` is the SQLSTATE when
  /// the driver exposed one.
  Pg {
    message: String,
    code: Option<String>,
  },
  /// Schema migration failed to apply.
  Migrate(String),
  /// The configured `[db.layout]` is invalid, or disagrees with the layout
  /// recorded in `init_params` when this database was created. Always fatal:
  /// the values are immutable for the life of the database, so retrying can
  /// only produce the same answer.
  Layout(String),
  /// A decoded row violated an invariant the type system could not encode
  /// (e.g. a hash column that did not hold 32 bytes).
  Decode(String),
  /// The caller-supplied [`crate::HyDbDataSrc`] rejected a block's bytes.
  Derive(String),
  /// A mock/in-memory backend method has no implementation yet. Never returned
  /// by the Postgres backend.
  MockGap(&'static str),
  /// A Postgres backend method whose body has not yet been ported. Temporary:
  /// removed as each domain is filled in.
  Unimplemented(&'static str),
}

impl HyDbError {
  /// Classify this error into a severity [`Outcome`].
  pub fn classify(&self) -> Outcome {
    match self {
      Self::Config(_) => Outcome::Fatal,
      Self::Migrate(_) => Outcome::Fatal,
      Self::Layout(_) => Outcome::Fatal,
      Self::Pg { code, message } => classify_pg(code.as_deref(), message),
      Self::Decode(_) | Self::Derive(_) | Self::MockGap(_) | Self::Unimplemented(_) => {
        Outcome::Error
      }
    }
  }

  /// Convenience: `true` when this error is transient.
  pub fn is_transient(&self) -> bool {
    self.classify() == Outcome::Warning
  }

  /// Build a [`HyDbError::Pg`] from an [`sqlx::Error`], keeping the SQLSTATE.
  pub(crate) fn pg(context: &str, error: sqlx::Error) -> Self {
    let code = error
      .as_database_error()
      .and_then(|db| db.code())
      .map(|c| c.to_string());
    Self::Pg {
      message: format!("{context}: {error}"),
      code,
    }
  }
}

impl fmt::Display for HyDbError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Config(msg) => write!(f, "db config error: {msg}"),
      Self::Pg { message, .. } => write!(f, "postgres error: {message}"),
      Self::Migrate(msg) => write!(f, "migration error: {msg}"),
      Self::Layout(msg) => write!(f, "storage layout error: {msg}"),
      Self::Decode(msg) => write!(f, "row decode error: {msg}"),
      Self::Derive(msg) => write!(f, "derive error: {msg}"),
      Self::MockGap(name) => write!(f, "mock backend has no impl for `{name}`"),
      Self::Unimplemented(name) => write!(f, "backend method `{name}` is not yet ported"),
    }
  }
}

impl StdError for HyDbError {}

impl From<hex::FromHexError> for HyDbError {
  fn from(e: hex::FromHexError) -> Self {
    Self::Decode(format!("hex decode: {e}"))
  }
}

/// Severity of a storage error, mirroring the daemon's supervisor contract:
/// transient failures self-resolve with backoff, permanent ones need
/// intervention, fatal ones stop the daemon.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
  Warning,
  Error,
  Fatal,
}

/// SQLSTATE-first, message-fallback classification for Postgres errors.
fn classify_pg(code: Option<&str>, message: &str) -> Outcome {
  if let Some(sqlstate) = code {
    return match sqlstate {
      _ if sqlstate.starts_with("08") => Outcome::Warning, // connection exception
      _ if sqlstate.starts_with("40") => Outcome::Warning, // transaction rollback
      _ if sqlstate.starts_with("53") => Outcome::Warning, // insufficient resources
      _ if sqlstate.starts_with("57") => Outcome::Warning, // operator intervention
      "55006" => Outcome::Warning,                         // database in recovery
      _ => Outcome::Error,
    };
  }
  let lower = message.to_ascii_lowercase();
  const TRANSIENT: &[&str] = &[
    "pool closed",
    "pool timed out",
    "failed to acquire connection",
    "connection refused",
    "connection reset",
    "broken pipe",
    "the database system is shutting down",
    "the database system is starting up",
    "could not connect to server",
  ];
  if TRANSIENT.iter().any(|p| lower.contains(p)) {
    Outcome::Warning
  } else {
    Outcome::Error
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use rstest::rstest;

  #[test]
  fn config_and_migrate_are_fatal() {
    assert_eq!(HyDbError::Config("x".into()).classify(), Outcome::Fatal);
    assert_eq!(HyDbError::Migrate("x".into()).classify(), Outcome::Fatal);
  }

  /// SQLSTATE-driven classification across every transient class and the
  /// permanent fallback. A `Some(code)` short-circuits before the message is
  /// ever inspected, so the message here is deliberately irrelevant.
  #[rstest]
  #[case("08006", Outcome::Warning)] // connection exception
  #[case("08001", Outcome::Warning)] // connection class, other member
  #[case("40001", Outcome::Warning)] // serialization failure
  #[case("40P01", Outcome::Warning)] // deadlock detected
  #[case("53300", Outcome::Warning)] // too many connections
  #[case("57014", Outcome::Warning)] // query canceled / operator intervention
  #[case("55006", Outcome::Warning)] // object in use — database in recovery
  #[case("23505", Outcome::Error)] // unique violation — permanent
  #[case("42P01", Outcome::Error)] // undefined table — permanent
  #[case("42601", Outcome::Error)] // syntax error — permanent
  fn classifies_pg_by_sqlstate(#[case] code: &str, #[case] want: Outcome) {
    let err = HyDbError::Pg {
      message: "irrelevant".into(),
      code: Some(code.into()),
    };
    assert_eq!(err.classify(), want);
    assert_eq!(err.is_transient(), want == Outcome::Warning);
  }

  /// With no SQLSTATE, classification falls back to substring-matching the
  /// message against the transient list. Covers every entry plus a miss.
  #[rstest]
  #[case("the connection pool closed", Outcome::Warning)]
  #[case("pool timed out waiting for a connection", Outcome::Warning)]
  #[case("failed to acquire connection from pool", Outcome::Warning)]
  #[case("connection refused (os error 61)", Outcome::Warning)]
  #[case("connection reset by peer", Outcome::Warning)]
  #[case("broken pipe", Outcome::Warning)]
  #[case("FATAL: the database system is shutting down", Outcome::Warning)]
  #[case("FATAL: the database system is starting up", Outcome::Warning)]
  #[case("could not connect to server", Outcome::Warning)]
  #[case(
    "ERROR: duplicate key value violates unique constraint",
    Outcome::Error
  )]
  fn classifies_pg_by_message_fallback(#[case] message: &str, #[case] want: Outcome) {
    let err = HyDbError::Pg {
      message: message.into(),
      code: None,
    };
    assert_eq!(err.classify(), want);
  }

  /// The non-Pg, non-config variants all classify as plain `Error`.
  #[rstest]
  #[case(HyDbError::Decode("bad row".into()))]
  #[case(HyDbError::Derive("bad block".into()))]
  #[case(HyDbError::MockGap("mock.method"))]
  #[case(HyDbError::Unimplemented("pg.method"))]
  fn other_variants_are_plain_error(#[case] err: HyDbError) {
    assert_eq!(err.classify(), Outcome::Error);
    assert!(!err.is_transient());
  }

  /// Every `Display` arm renders its prefix and payload.
  #[rstest]
  #[case(HyDbError::Config("c".into()), "db config error: c")]
  #[case(HyDbError::Pg { message: "m".into(), code: None }, "postgres error: m")]
  #[case(HyDbError::Migrate("mg".into()), "migration error: mg")]
  #[case(HyDbError::Decode("d".into()), "row decode error: d")]
  #[case(HyDbError::Derive("dr".into()), "derive error: dr")]
  #[case(HyDbError::MockGap("n"), "mock backend has no impl for `n`")]
  #[case(HyDbError::Unimplemented("n"), "backend method `n` is not yet ported")]
  fn display_renders_each_variant(#[case] err: HyDbError, #[case] want: &str) {
    assert_eq!(err.to_string(), want);
  }

  #[test]
  fn from_hex_error_becomes_decode() {
    // `zz` is not valid hex → `FromHexError`, which must convert to `Decode`.
    let converted = hex::decode("zz").map_err(HyDbError::from);
    assert!(
      matches!(&converted, Err(HyDbError::Decode(msg)) if msg.starts_with("hex decode:")),
      "expected Decode(hex decode: …), got {converted:?}"
    );
  }
}
