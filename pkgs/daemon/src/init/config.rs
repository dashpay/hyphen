//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Strict daemon config loading.

use std::fs;
use std::path::Path;

use crate::config::Config;
use crate::error::DaemonError;

/// Load `hyphen.conf` from the data directory.
///
/// Missing configuration files fall back to defaults.
///
/// # Errors
///
/// Returns `DaemonError::Io` when reading the file fails, or
/// `DaemonError::Config` when TOML deserialization or validation fails.
pub fn load_config(datadir: &Path) -> Result<Config, DaemonError> {
  let path = datadir.join("hyphen.conf");
  if !path.exists() {
    tracing::warn!(
      path = %path.display(),
      "hyphen.conf not found, using defaults"
    );
    return Ok(Config::default());
  }
  let text = fs::read_to_string(&path).map_err(|e| DaemonError::Io {
    context: "read hyphen.conf",
    source: e,
  })?;
  let cfg: Config = toml::from_str(&text).map_err(|e| DaemonError::Config(e.to_string()))?;
  if cfg.rocksdb.is_some() {
    tracing::warn!(
      "hyphen.conf contains a [rocksdb] section; RocksDB was removed and the \
       section is ignored — chain storage is PostgreSQL-only"
    );
  }
  cfg.validate()?;
  tracing::info!(path = %path.display(), "hyphen.conf loaded");
  Ok(cfg)
}

#[cfg(test)]
mod tests {
  #[test]
  fn config_rejects_unknown_top_level_keys() {
    let err = toml::from_str::<crate::config::Config>(
      r#"
        [core]
        url = "http://127.0.0.1:9998"
        nonsense = true
      "#,
    )
    .expect_err("unknown config key should fail");
    assert!(err.to_string().contains("unknown field"));
  }

  #[test]
  fn config_rejects_old_live_key_names() {
    let err = toml::from_str::<crate::config::Config>(
      r#"
        [core]
        url = "http://127.0.0.1:9998"

        [live]
        mempool = true
      "#,
    )
    .expect_err("legacy live key should fail");
    assert!(err.to_string().contains("unknown field"));
  }
}
