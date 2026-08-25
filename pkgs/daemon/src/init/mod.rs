//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Early daemon initialization helpers.

pub mod args;
pub mod config;
pub mod logs;
pub mod options;

pub use args::Args;
pub use config::load_config;
pub use logs::init_logs;
pub use options::{Options, StartupMode};

/// Desired minimum soft file-descriptor limit for the daemon process.
#[cfg(not(unix))]
const MIN_OPEN_FILES_SOFT_LIMIT: u64 = 2_048;

/// Desired raised soft file-descriptor limit for Unix daemon processes.
#[cfg(unix)]
const TARGET_OPEN_FILES_SOFT_LIMIT: u64 = 16_384;

/// Best-effort soft file-descriptor tuning for the daemon process.
///
/// Returns the observed soft file-descriptor limit after any attempted raise.
#[cfg(unix)]
pub fn raise_open_file_limit() -> u64 {
  let Ok((mut soft, hard)) = rlimit::Resource::NOFILE.get() else {
    return TARGET_OPEN_FILES_SOFT_LIMIT;
  };

  if soft < TARGET_OPEN_FILES_SOFT_LIMIT {
    soft = TARGET_OPEN_FILES_SOFT_LIMIT.min(hard);
    let _ = rlimit::Resource::NOFILE.set(soft, hard);
    if let Ok((updated_soft, _)) = rlimit::Resource::NOFILE.get() {
      soft = updated_soft;
    }
  }

  soft.min(i32::MAX as u64)
}

/// Best-effort soft file-descriptor tuning for non-Unix targets.
///
/// Returns a conservative fallback limit because Unix-style resource limits are
/// unavailable.
#[cfg(not(unix))]
pub fn raise_open_file_limit() -> u64 {
  MIN_OPEN_FILES_SOFT_LIMIT
}
