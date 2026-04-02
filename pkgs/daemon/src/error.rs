//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Central error type with transient/permanent classification.

use core::fmt;

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
  RestSource(String),
  Config(String),
  Pg(String),
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
      Self::RestSource(msg) => write!(f, "data source error: {msg}"),
      Self::Config(msg) => write!(f, "config error: {msg}"),
      Self::Pg(msg) => write!(f, "postgres error: {msg}"),
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
