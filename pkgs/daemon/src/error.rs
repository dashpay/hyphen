//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Central error type with transient/permanent classification.
//!
//! Storage failures cross this seam as the typed [`HyDbError`] they were born
//! as — never as a formatted string. `DaemonError::Storage` holds the source
//! error itself, so `std::error::Error::source()` walks the causal chain back
//! to the SQLSTATE and [`classify`](DaemonError::classify) can ask the storage
//! crate for its verdict instead of re-deriving one from a message.

use core::fmt;

use hyphen_db::{HyDbError, Outcome as HyDbOutcome};

/// Classification of the underlying REST transport failure.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestErrorKind {
  Connect,
  Timeout,
  Http(u16),
  Other,
}

/// Top-level daemon error covering setup, storage, and indexing failures.
#[derive(Debug)]
pub enum DaemonError {
  NoPlatformDataDir,
  AlreadyRunning,
  InsufficientDisk {
    available: u64,
    required: u64,
  },
  Io {
    context: &'static str,
    source: std::io::Error,
  },
  /// A storage-engine failure. `context` names the operation that failed;
  /// `source` is the storage crate's own typed error, kept intact so its
  /// SQLSTATE and classification survive the hop.
  Storage {
    context: &'static str,
    source: HyDbError,
  },
  RestSource {
    message: String,
    kind: RestErrorKind,
  },
  /// An operator-facing configuration problem: `[section] key` requires
  /// something the file does not provide.
  ///
  /// The payload stays a sentence deliberately. Every surviving constructor is
  /// a startup validation message whose only consumer is the operator reading
  /// stderr, which is the one place prose is the right representation. What
  /// used to also travel through here — supervisor and shutdown failures — has
  /// its own variants below.
  Config(String),
  /// A supervised subsystem could not be brought back up.
  ///
  /// Previously smuggled through `Config(String)`, which it never was: the
  /// configuration is fine, the worker is not. Keeping the subsystem and the
  /// disposition typed means a supervisor decision can be read without parsing
  /// the sentence it was rendered into.
  Supervisor {
    subsystem: &'static str,
    failure: SupervisorFailure,
  },
  /// A subsystem did not finish shutting down inside its grace period.
  ShutdownTimeout {
    subsystem: &'static str,
    secs: u64,
  },
  /// A storage startup step did not return inside the readiness window.
  ///
  /// Distinct from `Storage`, which carries a failure the server reported. This
  /// is the absence of any answer — a connection accepted and then left hanging
  /// mid-handshake, or a lock wait with nothing behind it. The retry ladder
  /// cannot classify what it never received, so the window has to bound the
  /// individual attempt and not only the gaps between them.
  StorageTimeout {
    context: &'static str,
    secs: u64,
  },
  Index(String),
}

/// Why a supervised subsystem could not be brought back.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SupervisorFailure {
  /// Recovery was declined: the run state does not permit a restart.
  Blocked(&'static str),
  /// The retry budget is spent.
  Exhausted,
  /// This worker cannot be restarted at all.
  Unavailable(&'static str),
}

impl fmt::Display for SupervisorFailure {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Blocked(reason) => write!(f, "recovery blocked: {reason}"),
      Self::Exhausted => write!(f, "recovery exhausted its retry budget"),
      Self::Unavailable(reason) => write!(f, "recovery unavailable: {reason}"),
    }
  }
}

impl fmt::Display for DaemonError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::NoPlatformDataDir => write!(f, "cannot determine platform data directory"),
      Self::AlreadyRunning => write!(f, "another hyphend instance holds the lockfile"),
      Self::InsufficientDisk {
        available,
        required,
      } => {
        write!(
          f,
          "insufficient disk: {available} bytes free, {required} required"
        )
      }
      Self::Io { context, source } => write!(f, "I/O error ({context}): {source}"),
      Self::Storage { context, source } => write!(f, "storage error ({context}): {source}"),
      Self::RestSource { message, .. } => write!(f, "data source error: {message}"),
      Self::Config(msg) => write!(f, "config error: {msg}"),
      Self::Supervisor { subsystem, failure } => {
        write!(f, "{subsystem} subsystem: {failure}")
      }
      Self::ShutdownTimeout { subsystem, secs } => {
        write!(f, "{subsystem} did not shut down within {secs}s")
      }
      Self::StorageTimeout { context, secs } => {
        write!(f, "storage engine did not answer {context} within {secs}s")
      }
      Self::Index(msg) => write!(f, "index error: {msg}"),
    }
  }
}

