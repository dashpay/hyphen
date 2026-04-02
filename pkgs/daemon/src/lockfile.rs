//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Exclusive PID lockfile to prevent multiple daemon instances.

use core::fmt;
use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Path, PathBuf};

use fs2::FileExt;

use crate::error::DaemonError;

/// Holds an exclusive lock on `hyphen.lock` inside the data directory.
/// The file is removed on explicit release.
pub struct Lock {
  path: PathBuf,
  _file: File,
}

impl fmt::Debug for Lock {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("Lock").field("path", &self.path).finish()
  }
}

/// Acquires an exclusive lockfile, writing the current PID.
///
/// # Errors
///
/// Returns `DaemonError::AlreadyRunning` when another daemon instance holds
/// the lock, or `DaemonError::Io` when the lockfile cannot be created or
/// updated.
pub fn acquire(datadir: &Path) -> Result<Lock, DaemonError> {
  let path = datadir.join("hyphen.lock");
  let file = OpenOptions::new()
    .create(true)
    .write(true)
    .truncate(true)
    .open(&path)
    .map_err(|e| DaemonError::Io {
      context: "open lockfile",
      source: e,
    })?;

  file
    .try_lock_exclusive()
    .map_err(|_| DaemonError::AlreadyRunning)?;

  let pid = std::process::id();
  (&file)
    .write_fmt(format_args!("{pid}"))
    .map_err(|e| DaemonError::Io {
      context: "write lockfile",
      source: e,
    })?;

  Ok(Lock { path, _file: file })
}

impl Lock {
  /// Releases the lock and removes the lockfile.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::Io` when the lockfile cannot be removed.
  pub fn release(self) -> Result<(), DaemonError> {
    let path = self.path.clone();
    drop(self._file);
    std::fs::remove_file(&path).map_err(|e| DaemonError::Io {
      context: "remove lockfile",
      source: e,
    })
  }
}
