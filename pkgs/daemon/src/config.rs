//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Daemon configuration types.

use std::fmt;
use std::path::PathBuf;

use serde::{Deserialize, Serialize};

use crate::error::DaemonError;
use crate::rest::DEFAULT_API_BIND;
use hyphen_db::layout::HyDbLayoutCfg;
use hyphen_db::{HyDbClientCfg, HyDbIngestCfg};

/// Top-level daemon configuration loaded from `hyphen.conf`.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Config {
  pub core: CoreConfig,
  pub api: Option<ApiConfig>,
  pub indices: Option<IndicesConfig>,
  pub live: Option<LiveConfig>,
  pub overlays: Option<OverlaysConfig>,
  #[serde(default)]
  pub chain: ChainConfig,
  /// Everything about the storage engine: what the database *contains*
  /// (`[db.layout]`) and how it is connected to (`[db.client]`).
  #[serde(default)]
  pub db: DbConfig,
}

/// The `[db]` tree. Three parts that must not be confused with each other:
/// `[db.layout]` is immutable for the life of a database and is enforced
/// against `init_params` at startup; `[db.client]` is pure connection
/// configuration; `[db.ingest]` is the write path's runtime tunables. The
/// last two may change on any restart and invalidate nothing.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct DbConfig {
  /// Immutable storage-layout parameters. Deserialized straight into the
  /// engine's own type rather than mirrored here: a mirror would be one more
  /// place for the key set to drift out of step with the contract that
  /// enforces it, and unlike the connection URL there is no secret to redact.
  pub layout: HyDbLayoutCfg,
  /// Connection and per-pool settings. Also the engine's own type: it already
  /// owns the redacting `Debug`/`Serialize` the daemon needs, and a mirrored
  /// copy would have to be kept in step with four pool sub-sections by hand.
  pub client: HyDbClientCfg,
  /// Write-path tunables. The engine's own type for the same reason as the
  /// other two: one definition of the key set, next to the code that reads
  /// it.
  pub ingest: HyDbIngestCfg,
}

/// Connection settings for the node REST interface.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct CoreConfig {
  pub url: String,
  pub rest_concurrency: usize,
  #[serde(default)]
  pub rpc: Option<RpcConfig>,
  #[serde(default)]
  pub zmq: Option<ZmqConfig>,
}

pub const DEFAULT_CORE_URL: &str = "http://127.0.0.1:9998";

pub const DEFAULT_REST_CONCURRENCY: usize = 8;

impl Default for CoreConfig {
  fn default() -> Self {
    Self {
      url: DEFAULT_CORE_URL.to_owned(),
      rest_concurrency: DEFAULT_REST_CONCURRENCY,
      rpc: None,
      zmq: None,
    }
  }
}

/// Authentication mode for node JSON-RPC.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum RpcAuth {
  Cookie,
  UserPass,
}

/// A credential that does not print itself.
///
/// The redaction lives on the type rather than on each struct that holds one, so
/// a new field carrying a secret is redacted by construction and a derived
/// `Debug` or `Serialize` on its container stays safe. Reading the value is
/// deliberately a named call, which makes the few places that legitimately need
/// it greppable.
#[derive(Clone, PartialEq, Eq, Deserialize)]
#[serde(transparent)]
pub struct Secret(String);

impl Secret {
  /// The credential itself, for the call that has to transmit it.
  pub fn expose(&self) -> &str {
    &self.0
  }
}

impl From<String> for Secret {
  fn from(value: String) -> Self {
    Self(value)
  }
}

impl fmt::Debug for Secret {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str("Secret(***)")
  }
}

impl Serialize for Secret {
  fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str("***")
  }
}

/// Resolved JSON-RPC authentication prepared for runtime use.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ResolvedRpcAuth {
  CookieFile(PathBuf),
  UserPass { username: String, password: Secret },
}

/// Optional node JSON-RPC settings used for governance and live state.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct RpcConfig {
  pub enabled: bool,
  pub url: String,
  pub datadir: Option<PathBuf>,
  pub cookie_file: Option<PathBuf>,
  pub username: Option<String>,
  pub password: Option<Secret>,
  pub timeout_secs: u64,
  pub network_refresh_secs: u64,
  pub governance_refresh_secs: u64,
  pub chainlock_refresh_secs: u64,
  pub protx_refresh_secs: u64,
}

pub const DEFAULT_RPC_URL: &str = "http://127.0.0.1:9998";
pub const DEFAULT_RPC_TIMEOUT_SECS: u64 = 10;
pub const DEFAULT_NETWORK_REFRESH_SECS: u64 = 15;
pub const DEFAULT_GOVERNANCE_REFRESH_SECS: u64 = 60;
pub const DEFAULT_CHAINLOCK_REFRESH_SECS: u64 = 5;
pub const DEFAULT_PROTX_REFRESH_SECS: u64 = 30;