impl std::error::Error for DaemonError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Io { source, .. } => Some(source),
      Self::Storage { source, .. } => Some(source),
      _ => None,
    }
  }
}

/// Severity classification for daemon errors.
///
/// `classify()` maps every [`DaemonError`] variant to one of these outcomes
/// so that supervisors can decide whether to retry, pause, or shut down.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
  /// Transient — expected to self-resolve with backoff (network blip,
  /// pool exhaustion, PG restart, HTTP 503/429).
  Warning,
  /// Requires admin intervention — the affected subsystem pauses but the
  /// daemon (including the REST API) stays up.
  Error,
  /// Daemon must shut down (missing data dir, conflicting instance, bad
  /// config, insufficient disk).
  Fatal,
}

impl DaemonError {
  /// Classify this error into a severity [`Outcome`].
  pub fn classify(&self) -> Outcome {
    match self {
      Self::NoPlatformDataDir
      | Self::AlreadyRunning
      | Self::InsufficientDisk { .. }
      | Self::Config(_)
      // A supervisor that has given up is the end of the road for the process:
      // graded exactly as the `Config(_)` these used to be built from, so the
      // typing changes what can be read off the error, not what happens.
      | Self::Supervisor { .. }
      | Self::ShutdownTimeout { .. }
      // Startup already spent its whole readiness window waiting. Retrying the
      // same unanswered call is what the window was for.
      | Self::StorageTimeout { .. } => Outcome::Fatal,

      // Delegate rather than re-derive: the storage crate owns the
      // SQLSTATE policy and is the only place it is written down.
      Self::Storage { source, .. } => Outcome::from(source.classify()),

      Self::RestSource { kind, .. } => match kind {
        RestErrorKind::Connect | RestErrorKind::Timeout => Outcome::Warning,
        RestErrorKind::Http(503 | 429) => Outcome::Warning,
        _ => Outcome::Error,
      },

      Self::Io { .. } | Self::Index(_) => Outcome::Error,
    }
  }

  /// Convenience: returns `true` when this error is transient.
  pub fn is_transient(&self) -> bool {
    self.classify() == Outcome::Warning
  }
}

impl DaemonError {
  /// Wrap a storage-engine failure, tagging it with the operation that failed.
  ///
  /// This is the only sanctioned way for a `HyDbError` to become a
  /// `DaemonError`: it moves the error in whole, so nothing downstream has to
  /// parse a message to learn what happened.
  pub fn storage(context: &'static str, source: HyDbError) -> Self {
    Self::Storage { context, source }
  }

  /// The storage error underneath, when this failure came from `hyphen-db`.
  ///
  /// Callers that need the SQLSTATE (telemetry, status mapping) match on this
  /// rather than reading [`Display`](fmt::Display) output.
  pub fn storage_source(&self) -> Option<&HyDbError> {
    match self {
      Self::Storage { source, .. } => Some(source),
      _ => None,
    }
  }
}

impl From<HyDbError> for DaemonError {
  /// `?` shorthand for call sites with no more specific context to add.
  fn from(source: HyDbError) -> Self {
    Self::Storage {
      context: "storage engine",
      source,
    }
  }
}

