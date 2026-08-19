//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! `[db]` as the storage engine sees it: `[db.client]` (how to connect, and
//! how each of the four workload pools behaves) plus `[db.layout]` (what the
//! database contains — see [`crate::layout`]).
//!
//! # Why the pools are configured separately
//!
//! The four pools exist because the workloads are genuinely different, and the
//! per-session settings are where that difference is *expressed*. They are
//! applied with `SET` on each connection as it is established, which is
//! strictly better than the same values set cluster-wide: `synchronous_commit
//! = off` gives the bulk-load win on the ingest writer without making an
//! API-driven write non-durable, and a `statement_timeout` that caps a runaway
//! analytical read must not be able to kill a block-connect transaction
//! halfway through.
//!
//! # Secrets
//!
//! The URL may embed a password, so this type never exposes it: `Debug` and
//! `Serialize` both emit the redacted form. Only `Deserialize` (config-file
//! parsing) and the connection path read the raw value.

use core::fmt;

use serde::ser::SerializeStruct as _;
use serde::{Deserialize, Serialize, Serializer};

use crate::error::HyDbError;
use crate::layout::{HyDbLayoutCfg, DEFAULT_PARTITION_SPAN};
use crate::tuning::HyDbIngestCfg;
use crate::util::redact_url;

pub const DEFAULT_POSTGRES_URL: &str = "postgresql://hyphen:hyphen@127.0.0.1:5432/hyphen";

/// Default `application_name` prefix. Each pool appends its own name, so
/// `pg_stat_activity` attributes a backend to the workload that owns it.
pub const DEFAULT_APP_NAME: &str = "hyphend";

pub const DEFAULT_SLOW_QUERY_MS: u64 = 500;
pub const DEFAULT_CONNECT_TIMEOUT: u64 = 10;
pub const DEFAULT_IDLE_TIMEOUT: u64 = 600;
pub const DEFAULT_MAX_LIFETIME: u64 = 1800;
pub const DEFAULT_STMT_CACHE_SIZE: usize = 100;

pub const DEFAULT_WRITE_POOL_SIZE: usize = 32;
pub const DEFAULT_QUERY_POOL_SIZE: usize = 16;
pub const DEFAULT_HEAVY_POOL_SIZE: usize = 8;
pub const DEFAULT_ADMIN_POOL_SIZE: usize = 4;

/// Seconds the admin pool waits for a free connection. Not a config key:
/// `[db.client.admin]` is `size` and `maint_work_mem` only. The admin pool
/// serves maintenance and diagnostics, where a wait longer than this means
/// something is wrong that a longer wait will not fix.
const ADMIN_ACQUIRE_TIMEOUT: u64 = 2;

/// Everything the engine is given: `[db.client]` and `[db.layout]`.
#[derive(Debug, Clone)]
pub struct HyDbConfig {
  /// `[db.client]` — how to connect, and how each pool behaves.
  pub client: HyDbClientCfg,
  /// `[db.layout]` — the immutable storage-layout contract. Compared against
  /// `init_params` on connect; see [`crate::layout`].
  pub layout: HyDbLayoutCfg,
  /// `[db.ingest]` — the runtime write-path tunables; see [`crate::tuning`].
  pub ingest: HyDbIngestCfg,
  /// Identifier recorded in `init_params.created_by` when this process writes
  /// a layout key. Set by the daemon to its own version; not a config key.
  pub created_by: String,
}

impl Default for HyDbConfig {
  fn default() -> Self {
    Self {
      client: HyDbClientCfg::default(),
      layout: HyDbLayoutCfg::default(),
      ingest: HyDbIngestCfg::default(),
      created_by: format!("hyphen-db/{}", env!("CARGO_PKG_VERSION")),
    }
  }
}

