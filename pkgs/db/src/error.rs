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
  /// Chain data on disk violated a consistency invariant the schema cannot
  /// express — currently, undo coverage that does not reach the canonical tip.
  /// Always fatal: the data is already written, so retrying reads it back
  /// unchanged; an operator has to rewind or re-derive.
  Consistency(String),
  /// A decoded row violated an invariant the type system could not encode
  /// (e.g. a hash column that did not hold 32 bytes).
  Decode(String),
  /// The caller-supplied [`crate::HyDbDataSrc`] rejected a block's bytes.
  Derive(String),
  /// A mock/in-memory backend method has no implementation yet. Never returned
  /// by the Postgres backend.
  MockGap(&'static str),
}

impl HyDbError {
  /// Classify this error into a severity [`Outcome`].
  pub fn classify(&self) -> Outcome {
    match self {
      Self::Config(_) => Outcome::Fatal,
      Self::Migrate(_) => Outcome::Fatal,
      Self::Layout(_) => Outcome::Fatal,
      Self::Consistency(_) => Outcome::Fatal,
      Self::Pg { code, message } => classify_pg(code.as_deref(), message),
      Self::Decode(_) | Self::Derive(_) | Self::MockGap(_) => Outcome::Error,
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
      Self::Consistency(msg) => write!(f, "storage consistency error: {msg}"),
      Self::Decode(msg) => write!(f, "row decode error: {msg}"),
      Self::Derive(msg) => write!(f, "derive error: {msg}"),
      Self::MockGap(name) => write!(f, "mock backend has no impl for `{name}`"),
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
      _ if sqlstate.starts_with("58") => classify_class_58(sqlstate),
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
    // Below: class-58 (`system_error`) wording, for the rare path where the
    // driver surfaces an OS-level I/O failure as a plain `sqlx::Error` (e.g.
    // a socket write failing outright) rather than as a `Database` error with
    // a SQLSTATE attached. When a SQLSTATE *is* present, `classify_class_58`
    // above runs instead and this list is never consulted — see the doc
    // comment on that function for why "Device not configured" reaching this
    // fallback was never actually the bug.
    "device not configured",
    "input/output error",
    "no such device",
  ];
  if TRANSIENT.iter().any(|p| lower.contains(p)) {
    Outcome::Warning
  } else {
    Outcome::Error
  }
}

/// Classify a class-58 (`system_error`) SQLSTATE: an error PostgreSQL raises
/// about the world outside itself — the filesystem, the OS, the device under
/// its data directory — rather than about the statement it was asked to run.
///
/// This is the class the mainnet incident actually hit: an external SSD
/// dropped off the bus mid-sync, a read failed with `ENXIO` ("Device not
/// configured"), and PostgreSQL reported it as SQLSTATE `58030` (`io_error`).
/// Before this function existed, class 58 fell through the blanket `_ =>
/// Outcome::Error` arm in [`classify_pg`], which the daemon turns into
/// [`FailureAction::Pause`](crate) — a park that needs a human — for a fault
/// that resolved itself when the device remounted a few minutes later. A
/// vanished-then-returned device is the textbook definition of transient.
///
/// The message-substring `TRANSIENT` fallback in [`classify_pg`] was never in
/// a position to catch this: the driver *did* attach a SQLSTATE (`58030`), so
/// `classify_pg` returns from the `Some(sqlstate)` arm before the substring
/// list is ever consulted. That fallback only runs when the driver error
/// carries no SQLSTATE at all (a connection that never reached the server),
/// which is a different failure shape from a statement that reached Postgres
/// and got an error back.
///
/// Class 58 is not treated as uniformly transient, though — two of its named
/// members describe a genuine misconfiguration rather than a device blip:
///
/// - `58030` `io_error` — a read/write against the data directory failed at
///   the OS level. External to Postgres by definition, and exactly the shape
///   of a device that dropped off the bus and can come back. **Warning.**
/// - `58P01` `undefined_file` — Postgres looked for a file it expects to
///   exist (e.g. a tablespace or WAL segment) and it was not there. Retrying
///   reads the same absent file; this needs an operator to explain why the
///   file is gone. **Error.**
/// - `58P02` `duplicate_file` — the reverse shape: Postgres tried to create a
///   file that already exists. Also not something a retry resolves on its
///   own. **Error.**
/// - Any other/future class-58 code (including the bare `58000` root) is
///   classified `Warning`, on the same reasoning as `58030`: this whole class
///   is specifically the one PostgreSQL reserves for external system faults,
///   as opposed to the classes covering query, schema, or permission errors.
///   Erring toward "retry" here matches the ladder's own backstop
///   ([`MAX_BACKOFF_SECS`](crate) caps it, and the daemon's bounded park
///   catches a false "Warning" that never actually resolves), whereas erring
///   toward "needs a human" is exactly the defect this function exists to fix.
fn classify_class_58(sqlstate: &str) -> Outcome {
  match sqlstate {
    "58P01" | "58P02" => Outcome::Error,
    _ => Outcome::Warning,
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
  #[case("42601", Outcome::Error)]
  // syntax error — permanent
  // Class 58 (`system_error`): the mainnet incident's actual SQLSTATE.
  // `58030` is `io_error` — a vanished-then-returned block device is
  // transient by definition. Before this fix, every class-58 code fell
  // through the blanket `_ => Outcome::Error` arm, which is what turned a
  // device blip into a 26-hour operator-required park.
  #[case("58030", Outcome::Warning)] // io_error — external I/O fault
  #[case("58000", Outcome::Warning)] // bare system_error root — unclassified, err toward retry
  #[case("58003", Outcome::Warning)]
  // an unnamed/future class-58 member
  // `undefined_file`/`duplicate_file` are real misconfiguration, not a device
  // blip: retrying re-reads the same absent (or already-present) file.
  #[case("58P01", Outcome::Error)] // undefined_file
  #[case("58P02", Outcome::Error)] // duplicate_file
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
  // Defense-in-depth for the rare case where a class-58 fault surfaces as a
  // driver-level I/O failure with no SQLSTATE at all (e.g. a socket write
  // failing outright rather than a statement the server itself rejected).
  #[case(
    "could not read blocks 369426..369426 in file \"base/16401/369426\": Device not configured",
    Outcome::Warning
  )]
  #[case(
    "Io(Custom { kind: Other, error: \"Input/output error\" })",
    Outcome::Warning
  )]
  fn classifies_pg_by_message_fallback(#[case] message: &str, #[case] want: Outcome) {
    let err = HyDbError::Pg {
      message: message.into(),
      code: None,
    };
    assert_eq!(err.classify(), want);
  }

  /// The exact incident: a `58030` (`io_error`) SQLSTATE from PostgreSQL for
  /// "could not read blocks ... Device not configured" must classify as
  /// transient, not permanent — this is what turns a 26-hour operator-required
  /// park into a backoff-ladder retry that resolves within minutes once the
  /// device remounts.
  #[test]
  fn the_incident_device_error_is_transient() {
    let err = HyDbError::Pg {
      message: "could not read blocks 369426..369426 in file \"base/16401/369426\": \
                Device not configured"
        .to_owned(),
      code: Some("58030".to_owned()),
    };
    assert_eq!(err.classify(), Outcome::Warning);
    assert!(err.is_transient());
  }

  /// A SQLSTATE, when present, always wins over the message — the substring
  /// fallback is consulted only when the driver attached no SQLSTATE at all.
  /// A message that happens to contain transient-sounding wording must not
  /// downgrade a genuinely permanent code.
  #[test]
  fn a_present_sqlstate_short_circuits_the_message_fallback() {
    let err = HyDbError::Pg {
      message: "connection refused, but this is actually a syntax error".to_owned(),
      code: Some("42601".to_owned()),
    };
    assert_eq!(err.classify(), Outcome::Error);
  }

  /// The non-Pg, non-config variants all classify as plain `Error`.
  #[rstest]
  #[case(HyDbError::Decode("bad row".into()))]
  #[case(HyDbError::Derive("bad block".into()))]
  #[case(HyDbError::MockGap("mock.method"))]
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