impl From<HyDbOutcome> for Outcome {
  /// The two severity ladders are the same ladder; this is the only place
  /// they are allowed to disagree, and they do not.
  fn from(outcome: HyDbOutcome) -> Self {
    match outcome {
      HyDbOutcome::Warning => Self::Warning,
      HyDbOutcome::Error => Self::Error,
      HyDbOutcome::Fatal => Self::Fatal,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn classify_fatal_variants() {
    assert_eq!(DaemonError::NoPlatformDataDir.classify(), Outcome::Fatal);
    assert_eq!(DaemonError::AlreadyRunning.classify(), Outcome::Fatal);
    assert_eq!(
      DaemonError::InsufficientDisk {
        available: 0,
        required: 1
      }
      .classify(),
      Outcome::Fatal
    );
    assert_eq!(
      DaemonError::Config("bad".to_owned()).classify(),
      Outcome::Fatal
    );
  }

  fn pg(code: Option<&str>, message: &str) -> HyDbError {
    HyDbError::Pg {
      message: message.to_owned(),
      code: code.map(str::to_owned),
    }
  }

  /// A transient SQLSTATE must survive the whole `sqlx -> HyDbError ->
  /// DaemonError` chain with its classification intact. This is the
  /// regression guard for E2.2: flatten the storage error into a string
  /// anywhere along the way and the code is gone, so these fail.
  #[test]
  fn storage_sqlstate_survives_conversion() {
    let cases: &[(&str, Outcome)] = &[
      ("08006", Outcome::Warning), // connection failure
      ("40P01", Outcome::Warning), // deadlock detected
      ("40001", Outcome::Warning), // serialization failure
      ("53300", Outcome::Warning), // too many connections
      ("57P01", Outcome::Warning), // admin shutdown
      ("57P03", Outcome::Warning), // database system is starting up
      ("55006", Outcome::Warning), // database in recovery
      ("23505", Outcome::Error),   // unique violation — permanent
      ("42P01", Outcome::Error),   // undefined table — permanent
    ];
    for (code, want) in cases {
      let source = pg(Some(code), "irrelevant");
      let graded_by_storage = Outcome::from(source.classify());
      let err = DaemonError::storage("ingest", source);

      assert_eq!(err.classify(), *want, "sqlstate {code} graded wrongly");
      assert_eq!(err.is_transient(), *want == Outcome::Warning);
      // The daemon holds no opinion of its own: it reports whatever the
      // storage crate decided.
      assert_eq!(err.classify(), graded_by_storage);

      // The SQLSTATE is still there to be read, not rendered away.
      let kept = match err.storage_source() {
        Some(HyDbError::Pg {
          code: Some(kept), ..
        }) => Some(kept.as_str()),
        _ => None,
      };
      assert_eq!(
        kept,
        Some(*code),
        "storage source lost its SQLSTATE: {err:?}"
      );
    }
  }

  /// `source()` walks to the storage error, so the causal chain stays intact
  /// for anything that reports errors by traversing it.
  #[test]
  fn storage_error_exposes_its_source() {
    use std::error::Error as _;
    let err = DaemonError::storage("read canonical tip", pg(Some("40P01"), "deadlock detected"));
    let carried = err
      .source()
      .and_then(|source| source.downcast_ref::<HyDbError>());
    assert!(
      carried.is_some(),
      "storage error must expose its hyphen-db source: {err:?}"
    );
    assert!(
      err
        .to_string()
        .starts_with("storage error (read canonical tip)"),
      "unexpected rendering: {err}"
    );
  }

  /// Pool-level failures never reach the engine and so carry no SQLSTATE;
  /// they must still classify as transient after conversion.
  #[test]
  fn storage_message_fallback_survives_conversion() {
    let cases: &[(&str, Outcome)] = &[
      (
        "pool timed out while waiting for a connection",
        Outcome::Warning,
      ),
      ("the database system is starting up", Outcome::Warning),
      ("constraint violation on insert", Outcome::Error),
    ];
    for (message, want) in cases {
      let err = DaemonError::storage("connect", pg(None, message));
      assert_eq!(err.classify(), *want, "message {message:?} graded wrongly");
    }
  }

  /// Non-Postgres storage variants keep their own grades across the seam: a
  /// bad config or a failed migration is still fatal, a bad row is not.
  #[test]
  fn storage_non_pg_variants_keep_their_grade() {
    let cases: Vec<(HyDbError, Outcome)> = vec![
      (HyDbError::Config("bad url".to_owned()), Outcome::Fatal),
      (HyDbError::Migrate("schema".to_owned()), Outcome::Fatal),
      (HyDbError::Decode("short hash".to_owned()), Outcome::Error),
      (HyDbError::Derive("bad block".to_owned()), Outcome::Error),
    ];
    for (source, want) in cases {
      let graded_by_storage = Outcome::from(source.classify());
      let err = DaemonError::storage("ctx", source);
      assert_eq!(err.classify(), want);
      assert_eq!(err.classify(), graded_by_storage);
    }
  }

  /// The bare `From` shorthand grades identically to the tagged constructor.
  #[test]
  fn from_impl_matches_tagged_constructor() {
    let err: DaemonError = pg(Some("40P01"), "deadlock").into();
    assert_eq!(err.classify(), Outcome::Warning);
  }

  /// E2.15: supervisor and shutdown failures were built from `format!` into
  /// the config variant. They keep their Fatal grade — the point is that the
  /// subsystem and the disposition survive as values.
  #[test]
  fn supervisor_failures_are_typed_and_still_fatal() {
    let err = DaemonError::Supervisor {
      subsystem: "ipc",
      failure: SupervisorFailure::Blocked("run mismatch"),
    };
    assert_eq!(err.classify(), Outcome::Fatal);
    assert_eq!(
      err.to_string(),
      "ipc subsystem: recovery blocked: run mismatch"
    );

    let err = DaemonError::Supervisor {
      subsystem: "live",
      failure: SupervisorFailure::Exhausted,
    };
    assert_eq!(err.classify(), Outcome::Fatal);
    match &err {
      DaemonError::Supervisor { subsystem, failure } => {
        assert_eq!(*subsystem, "live");
        assert_eq!(*failure, SupervisorFailure::Exhausted);
      }
      other => assert!(false, "unexpected variant: {other:?}"),
    }

    let err = DaemonError::ShutdownTimeout {
      subsystem: "docs-only server",
      secs: 5,
    };
    assert_eq!(err.classify(), Outcome::Fatal);
    assert_eq!(
      err.to_string(),
      "docs-only server did not shut down within 5s"
    );

    // A step that never answered is not a transient server failure to retry:
    // the readiness window is what was being spent while it hung.
    let err = DaemonError::StorageTimeout {
      context: "connect",
      secs: 120,
    };
    assert_eq!(err.classify(), Outcome::Fatal);
    assert_eq!(
      err.to_string(),
      "storage engine did not answer connect within 120s"
    );
  }

  #[test]
  fn classify_rest_connect_timeout_as_warning() {
    let err = DaemonError::RestSource {
      message: "connect failed".to_owned(),
      kind: RestErrorKind::Connect,
    };
    assert_eq!(err.classify(), Outcome::Warning);
    let err = DaemonError::RestSource {
      message: "timed out".to_owned(),
      kind: RestErrorKind::Timeout,
    };
    assert_eq!(err.classify(), Outcome::Warning);
  }

  #[test]
  fn classify_rest_503_429_as_warning() {
    let err = DaemonError::RestSource {
      message: "service unavailable".to_owned(),
      kind: RestErrorKind::Http(503),
    };
    assert_eq!(err.classify(), Outcome::Warning);
    let err = DaemonError::RestSource {
      message: "too many requests".to_owned(),
      kind: RestErrorKind::Http(429),
    };
    assert_eq!(err.classify(), Outcome::Warning);
  }

  #[test]
  fn classify_rest_other_as_error() {
    let err = DaemonError::RestSource {
      message: "not found".to_owned(),
      kind: RestErrorKind::Other,
    };
    assert_eq!(err.classify(), Outcome::Error);
    let err = DaemonError::RestSource {
      message: "not found".to_owned(),
      kind: RestErrorKind::Http(404),
    };
    assert_eq!(err.classify(), Outcome::Error);
  }

  #[test]
  fn classify_io_index_as_error() {
    assert_eq!(
      DaemonError::Io {
        context: "read",
        source: std::io::Error::new(std::io::ErrorKind::Other, "test")
      }
      .classify(),
      Outcome::Error
    );
    assert_eq!(
      DaemonError::Index("stale".to_owned()).classify(),
      Outcome::Error
    );
  }
}