impl HyDbConfig {
  /// Check the `[db.client]` values that are interpolated into `SET`
  /// statements, the sizes that must be non-zero, and the `[db.ingest]`
  /// tunables, before any connection is attempted.
  ///
  /// `[db.ingest] stepped_sync_gap` is checked against the *configured*
  /// partition span, or the built-in default when the operator left the key
  /// out. That is the best available answer before a connection exists;
  /// [`crate::HyphenDb::connect`] repeats the check against the span the
  /// database actually has, which is the authoritative one.
  ///
  /// # Errors
  ///
  /// [`HyDbError::Config`] naming the offending key.
  pub fn validate(&self) -> Result<(), HyDbError> {
    self.client.validate()?;
    self
      .ingest
      .validate(self.layout.partition_span.unwrap_or(DEFAULT_PARTITION_SPAN))
  }
}

/// `[db.client]`: how to connect to PostgreSQL, and how each workload pool
/// behaves.
#[derive(Clone, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HyDbClientCfg {
  /// Unix-socket **directory** (not the socket file). When set it wins over
  /// the URL's host: the pipeline is round-trip bound — backends measured
  /// 86–100% idle — so removing the loopback TCP stack from every round trip
  /// is worth more here than it would be on a CPU-bound workload. Credentials,
  /// database name and port still come from `url`, exactly as libpq's
  /// `?host=/path` form works.
  pub socket: Option<String>,
  /// Connection URL, and the fallback when no socket is configured. Also the
  /// source of user/password/database/port when one is. May contain a
  /// password — never logged or serialized unredacted.
  ///
  /// TLS is deliberately out of scope: the engine is built with `tls-none`.
  pub url: String,
  /// `application_name` prefix; each pool appends `/write`, `/query`,
  /// `/heavy` or `/admin`.
  pub app_name: String,
  /// Log statements slower than this threshold (milliseconds).
  pub slow_query_ms: u64,
  /// Seconds to wait for the pool's first connection before giving up.
  pub connect_timeout: u64,
  /// Seconds an idle pooled connection is kept before being closed. 0 keeps
  /// idle connections forever.
  pub idle_timeout: u64,
  /// Seconds any connection is kept, idle or not. 0 disables recycling.
  pub max_lifetime: u64,
  /// Prepared statements cached per connection. 0 disables the cache.
  pub stmt_cache_size: usize,

  /// `[db.client.write]` — the single-writer ingest path.
  pub write: HyDbWriteCfg,
  /// `[db.client.query]` — public, latency-sensitive reads.
  pub query: HyDbQueryCfg,
  /// `[db.client.heavy]` — heavy analytical reads.
  pub heavy: HyDbHeavyCfg,
  /// `[db.client.admin]` — maintenance, index builds and diagnostics.
  pub admin: HyDbAdminCfg,
}

/// `[db.client.write]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HyDbWriteCfg {
  pub size: usize,
  /// Seconds to wait for a free connection.
  pub acquire_timeout: u64,
  /// Milliseconds before the server cancels a statement. **0 = unbounded, and
  /// that is the default here on purpose**: a block connect is one atomic
  /// transaction covering every stage, and a timeout that fires halfway
  /// through buys nothing — the work is rolled back and immediately retried.
  pub statement_timeout: u64,
  /// `synchronous_commit`. Defaults to `off` *for this pool only*, which is
  /// the entire reason the setting is per-pool: the ingest writer gets the
  /// bulk-load win, and nothing else in the daemon becomes less durable.
  /// Safe here because it trades durability of the most recent commits, never
  /// consistency, and everything the ingest writes is re-derivable from the
  /// node — a crash costs seconds of re-sync, not correctness.
  pub sync_commit: String,
  /// `jit`. Off: the ingest issues short, parameterized, repeatedly-planned
  /// statements, where JIT compilation is pure overhead.
  pub jit: String,
}

/// `[db.client.query]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HyDbQueryCfg {
  pub size: usize,
  /// Seconds to wait for a free connection.
  pub acquire_timeout: u64,
  /// Milliseconds before the server cancels a statement. Non-zero here: a
  /// read that runs long must be cancelled at the server rather than held
  /// until it exhausts the pool and takes the API down with it.
  pub statement_timeout: u64,
  pub jit: String,
}