impl Default for RpcConfig {
  fn default() -> Self {
    Self {
      enabled: false,
      url: DEFAULT_RPC_URL.to_owned(),
      datadir: None,
      cookie_file: None,
      username: None,
      password: None,
      timeout_secs: DEFAULT_RPC_TIMEOUT_SECS,
      network_refresh_secs: DEFAULT_NETWORK_REFRESH_SECS,
      governance_refresh_secs: DEFAULT_GOVERNANCE_REFRESH_SECS,
      chainlock_refresh_secs: DEFAULT_CHAINLOCK_REFRESH_SECS,
      protx_refresh_secs: DEFAULT_PROTX_REFRESH_SECS,
    }
  }
}

impl RpcConfig {
  /// Resolve the configured authentication mode using explicit credentials
  /// first, then cookie file, then datadir `.cookie`.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::Config` when a partially configured auth method is
  /// present or no auth source can be resolved.
  pub fn resolve_auth(&self) -> Result<ResolvedRpcAuth, DaemonError> {
    match (&self.username, &self.password) {
      (Some(username), Some(password)) => {
        return Ok(ResolvedRpcAuth::UserPass {
          username: username.clone(),
          password: password.clone(),
        });
      }
      (Some(_), None) | (None, Some(_)) => {
        return Err(DaemonError::Config(
          "core.rpc username and password must be set together".to_owned(),
        ));
      }
      (None, None) => {}
    }

    if let Some(cookie_file) = &self.cookie_file {
      return Ok(ResolvedRpcAuth::CookieFile(cookie_file.clone()));
    }

    if let Some(datadir) = &self.datadir {
      return Ok(ResolvedRpcAuth::CookieFile(datadir.join(".cookie")));
    }

    Err(DaemonError::Config(
      "core.rpc requires username/password, cookie_file, or datadir".to_owned(),
    ))
  }

  /// Return the resolved authentication kind for diagnostics and snapshots.
  ///
  /// # Errors
  ///
  /// Returns the same configuration errors as `resolve_auth`.
  pub fn auth_kind(&self) -> Result<RpcAuth, DaemonError> {
    match self.resolve_auth()? {
      ResolvedRpcAuth::CookieFile(_) => Ok(RpcAuth::Cookie),
      ResolvedRpcAuth::UserPass { .. } => Ok(RpcAuth::UserPass),
    }
  }
}

/// Supported node ZMQ topics for live event ingestion.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ZmqTopic {
  RawTx,
  Sequence,
  RawTxLock,
  RawChainLock,
  RawGovernanceObject,
  RawGovernanceVote,
}

/// Optional node ZMQ settings used for live event ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ZmqConfig {
  pub enabled: bool,
  pub connect: Vec<String>,
  pub subscribe: Vec<ZmqTopic>,
}

impl Default for ZmqConfig {
  fn default() -> Self {
    Self {
      enabled: false,
      connect: Vec::new(),
      subscribe: vec![
        ZmqTopic::RawTx,
        ZmqTopic::Sequence,
        ZmqTopic::RawTxLock,
        ZmqTopic::RawChainLock,
        ZmqTopic::RawGovernanceObject,
        ZmqTopic::RawGovernanceVote,
      ],
    }
  }
}

/// Chain-specific parameters that are *not* storage layout.
///
/// The address version bytes used to live here. They moved to `[db.layout]`,
/// where they belong: every address string in the database was encoded with
/// them, so they are a property of the stored data and are enforced as one.
/// `block_time_secs` stays because it is applied at read time (hashrate is
/// derived from `difficulty` on the way out) and changing it rewrites nothing.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ChainConfig {
  pub block_time_secs: f64,
}

impl Default for ChainConfig {
  fn default() -> Self {
    Self {
      block_time_secs: DEFAULT_BLOCK_TIME_SECS,
    }
  }
}

pub const DEFAULT_BLOCK_TIME_SECS: f64 = 150.0;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct ApiConfig {
  pub bind: String,
  /// Allowed CORS origin. If unset, all origins are allowed.
  pub cors_origin: Option<String>,
}

impl Default for ApiConfig {
  fn default() -> Self {
    Self {
      bind: DEFAULT_API_BIND.to_owned(),
      cors_origin: None,
    }
  }
}

/// Optional stage enablement flags for derived indices.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct IndicesConfig {
  #[serde(default)]
  pub address: bool,
  #[serde(default)]
  pub validator: bool,
  #[serde(default)]
  pub metrics: bool,
}

/// Optional live capability and cache settings.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct LiveConfig {
  pub enable_mempool: bool,
  pub enable_islock: bool,
  pub enable_chainlock: bool,
  pub chainlock_start_block: Option<u32>,
  pub enable_governance: bool,
  pub enable_network: bool,
  pub enable_stream: bool,
  pub recent_mempool_txs: usize,
  pub stream_client_buffer: usize,
  pub stream_heartbeat_secs: u64,
}

