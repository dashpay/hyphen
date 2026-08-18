//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Connection configuration for the Postgres backend. Mirrors the daemon's
//! `PostgresConfig` field-for-field so the daemon can pass its parsed config
//! straight through without the crate depending on the daemon's config types.
//!
//! The URL may embed a password, so this type never exposes it: `Debug` and
//! `Serialize` both emit the redacted form. Only `Deserialize` (config-file
//! parsing) and the connection path read the raw value.

use core::fmt;

use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Serialize, Serializer};

use crate::util::redact_url;

pub const DEFAULT_POSTGRES_URL: &str = "postgresql://hyphen:hyphen@127.0.0.1:5432/hyphen";

pub const DEFAULT_WRITE_POOL_SIZE: usize = 20;
pub const DEFAULT_QUERY_POOL_SIZE: usize = 8;
pub const DEFAULT_HEAVY_QUERY_POOL_SIZE: usize = 4;
pub const DEFAULT_ADMIN_POOL_SIZE: usize = 4;
pub const DEFAULT_SLOW_QUERY_MS: u64 = 500;

/// How to connect to PostgreSQL and how to size the workload pools.
#[derive(Clone, Deserialize)]
#[serde(default)]
pub struct HyDbConfig {
  /// Connection URL. Supports the libpq `?host=/path` form for Unix sockets.
  /// May contain a password — never logged or serialized unredacted.
  pub url: String,
  pub write_pool_size: usize,
  pub query_pool_size: usize,
  pub heavy_query_pool_size: usize,
  pub admin_pool_size: usize,
  /// Log statements slower than this threshold (milliseconds).
  pub slow_query_ms: u64,
}

impl Default for HyDbConfig {
  fn default() -> Self {
    Self {
      url: DEFAULT_POSTGRES_URL.to_owned(),
      write_pool_size: DEFAULT_WRITE_POOL_SIZE,
      query_pool_size: DEFAULT_QUERY_POOL_SIZE,
      heavy_query_pool_size: DEFAULT_HEAVY_QUERY_POOL_SIZE,
      admin_pool_size: DEFAULT_ADMIN_POOL_SIZE,
      slow_query_ms: DEFAULT_SLOW_QUERY_MS,
    }
  }
}

// Manual `Debug`: a derived one would print the URL — and with it the Postgres
// password — in any debug dump of a struct that embeds this config.
impl fmt::Debug for HyDbConfig {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("HyDbConfig")
      .field("url", &redact_url(&self.url))
      .field("write_pool_size", &self.write_pool_size)
      .field("query_pool_size", &self.query_pool_size)
      .field("heavy_query_pool_size", &self.heavy_query_pool_size)
      .field("admin_pool_size", &self.admin_pool_size)
      .field("slow_query_ms", &self.slow_query_ms)
      .finish()
  }
}

// Manual `Serialize`: settings snapshots shown to operators serialize this
// config; the URL goes out redacted. (Deserialization still reads the raw
// value from the config file — the asymmetry is deliberate.)
impl Serialize for HyDbConfig {
  fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    let mut s = serializer.serialize_struct("HyDbConfig", 6)?;
    s.serialize_field("url", &redact_url(&self.url))?;
    s.serialize_field("write_pool_size", &self.write_pool_size)?;
    s.serialize_field("query_pool_size", &self.query_pool_size)?;
    s.serialize_field("heavy_query_pool_size", &self.heavy_query_pool_size)?;
    s.serialize_field("admin_pool_size", &self.admin_pool_size)?;
    s.serialize_field("slow_query_ms", &self.slow_query_ms)?;
    s.end()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn secret_cfg() -> HyDbConfig {
    HyDbConfig {
      url: "postgresql://user:s3cr3t@db.example:5432/hyphen".to_owned(),
      ..HyDbConfig::default()
    }
  }

  #[test]
  fn debug_never_prints_the_password() {
    let dump = format!("{:?}", secret_cfg());
    assert!(!dump.contains("s3cr3t"), "password leaked: {dump}");
    assert!(dump.contains("user:***@"));
  }

  #[test]
  fn serialize_never_prints_the_password() {
    let json = serde_json::to_string(&secret_cfg()).unwrap_or_default();
    assert!(!json.contains("s3cr3t"), "password leaked: {json}");
    assert!(json.contains("user:***@"));
  }

  #[test]
  fn deserialize_keeps_the_raw_url() {
    let cfg: HyDbConfig =
      serde_json::from_str(r#"{"url":"postgresql://u:pw@h/db"}"#).unwrap_or_default();
    assert_eq!(cfg.url, "postgresql://u:pw@h/db");
    assert_eq!(cfg.write_pool_size, DEFAULT_WRITE_POOL_SIZE);
  }
}