/// `[db.client.heavy]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HyDbHeavyCfg {
  pub size: usize,
  /// Seconds to wait for a free connection.
  pub acquire_timeout: u64,
  /// Milliseconds before the server cancels a statement.
  pub statement_timeout: u64,
  /// `work_mem` for this pool's sessions. Sorts and hashes on the analytical
  /// reads spill to disk below this; the pool is small precisely so this can
  /// be generous without multiplying across every backend.
  pub work_mem: String,
}

/// `[db.client.admin]`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HyDbAdminCfg {
  pub size: usize,
  /// `maintenance_work_mem` for this pool's sessions. This is the direct
  /// input to index-build speed — measured 6.47 s per 3M rows at 2 GB — which
  /// is what the stepped-sync mode's deferred builds will be paying.
  pub maint_work_mem: String,
}

impl HyDbAdminCfg {
  pub(crate) const fn acquire_timeout(&self) -> u64 {
    ADMIN_ACQUIRE_TIMEOUT
  }
}

impl Default for HyDbWriteCfg {
  fn default() -> Self {
    Self {
      size: DEFAULT_WRITE_POOL_SIZE,
      acquire_timeout: 10,
      statement_timeout: 0,
      sync_commit: "off".to_owned(),
      jit: "off".to_owned(),
    }
  }
}

impl Default for HyDbQueryCfg {
  fn default() -> Self {
    Self {
      size: DEFAULT_QUERY_POOL_SIZE,
      acquire_timeout: 2,
      statement_timeout: 5_000,
      jit: "off".to_owned(),
    }
  }
}

impl Default for HyDbHeavyCfg {
  fn default() -> Self {
    Self {
      size: DEFAULT_HEAVY_POOL_SIZE,
      acquire_timeout: 2,
      statement_timeout: 60_000,
      work_mem: "64MB".to_owned(),
    }
  }
}

impl Default for HyDbAdminCfg {
  fn default() -> Self {
    Self {
      size: DEFAULT_ADMIN_POOL_SIZE,
      maint_work_mem: "1GB".to_owned(),
    }
  }
}

impl Default for HyDbClientCfg {
  fn default() -> Self {
    Self {
      socket: None,
      url: DEFAULT_POSTGRES_URL.to_owned(),
      app_name: DEFAULT_APP_NAME.to_owned(),
      slow_query_ms: DEFAULT_SLOW_QUERY_MS,
      connect_timeout: DEFAULT_CONNECT_TIMEOUT,
      idle_timeout: DEFAULT_IDLE_TIMEOUT,
      max_lifetime: DEFAULT_MAX_LIFETIME,
      stmt_cache_size: DEFAULT_STMT_CACHE_SIZE,
      write: HyDbWriteCfg::default(),
      query: HyDbQueryCfg::default(),
      heavy: HyDbHeavyCfg::default(),
      admin: HyDbAdminCfg::default(),
    }
  }
}

/// Accepted `synchronous_commit` values.
const SYNC_COMMIT_VALUES: [&str; 5] = ["on", "off", "local", "remote_write", "remote_apply"];
/// Accepted `jit` values.
const TOGGLE_VALUES: [&str; 2] = ["on", "off"];

/// Validate a GUC value that will be interpolated into a `SET` statement.
///
/// These four values (`sync_commit`, `jit`, `work_mem`, `maint_work_mem`)
/// cannot be sent as bind parameters — `SET` takes no placeholders — so they
/// are checked against a closed set or a strict grammar before they reach a
/// statement. An operator's config file is not a hostile input, but a
/// config-to-SQL path with no validation is one bad deployment away from
/// being one, and the error message is better than the syntax error would be.
fn check_enum(section: &str, key: &str, value: &str, allowed: &[&str]) -> Result<(), HyDbError> {
  if allowed.contains(&value) {
    return Ok(());
  }
  Err(HyDbError::Config(format!(
    "db.client.{section}.{key} = \"{value}\" is not valid; expected one of {}",
    allowed.join(", ")
  )))
}

