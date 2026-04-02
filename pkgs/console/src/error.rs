//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Error types for the TUI console.

use std::fmt;

/// Top-level console error covering terminal and platform setup failures.
#[derive(Debug)]
pub enum TuiError {
  Io {
    context: &'static str,
    source: std::io::Error,
  },
  ThreadPanic {
    context: &'static str,
  },
  NoPlatformDataDir,
}

impl fmt::Display for TuiError {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    match self {
      Self::Io { context, source } => write!(f, "{context}: {source}"),
      Self::ThreadPanic { context } => write!(f, "{context}: thread panicked"),
      Self::NoPlatformDataDir => write!(f, "cannot determine platform data directory"),
    }
  }
}

impl std::error::Error for TuiError {
  fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
    match self {
      Self::Io { source, .. } => Some(source),
      _ => None,
    }
  }
}
