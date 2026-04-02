//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Platform-specific default data directory resolution.

use std::path::PathBuf;

use directories::BaseDirs;

/// Resolve the default platform-specific data directory for the daemon.
///
/// Returns `None` when the current operating system has no supported default
/// data directory mapping.
pub fn platform_datadir() -> Option<PathBuf> {
  let base = BaseDirs::new()?;
  #[cfg(target_os = "linux")]
  return Some(base.config_dir().join("hyphen"));
  #[cfg(target_os = "macos")]
  return Some(base.data_dir().join("Hyphen"));
  #[cfg(target_os = "windows")]
  return Some(base.data_local_dir().join("Hyphen"));
  #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
  return Some(base.home_dir().join(".hyphen"));
}