/// Validate a Postgres memory quantity: digits followed by an optional unit.
fn check_memory(section: &str, key: &str, value: &str) -> Result<(), HyDbError> {
  let trimmed = value.trim();
  let digits_end = trimmed
    .find(|c: char| !c.is_ascii_digit())
    .unwrap_or(trimmed.len());
  let (digits, unit) = trimmed.split_at(digits_end);
  let unit_ok = matches!(unit, "" | "kB" | "MB" | "GB" | "TB");
  if !digits.is_empty() && digits.parse::<u64>().is_ok_and(|n| n > 0) && unit_ok {
    return Ok(());
  }
  Err(HyDbError::Config(format!(
    "db.client.{section}.{key} = \"{value}\" is not a Postgres memory quantity \
     (a positive integer with an optional kB/MB/GB/TB unit, e.g. \"64MB\")"
  )))
}

impl HyDbClientCfg {
  /// Check the values that are interpolated into `SET` statements, and the
  /// sizes that must be non-zero, before any connection is attempted.
  ///
  /// # Errors
  ///
  /// [`HyDbError::Config`] naming the offending key.
  pub fn validate(&self) -> Result<(), HyDbError> {
    check_enum(
      "write",
      "sync_commit",
      &self.write.sync_commit,
      &SYNC_COMMIT_VALUES,
    )?;
    check_enum("write", "jit", &self.write.jit, &TOGGLE_VALUES)?;
    check_enum("query", "jit", &self.query.jit, &TOGGLE_VALUES)?;
    check_memory("heavy", "work_mem", &self.heavy.work_mem)?;
    check_memory("admin", "maint_work_mem", &self.admin.maint_work_mem)?;
    for (section, size) in [
      ("write", self.write.size),
      ("query", self.query.size),
      ("heavy", self.heavy.size),
      ("admin", self.admin.size),
    ] {
      if size == 0 {
        return Err(HyDbError::Config(format!(
          "db.client.{section}.size must be at least 1"
        )));
      }
    }
    // A zero timeout is taken literally by the pool: every connect, or every
    // acquire, fails the instant it is attempted. Rejected at load rather than
    // left to present itself at runtime as a storage engine that times out
    // immediately and always, which reads like a dead server.
    if self.connect_timeout == 0 {
      return Err(HyDbError::Config(
        "db.client.connect_timeout must be at least 1 second — zero is taken \
         literally and fails every connection attempt outright"
          .to_owned(),
      ));
    }
    for (section, acquire_timeout) in [
      ("write", self.write.acquire_timeout),
      ("query", self.query.acquire_timeout),
      ("heavy", self.heavy.acquire_timeout),
      ("admin", self.admin.acquire_timeout()),
    ] {
      if acquire_timeout == 0 {
        return Err(HyDbError::Config(format!(
          "db.client.{section}.acquire_timeout must be at least 1 second — zero \
           is taken literally and fails every checkout outright"
        )));
      }
    }
    if self.app_name.trim().is_empty() {
      return Err(HyDbError::Config(
        "db.client.app_name must not be empty — it is what attributes backends \
         to a pool in pg_stat_activity"
          .to_owned(),
      ));
    }
    Ok(())
  }
}

impl HyDbClientCfg {
  /// A loggable, credential-free description of what the pools connect to.
  /// Shows the socket when one is configured, because that is what is actually
  /// in use — printing the URL's TCP host in that case would be a lie an
  /// operator would then try to debug.
  pub fn endpoint(&self) -> String {
    match self
      .socket
      .as_deref()
      .map(str::trim)
      .filter(|s| !s.is_empty())
    {
      Some(dir) => format!("unix:{dir}"),
      None => redact_url(&self.url),
    }
  }
}

// Manual `Debug`: a derived one would print the URL — and with it the Postgres
// password — in any debug dump of a struct that embeds this config.
impl fmt::Debug for HyDbClientCfg {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("HyDbClientCfg")
      .field("socket", &self.socket)
      .field("url", &redact_url(&self.url))
      .field("app_name", &self.app_name)
      .field("slow_query_ms", &self.slow_query_ms)
      .field("connect_timeout", &self.connect_timeout)
      .field("idle_timeout", &self.idle_timeout)
      .field("max_lifetime", &self.max_lifetime)
      .field("stmt_cache_size", &self.stmt_cache_size)
      .field("write", &self.write)
      .field("query", &self.query)
      .field("heavy", &self.heavy)
      .field("admin", &self.admin)
      .finish()
  }
}

