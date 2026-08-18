//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Central error type with transient/permanent classification.

use core::fmt;

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
  Db(String),
  RestSource {
    message: String,
    kind: RestErrorKind,
  },
  Config(String),
  Pg {
    message: String,
    code: Option<String>,
  },
  Index(String),
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
      Self::Db(msg) => write!(f, "database error: {msg}"),
      Self::RestSource { message, .. } => write!(f, "data source error: {message}"),
      Self::Config(msg) => write!(f, "config error: {msg}"),
      Self::Pg { message, .. } => write!(f, "postgres error: {message}"),
      Self::Index(msg) => write!(f, "index error: {msg}"),
    }
  }
}

impl std::error::Error for DaemonError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Io { source, .. } => Some(source),
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
      | Self::Config(_) => Outcome::Fatal,

      Self::Pg { code, message } => classify_pg(code.as_deref(), message),

      Self::RestSource { kind, .. } => match kind {
        RestErrorKind::Connect | RestErrorKind::Timeout => Outcome::Warning,
        RestErrorKind::Http(503 | 429) => Outcome::Warning,
        _ => Outcome::Error,
      },

      Self::Io { .. } | Self::Db(_) | Self::Index(_) => Outcome::Error,
    }
  }

  /// Convenience: returns `true` when this error is transient.
  pub fn is_transient(&self) -> bool {
    self.classify() == Outcome::Warning
  }
}

/// SQLSTATE-first, message-fallback classification for Postgres errors.
fn classify_pg(code: Option<&str>, message: &str) -> Outcome {
  if let Some(sqlstate) = code {
    return match sqlstate {
      _ if sqlstate.starts_with("08") => Outcome::Warning, // Connection Exception
      _ if sqlstate.starts_with("40") => Outcome::Warning, // Transaction Rollback (deadlock, serialization)
      _ if sqlstate.starts_with("53") => Outcome::Warning, // Insufficient Resources
      _ if sqlstate.starts_with("57") => Outcome::Warning, // Operator Intervention (admin shutdown)
      "55006" => Outcome::Warning,                         // database in recovery
      _ if sqlstate.starts_with("23") => Outcome::Error,   // Integrity Constraint Violation
      _ if sqlstate.starts_with("42") => Outcome::Error,   // Syntax/Access Rule Violation
      _ => Outcome::Error,
    };
  }
  // Pool-level errors have no SQLSTATE (never reached the DB engine)
  let lower = message.to_ascii_lowercase();
  if [
    "pool closed",
    "pool timed out",
    "failed to acquire connection",
    "connection refused",
    "connection reset",
    "broken pipe",
    "the database system is shutting down",
    "the database system is starting up",
    "could not connect to server",
  ]
  .iter()
  .any(|p| lower.contains(p))
  {
    Outcome::Warning
  } else {
    Outcome::Error
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

  #[test]
  fn classify_pg_transient_by_sqlstate() {
    // 40P01 = deadlock
    let err = DaemonError::Pg {
      message: "deadlock detected".to_owned(),
      code: Some("40P01".to_owned()),
    };
    assert_eq!(err.classify(), Outcome::Warning);
    // 08006 = connection failure
    let err = DaemonError::Pg {
      message: "connection failure".to_owned(),
      code: Some("08006".to_owned()),
    };
    assert_eq!(err.classify(), Outcome::Warning);
    // 57P01 = admin shutdown
    let err = DaemonError::Pg {
      message: "admin shutdown".to_owned(),
      code: Some("57P01".to_owned()),
    };
    assert_eq!(err.classify(), Outcome::Warning);
    // 53300 = too many connections
    let err = DaemonError::Pg {
      message: "too many connections".to_owned(),
      code: Some("53300".to_owned()),
    };
    assert_eq!(err.classify(), Outcome::Warning);
  }

  #[test]
  fn classify_pg_permanent_by_sqlstate() {
    // 23505 = unique violation
    let err = DaemonError::Pg {
      message: "unique constraint".to_owned(),
      code: Some("23505".to_owned()),
    };
    assert_eq!(err.classify(), Outcome::Error);
    // 42P01 = undefined table
    let err = DaemonError::Pg {
      message: "missing table".to_owned(),
      code: Some("42P01".to_owned()),
    };
    assert_eq!(err.classify(), Outcome::Error);
  }

  #[test]
  fn classify_pg_fallback_pool_errors() {
    let err = DaemonError::Pg {
      message: "pool closed".to_owned(),
      code: None,
    };
    assert_eq!(err.classify(), Outcome::Warning);
    assert!(err.is_transient());

    let err = DaemonError::Pg {
      message: "pool timed out waiting for connection".to_owned(),
      code: None,
    };
    assert_eq!(err.classify(), Outcome::Warning);
  }

  #[test]
  fn classify_pg_permanent_no_code() {
    let err = DaemonError::Pg {
      message: "constraint violation on insert".to_owned(),
      code: None,
    };
    assert_eq!(err.classify(), Outcome::Error);
    assert!(!err.is_transient());
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
  fn classify_db_io_index_as_error() {
    assert_eq!(
      DaemonError::Db("corruption".to_owned()).classify(),
      Outcome::Error
    );
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