pub const DEFAULT_RECENT_MEMPOOL_TXS: usize = 250;
pub const DEFAULT_STREAM_CLIENT_BUFFER: usize = 256;
pub const DEFAULT_STREAM_HEARTBEAT_SECS: u64 = 15;

impl Default for LiveConfig {
  fn default() -> Self {
    Self {
      enable_mempool: false,
      enable_islock: false,
      enable_chainlock: false,
      chainlock_start_block: None,
      enable_governance: false,
      enable_network: false,
      enable_stream: false,
      recent_mempool_txs: DEFAULT_RECENT_MEMPOOL_TXS,
      stream_client_buffer: DEFAULT_STREAM_CLIENT_BUFFER,
      stream_heartbeat_secs: DEFAULT_STREAM_HEARTBEAT_SECS,
    }
  }
}

/// Optional overlay capability settings used to enrich indexed APIs.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(default)]
#[serde(deny_unknown_fields)]
pub struct OverlaysConfig {
  pub enable_protx: bool,
  pub enable_chainlock: bool,
}

impl Config {
  /// Build the storage engine's configuration from `[db]`.
  ///
  /// One builder, used by both the serve path and `--show-layout`, so the
  /// diagnostic can never report against a different database than the one
  /// the daemon would open.
  pub fn hydb_config(&self) -> hyphen_db::HyDbConfig {
    hyphen_db::HyDbConfig {
      client: self.db.client.clone(),
      layout: self.db.layout.clone(),
      ingest: self.db.ingest.clone(),
      created_by: format!("hyphend/{}", env!("CARGO_PKG_VERSION")),
    }
  }

  /// Validate configuration that depends on multiple related settings.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::Config` when one enabled feature is missing a
  /// required companion setting.
  pub fn validate(&self) -> Result<(), DaemonError> {
    let live = self.live.clone().unwrap_or_default();
    let overlays = self.overlays.clone().unwrap_or_default();
    let rpc_enabled = self.core.rpc.as_ref().is_some_and(|rpc| rpc.enabled);
    let zmq_enabled = self.core.zmq.as_ref().is_some_and(|zmq| zmq.enabled);

    if live.enable_mempool && !rpc_enabled {
      return Err(DaemonError::Config(
        "live.enable_mempool requires core.rpc.enabled".to_owned(),
      ));
    }
    if live.enable_mempool && !zmq_enabled {
      return Err(DaemonError::Config(
        "live.enable_mempool requires core.zmq.enabled".to_owned(),
      ));
    }
    if live.enable_islock && !zmq_enabled {
      return Err(DaemonError::Config(
        "live.enable_islock requires core.zmq.enabled".to_owned(),
      ));
    }
    if live.enable_network && !rpc_enabled {
      return Err(DaemonError::Config(
        "live.enable_network requires core.rpc.enabled".to_owned(),
      ));
    }
    if live.enable_governance && !rpc_enabled {
      return Err(DaemonError::Config(
        "live.enable_governance requires core.rpc.enabled".to_owned(),
      ));
    }
    if live.enable_stream && !zmq_enabled {
      return Err(DaemonError::Config(
        "live.enable_stream requires core.zmq.enabled".to_owned(),
      ));
    }
    if overlays.enable_protx && !rpc_enabled {
      return Err(DaemonError::Config(
        "overlays.enable_protx requires core.rpc.enabled".to_owned(),
      ));
    }
    if (live.enable_chainlock || overlays.enable_chainlock) && !rpc_enabled {
      return Err(DaemonError::Config(
        "chainlock live/overlay requires core.rpc.enabled".to_owned(),
      ));
    }
    if (live.enable_chainlock || overlays.enable_chainlock) && live.chainlock_start_block.is_none()
    {
      return Err(DaemonError::Config(
        "chainlock live/overlay requires live.chainlock_start_block".to_owned(),
      ));
    }
    Ok(())
  }
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;

  use super::{
    Config, CoreConfig, LiveConfig, ResolvedRpcAuth, RpcAuth, RpcConfig, ZmqConfig, ZmqTopic,
  };