// Manual `Serialize`: settings snapshots shown to operators serialize this
// config; the URL goes out redacted. (Deserialization still reads the raw
// value from the config file — the asymmetry is deliberate.)
impl Serialize for HyDbClientCfg {
  fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    let mut s = serializer.serialize_struct("HyDbClientCfg", 12)?;
    s.serialize_field("socket", &self.socket)?;
    s.serialize_field("url", &redact_url(&self.url))?;
    s.serialize_field("app_name", &self.app_name)?;
    s.serialize_field("slow_query_ms", &self.slow_query_ms)?;
    s.serialize_field("connect_timeout", &self.connect_timeout)?;
    s.serialize_field("idle_timeout", &self.idle_timeout)?;
    s.serialize_field("max_lifetime", &self.max_lifetime)?;
    s.serialize_field("stmt_cache_size", &self.stmt_cache_size)?;
    s.serialize_field("write", &self.write)?;
    s.serialize_field("query", &self.query)?;
    s.serialize_field("heavy", &self.heavy)?;
    s.serialize_field("admin", &self.admin)?;
    s.end()
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn secret_cfg() -> HyDbClientCfg {
    HyDbClientCfg {
      url: "postgresql://user:s3cr3t@db.example:5432/hyphen".to_owned(),
      ..HyDbClientCfg::default()
    }
  }

  /// Passwords that leaked while the two tests below passed. Both hold a
  /// character that used to end the authority ahead of the `@`, so the
  /// redaction found no `@` and returned the URL whole. A password with no
  /// reserved character cannot catch that, which is why these are separate.
  fn awkward_secret_cfgs() -> Vec<(HyDbClientCfg, &'static str)> {
    ["pa/ss", "pa?ss", "p/a?s/s"]
      .into_iter()
      .map(|secret| {
        (
          HyDbClientCfg {
            url: format!("postgresql://user:{secret}@db.example:5432/hyphen"),
            ..HyDbClientCfg::default()
          },
          secret,
        )
      })
      .collect()
  }

  #[test]
  fn debug_never_prints_the_password() {
    let dump = format!("{:?}", secret_cfg());
    assert!(!dump.contains("s3cr3t"), "password leaked: {dump}");
    assert!(dump.contains("user:***@"));

    for (cfg, secret) in awkward_secret_cfgs() {
      let dump = format!("{cfg:?}");
      assert!(!dump.contains(secret), "password {secret} leaked: {dump}");
      assert!(dump.contains("user:***@"));
    }
  }

  #[test]
  fn serialize_never_prints_the_password() {
    let json = serde_json::to_string(&secret_cfg()).unwrap_or_default();
    assert!(!json.contains("s3cr3t"), "password leaked: {json}");
    assert!(json.contains("user:***@"));

    for (cfg, secret) in awkward_secret_cfgs() {
      let json = serde_json::to_string(&cfg).unwrap_or_default();
      assert!(!json.contains(secret), "password {secret} leaked: {json}");
      assert!(json.contains("user:***@"));
    }
  }

  #[test]
  fn endpoint_never_prints_the_password() {
    // `endpoint()` is what the pool-creation log line renders on every
    // startup, so it is the widest of the three exposures.
    for (cfg, secret) in awkward_secret_cfgs() {
      let shown = cfg.endpoint();
      assert!(!shown.contains(secret), "password {secret} leaked: {shown}");
      assert!(shown.contains("user:***@"));
    }
  }

