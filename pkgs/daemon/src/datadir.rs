//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Platform-specific data directory resolution and disk checks.

use std::path::{Path, PathBuf};

use directories::BaseDirs;

use crate::error::DaemonError;

const MIN_FREE_BYTES: u64 = 5 * 1024 * 1024 * 1024;

/// Resolves the data directory, using the override if provided,
/// otherwise falling back to the platform default.
///
/// # Errors
///
/// Returns `DaemonError::NoPlatformDataDir` when the current platform does
/// not expose a standard base directory.
pub fn resolve(override_path: &Option<PathBuf>) -> Result<PathBuf, DaemonError> {
  if let Some(p) = override_path {
    return Ok(p.clone());
  }
  let base = BaseDirs::new().ok_or(DaemonError::NoPlatformDataDir)?;
  #[cfg(target_os = "linux")]
  return Ok(base.config_dir().join("hyphen"));
  #[cfg(target_os = "macos")]
  return Ok(base.data_dir().join("Hyphen"));
  #[cfg(target_os = "windows")]
  return Ok(base.data_local_dir().join("Hyphen"));
  #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
  return Ok(base.home_dir().join(".hyphen"));
}

/// Creates the data directory if it does not exist.
///
/// # Errors
///
/// Returns `DaemonError::Io` when the directory cannot be created.
pub fn ensure_exists(path: &Path) -> Result<(), DaemonError> {
  std::fs::create_dir_all(path).map_err(|e| DaemonError::Io {
    context: "create datadir",
    source: e,
  })
}

/// Verifies at least 5 GB of free space is available.
///
/// # Errors
///
/// Returns `DaemonError::Io` when free space cannot be queried, or
/// `DaemonError::InsufficientDisk` when the threshold is not met.
pub fn check_free_space(path: &Path) -> Result<(), DaemonError> {
  let free = fs2::free_space(path).map_err(|e| DaemonError::Io {
    context: "check free space",
    source: e,
  })?;
  if free < MIN_FREE_BYTES {
    return Err(DaemonError::InsufficientDisk {
      available: free,
      required: MIN_FREE_BYTES,
    });
  }
  Ok(())
}