  /// The RPC password must not reach a log line, a debug dump or an IPC frame.
  ///
  /// It used to do all three: `RpcConfig` derived both `Debug` and `Serialize`
  /// straight over a `String`, and `ResolvedRpcAuth` derived `Debug`, so any
  /// `{:?}` of a struct holding one printed the credential.
  #[test]
  fn the_rpc_password_is_redacted_everywhere_it_can_be_seen() {
    let cfg = RpcConfig {
      username: Some("alice".to_owned()),
      password: Some("s3cr3t".to_owned().into()),
      ..RpcConfig::default()
    };

    let dump = format!("{cfg:?}");
    assert!(!dump.contains("s3cr3t"), "password leaked: {dump}");
    let json = serde_json::to_string(&cfg).unwrap_or_default();
    assert!(!json.contains("s3cr3t"), "password leaked: {json}");

    // ...and again once resolved, which is the form the transport holds.
    let resolved = format!("{:?}", cfg.resolve_auth());
    assert!(!resolved.contains("s3cr3t"), "password leaked: {resolved}");

    // The value is still readable by the one caller that must transmit it.
    match cfg.resolve_auth() {
      Ok(ResolvedRpcAuth::UserPass { password, .. }) => {
        assert_eq!(password.expose(), "s3cr3t");
      }
      other => assert!(false, "expected userpass auth, got {other:?}"),
    }
  }

  #[test]
  fn rpc_auth_prefers_userpass_over_cookie_sources() {
    let cfg = RpcConfig {
      username: Some("alice".to_owned()),
      password: Some("secret".to_owned().into()),
      cookie_file: Some(PathBuf::from("/tmp/core.cookie")),
      datadir: Some(PathBuf::from("/tmp/core")),
      ..RpcConfig::default()
    };

    let auth = cfg.resolve_auth().expect("userpass auth should resolve");
    assert!(matches!(
      auth,
      ResolvedRpcAuth::UserPass { username, password }
      if username == "alice" && password.expose() == "secret"
    ));
    assert!(matches!(cfg.auth_kind(), Ok(RpcAuth::UserPass)));
  }

  #[test]
  fn rpc_auth_uses_explicit_cookie_file_before_datadir() {
    let cfg = RpcConfig {
      cookie_file: Some(PathBuf::from("/tmp/core.cookie")),
      datadir: Some(PathBuf::from("/tmp/core")),
      ..RpcConfig::default()
    };

    let auth = cfg.resolve_auth().expect("cookie auth should resolve");
    assert!(matches!(
      auth,
      ResolvedRpcAuth::CookieFile(path)
      if path == PathBuf::from("/tmp/core.cookie")
    ));
    assert!(matches!(cfg.auth_kind(), Ok(RpcAuth::Cookie)));
  }

  #[test]
  fn rpc_auth_falls_back_to_datadir_cookie() {
    let cfg = RpcConfig {
      datadir: Some(PathBuf::from("/tmp/core")),
      ..RpcConfig::default()
    };

    let auth = cfg.resolve_auth().expect("datadir cookie should resolve");
    assert!(matches!(
      auth,
      ResolvedRpcAuth::CookieFile(path)
      if path == PathBuf::from("/tmp/core/.cookie")
    ));
  }

  #[test]
  fn rpc_auth_rejects_partial_userpass() {
    let cfg = RpcConfig {
      username: Some("alice".to_owned()),
      ..RpcConfig::default()
    };

    let err = cfg
      .resolve_auth()
      .expect_err("partial userpass should fail");
    assert!(matches!(
      err,
      crate::error::DaemonError::Config(message)
      if message.contains("username and password")
    ));
  }

  #[test]
  fn rpc_auth_requires_any_auth_source() {
    let err = RpcConfig::default()
      .resolve_auth()
      .expect_err("missing auth should fail");
    assert!(matches!(
      err,
      crate::error::DaemonError::Config(message)
      if message.contains("requires username/password, cookie_file, or datadir")
    ));
  }

  #[test]
  fn zmq_config_subscribes_to_live_topics_by_default() {
    let cfg = ZmqConfig::default();
    assert_eq!(
      cfg.subscribe,
      vec![
        ZmqTopic::RawTx,
        ZmqTopic::Sequence,
        ZmqTopic::RawTxLock,
        ZmqTopic::RawChainLock,
        ZmqTopic::RawGovernanceObject,
        ZmqTopic::RawGovernanceVote,
      ]
    );
  }

  #[test]
  fn config_requires_chainlock_start_block_when_enabled() {
    let cfg = Config {
      core: CoreConfig {
        rpc: Some(RpcConfig {
          enabled: true,
          ..RpcConfig::default()
        }),
        ..CoreConfig::default()
      },
      live: Some(LiveConfig {
        enable_chainlock: true,
        chainlock_start_block: None,
        ..LiveConfig::default()
      }),
      ..Config::default()
    };

    let err = cfg
      .validate()
      .expect_err("missing chainlock start block should fail");
    assert!(matches!(
      err,
      crate::error::DaemonError::Config(message)
      if message.contains("chainlock live/overlay requires live.chainlock_start_block")
    ));
  }

  #[test]
  fn config_allows_disabled_chainlock_without_start_block() {
    let cfg = Config {
      live: Some(LiveConfig {
        enable_chainlock: false,
        chainlock_start_block: None,
        ..LiveConfig::default()
      }),
      ..Config::default()
    };

    assert!(cfg.validate().is_ok());
  }
}