  #[test]
  fn deserialize_keeps_the_raw_url() {
    let cfg: HyDbClientCfg =
      serde_json::from_str(r#"{"url":"postgresql://u:pw@h/db"}"#).unwrap_or_default();
    assert_eq!(cfg.url, "postgresql://u:pw@h/db");
    assert_eq!(cfg.write.size, DEFAULT_WRITE_POOL_SIZE);
  }

  #[test]
  fn every_client_key_fits_the_24_character_budget() {
    // Section prefixes are excluded; these are the leaf names as written in
    // the config file.
    const KEYS: [&str; 17] = [
      "socket",
      "url",
      "app_name",
      "slow_query_ms",
      "connect_timeout",
      "idle_timeout",
      "max_lifetime",
      "stmt_cache_size",
      "size",
      "acquire_timeout",
      "statement_timeout",
      "sync_commit",
      "jit",
      "work_mem",
      "maint_work_mem",
      "network",
      "partition_span",
    ];
    for key in KEYS {
      assert!(key.len() <= 24, "config key `{key}` is {} chars", key.len());
    }
  }

  #[test]
  fn default_write_pool_is_unbounded_and_asynchronous() {
    let cfg = HyDbClientCfg::default();
    assert_eq!(
      cfg.write.statement_timeout, 0,
      "a block connect is one atomic transaction and must not be killed mid-flight"
    );
    assert_eq!(cfg.write.sync_commit, "off");
    // ...and the other pools must not inherit either of those.
    assert!(cfg.query.statement_timeout > 0);
    assert!(cfg.heavy.statement_timeout > 0);
  }

  #[test]
  fn validate_rejects_values_that_would_reach_a_set_statement() {
    let bad_sync = HyDbClientCfg {
      write: HyDbWriteCfg {
        sync_commit: "off; DROP TABLE blocks".to_owned(),
        ..HyDbWriteCfg::default()
      },
      ..HyDbClientCfg::default()
    };
    assert!(bad_sync.validate().is_err());

    let bad_mem = HyDbClientCfg {
      heavy: HyDbHeavyCfg {
        work_mem: "64MB'; SELECT 1--".to_owned(),
        ..HyDbHeavyCfg::default()
      },
      ..HyDbClientCfg::default()
    };
    assert!(bad_mem.validate().is_err());

    let bad_jit = HyDbClientCfg {
      query: HyDbQueryCfg {
        jit: "maybe".to_owned(),
        ..HyDbQueryCfg::default()
      },
      ..HyDbClientCfg::default()
    };
    assert!(bad_jit.validate().is_err());

    let zero_pool = HyDbClientCfg {
      admin: HyDbAdminCfg {
        size: 0,
        ..HyDbAdminCfg::default()
      },
      ..HyDbClientCfg::default()
    };
    assert!(zero_pool.validate().is_err());

    assert!(HyDbClientCfg::default().validate().is_ok());
  }

  /// A zero timeout is a configuration that cannot work, not a configuration
  /// meaning "no limit": the pool takes it literally and fails every attempt at
  /// once, which at runtime looks like a server that is down.
  #[test]
  fn validate_rejects_timeouts_of_zero() {
    let zero_connect = HyDbClientCfg {
      connect_timeout: 0,
      ..HyDbClientCfg::default()
    };
    let err = zero_connect
      .validate()
      .err()
      .map(|e| e.to_string())
      .unwrap_or_default();
    assert!(err.contains("connect_timeout"), "{err}");

    let zero_acquire = HyDbClientCfg {
      query: HyDbQueryCfg {
        acquire_timeout: 0,
        ..HyDbQueryCfg::default()
      },
      ..HyDbClientCfg::default()
    };
    let err = zero_acquire
      .validate()
      .err()
      .map(|e| e.to_string())
      .unwrap_or_default();
    assert!(err.contains("query.acquire_timeout"), "{err}");
  }

  #[test]
  fn validate_accepts_the_memory_forms_postgres_accepts() {
    for good in ["64MB", "2GB", "8192kB", "1024", "1TB"] {
      assert!(check_memory("heavy", "work_mem", good).is_ok(), "{good}");
    }
    for bad in ["", "MB", "0MB", "-1MB", "64 MB", "64mb", "64MiB"] {
      assert!(check_memory("heavy", "work_mem", bad).is_err(), "{bad}");
    }
  }
}
