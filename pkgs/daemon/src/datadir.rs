//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Platform-specific data directory resolution and disk checks.

use std::path::{Path, PathBuf};

use directories::BaseDirs;

use crate::error::DaemonError;

/// Floor on free space in the data directory, below which the daemon refuses
/// to start.
///
/// Sized against what actually lives here, which post-RocksDB is a config
/// file, a lockfile and the rolling logs — one file per process start, kept
/// seven days (`init::logs`). The chain store is in Postgres, possibly on
/// another host, so the old 5 GiB was vestigial sizing for a store this
/// directory no longer holds.
///
/// The volume that can genuinely fill is Postgres's, and this check cannot see
/// it. A server-side free-space check belongs with the startup GUC review
/// (`policy::server_settings`) and needs the storage crate to expose
/// it.
const MIN_FREE_BYTES: u64 = 1024 * 1024 * 1024;

/// Free space below which the daemon starts but says so: a week of
/// debug-level logs can plausibly grow into this band.
const LOW_FREE_BYTES: u64 = 4 * 1024 * 1024 * 1024;

/// What the free-space reading means.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SpaceVerdict {
  /// Room for the logs and then some.
  Ample,
  /// Enough to start, tight enough to mention.
  Low,
  /// Below the floor; the daemon refuses to start.
  Insufficient,
}

/// Grade a free-space reading. Pure, so the thresholds are testable without a
/// filesystem in a particular state.
#[must_use]
pub fn classify_free_space(free: u64) -> SpaceVerdict {
  if free < MIN_FREE_BYTES {
    SpaceVerdict::Insufficient
  } else if free < LOW_FREE_BYTES {
    SpaceVerdict::Low
  } else {
    SpaceVerdict::Ample
  }
}

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

/// Verify the data directory has room for its logs, and report how much.
///
/// Returns the free bytes so the caller can log the actual number rather than
/// an unfalsifiable "disk space OK" — which is what let the check itself go
/// missing once while the reassuring line stayed.
///
/// # Errors
///
/// Returns `DaemonError::Io` when free space cannot be queried, or
/// `DaemonError::InsufficientDisk` when it is below [`MIN_FREE_BYTES`].
pub fn check_free_space(path: &Path) -> Result<u64, DaemonError> {
  let free = fs2::free_space(path).map_err(|e| DaemonError::Io {
    context: "check free space",
    source: e,
  })?;
  match classify_free_space(free) {
    SpaceVerdict::Insufficient => Err(DaemonError::InsufficientDisk {
      available: free,
      required: MIN_FREE_BYTES,
    }),
    SpaceVerdict::Low => {
      tracing::warn!(
        free_bytes = free,
        floor_bytes = MIN_FREE_BYTES,
        "datadir free space is low for seven days of rolling logs"
      );
      Ok(free)
    }
    SpaceVerdict::Ample => Ok(free),
  }
}

#[cfg(test)]
mod tests {
  use super::{classify_free_space, SpaceVerdict, LOW_FREE_BYTES, MIN_FREE_BYTES};

  /// E2.13: the floor is sized for logs, not for a chain store that now lives
  /// in Postgres. A datadir with 2 GiB free must start.
  #[test]
  fn the_floor_is_sized_for_log_retention() {
    assert_eq!(MIN_FREE_BYTES, 1024 * 1024 * 1024);
    assert_eq!(
      classify_free_space(2 * 1024 * 1024 * 1024),
      SpaceVerdict::Low
    );
    assert_eq!(
      classify_free_space(LOW_FREE_BYTES),
      SpaceVerdict::Ample,
      "the old 5 GiB requirement must no longer refuse a working host"
    );
    assert_eq!(
      classify_free_space(MIN_FREE_BYTES - 1),
      SpaceVerdict::Insufficient
    );
    assert_eq!(classify_free_space(0), SpaceVerdict::Insufficient);
    assert_eq!(classify_free_space(MIN_FREE_BYTES), SpaceVerdict::Low);
  }
}
