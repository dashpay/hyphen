//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Derived runtime options resolved from CLI args and config.

use crate::config::{
  self, ApiConfig, Config, DEFAULT_CHAINLOCK_REFRESH_SECS, DEFAULT_GOVERNANCE_REFRESH_SECS,
  DEFAULT_NETWORK_REFRESH_SECS, DEFAULT_PROTX_REFRESH_SECS,
};
use crate::error::DaemonError;
use crate::init::args::Args;
use hyphend::wire::msg::{
  ConfigApiSnapshot, ConfigIndexSnapshot, ConfigLiveSnapshot, ConfigOverlaySnapshot,
  ConfigPostgresSnapshot, ConfigSnapshot, ConfigSourcesSnapshot,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartupMode {
  Serve,
  DocsOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct IndexOptions {
  pub address: bool,
  pub validator: bool,
  pub metrics: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct LiveOptions {
  pub mempool: bool,
  pub islock: bool,
  pub chainlock: bool,
  pub governance: bool,
  pub network: bool,
  pub stream: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayOptions {
  pub protx: bool,
  pub chainlock: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApiOptions {
  pub enabled: bool,
  pub bind: Option<String>,
  pub cors_origin: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Options {
  pub startup_mode: StartupMode,
  pub indices: IndexOptions,
  pub live: LiveOptions,
  pub overlays: OverlayOptions,
  pub api: ApiOptions,
  pub recent_mempool_txs: usize,
  pub stream_client_buffer: usize,
  pub stream_heartbeat_secs: u64,
  pub chainlock_start_block: Option<u32>,
  pub network_refresh_secs: u64,
  pub governance_refresh_secs: u64,
  pub chainlock_refresh_secs: u64,
  pub protx_refresh_secs: u64,
}

impl Options {
  /// Resolve runtime options from parsed args and validated config.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::Config` when the combined startup policy is invalid.
  pub fn resolve(args: &Args, cfg: &Config) -> Result<Self, DaemonError> {
    let startup_mode = if args.docs_only {
      StartupMode::DocsOnly
    } else {
      StartupMode::Serve
    };

    let live_cfg = cfg.live.clone().unwrap_or_default();
    let overlay_cfg = cfg.overlays.clone().unwrap_or_default();
    if startup_mode != StartupMode::DocsOnly {
      let rpc_enabled = cfg.core.rpc.as_ref().is_some_and(|rpc| rpc.enabled);
      let zmq_enabled = cfg.core.zmq.as_ref().is_some_and(|zmq| zmq.enabled);
      if live_cfg.enable_mempool && !rpc_enabled {
        return Err(DaemonError::Config(
          "live.enable_mempool requires core.rpc.enabled".to_owned(),
        ));
      }
      if live_cfg.enable_mempool && !zmq_enabled {
        return Err(DaemonError::Config(
          "live.enable_mempool requires core.zmq.enabled".to_owned(),
        ));
      }
      if live_cfg.enable_islock && !zmq_enabled {
        return Err(DaemonError::Config(
          "live.enable_islock requires core.zmq.enabled".to_owned(),
        ));
      }
      if live_cfg.enable_network && !rpc_enabled {
        return Err(DaemonError::Config(
          "live.enable_network requires core.rpc.enabled".to_owned(),
        ));
      }
      if live_cfg.enable_governance && !rpc_enabled {
        return Err(DaemonError::Config(
          "live.enable_governance requires core.rpc.enabled".to_owned(),
        ));
      }
      if live_cfg.enable_stream && !zmq_enabled {
        return Err(DaemonError::Config(
          "live.enable_stream requires core.zmq.enabled".to_owned(),
        ));
      }
      if overlay_cfg.enable_protx && !rpc_enabled {
        return Err(DaemonError::Config(
          "overlays.enable_protx requires core.rpc.enabled".to_owned(),
        ));
      }

      let chainlock_requested = live_cfg.enable_chainlock || overlay_cfg.enable_chainlock;
      if chainlock_requested && !rpc_enabled {
        return Err(DaemonError::Config(
          "chainlock live/overlay requires core.rpc.enabled".to_owned(),
        ));
      }
      if chainlock_requested && live_cfg.chainlock_start_block.is_none() {
        return Err(DaemonError::Config(
          "chainlock live/overlay requires live.chainlock_start_block".to_owned(),
        ));
      }
    }

    Ok(Self {
      startup_mode,
      indices: IndexOptions {
        address: cfg.indices.as_ref().is_some_and(|i| i.address),
        validator: cfg.indices.as_ref().is_some_and(|i| i.validator),
        metrics: cfg.indices.as_ref().is_some_and(|i| i.metrics),
      },
      live: LiveOptions {
        mempool: live_cfg.enable_mempool,
        islock: live_cfg.enable_islock,
        chainlock: live_cfg.enable_chainlock,
        governance: live_cfg.enable_governance,
        network: live_cfg.enable_network,
        stream: live_cfg.enable_stream,
      },
      overlays: OverlayOptions {
        protx: overlay_cfg.enable_protx,
        chainlock: overlay_cfg.enable_chainlock,
      },
      api: ApiOptions {
        enabled: cfg.api.is_some(),
        bind: cfg.api.as_ref().map(|api| api.bind.clone()),
        cors_origin: cfg.api.as_ref().and_then(|api| api.cors_origin.clone()),
      },
      recent_mempool_txs: live_cfg.recent_mempool_txs,
      stream_client_buffer: live_cfg.stream_client_buffer,
      stream_heartbeat_secs: live_cfg.stream_heartbeat_secs,
      chainlock_start_block: live_cfg.chainlock_start_block,
      network_refresh_secs: cfg
        .core
        .rpc
        .as_ref()
        .map_or(DEFAULT_NETWORK_REFRESH_SECS, |rpc| rpc.network_refresh_secs),
      governance_refresh_secs: cfg
        .core
        .rpc
        .as_ref()
        .map_or(DEFAULT_GOVERNANCE_REFRESH_SECS, |rpc| {
          rpc.governance_refresh_secs
        }),
      chainlock_refresh_secs: cfg
        .core
        .rpc
        .as_ref()
        .map_or(DEFAULT_CHAINLOCK_REFRESH_SECS, |rpc| {
          rpc.chainlock_refresh_secs
        }),
      protx_refresh_secs: cfg
        .core
        .rpc
        .as_ref()
        .map_or(DEFAULT_PROTX_REFRESH_SECS, |rpc| rpc.protx_refresh_secs),
    })
  }

  /// Build the runtime config snapshot published to IPC and the console.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::Config` when one configured RPC auth mode cannot be
  /// resolved for snapshot diagnostics.
  pub fn config_snapshot(&self, cfg: &Config) -> Result<ConfigSnapshot, DaemonError> {
    Ok(ConfigSnapshot {
      sources: ConfigSourcesSnapshot {
        rest_url: cfg.core.url.clone(),
        postgres_enabled: true,
        postgres_write_pool_size: Some(cfg.postgres.write_pool_size),
        postgres_query_pool_size: Some(cfg.postgres.query_pool_size),
        postgres_admin_pool_size: Some(cfg.postgres.admin_pool_size),
        rpc_enabled: cfg.core.rpc.as_ref().is_some_and(|rpc| rpc.enabled),
        rpc_url: cfg.core.rpc.as_ref().map(|rpc| rpc.url.clone()),
        rpc_auth_type: cfg
          .core
          .rpc
          .as_ref()
          .filter(|rpc| rpc.enabled)
          .map(|rpc| rpc.auth_kind())
          .transpose()?
          .map(|auth| match auth {
            config::RpcAuth::Cookie => "cookie".to_owned(),
            config::RpcAuth::UserPass => "user_pass".to_owned(),
          }),
        zmq_enabled: cfg.core.zmq.as_ref().is_some_and(|zmq| zmq.enabled),
        zmq_connect: cfg
          .core
          .zmq
          .as_ref()
          .map(|zmq| zmq.connect.clone())
          .unwrap_or_default(),
        zmq_subscriptions: cfg
          .core
          .zmq
          .as_ref()
          .map(|zmq| {
            zmq
              .subscribe
              .iter()
              .map(|topic| match topic {
                config::ZmqTopic::RawTx => "rawtx".to_owned(),
                config::ZmqTopic::Sequence => "sequence".to_owned(),
                config::ZmqTopic::RawTxLock => "rawtxlock".to_owned(),
                config::ZmqTopic::RawChainLock => "rawchainlock".to_owned(),
                config::ZmqTopic::RawGovernanceObject => "rawgovernanceobject".to_owned(),
                config::ZmqTopic::RawGovernanceVote => "rawgovernancevote".to_owned(),
              })
              .collect()
          })
          .unwrap_or_default(),
        postgres_url: Some(redact_pg_url(&cfg.postgres.url)),
      },
      api: ConfigApiSnapshot {
        enabled: self.api.enabled,
        bind: self.api.bind.clone(),
        rest_concurrency: Some(cfg.core.rest_concurrency),
      },
      indices: ConfigIndexSnapshot {
        address: self.indices.address,
        validator: self.indices.validator,
        metrics: self.indices.metrics,
      },
      live: ConfigLiveSnapshot {
        mempool: self.live.mempool,
        islock: self.live.islock,
        chainlock: self.live.chainlock,
        chainlock_start_block: self.chainlock_start_block,
        governance: self.live.governance,
        network: self.live.network,
        stream: self.live.stream,
        recent_mempool_txs: Some(self.recent_mempool_txs),
        stream_client_buffer: Some(self.stream_client_buffer),
        stream_heartbeat_secs: Some(self.stream_heartbeat_secs),
      },
      overlays: ConfigOverlaySnapshot {
        protx: self.overlays.protx,
        chainlock: self.overlays.chainlock,
      },
      postgres: ConfigPostgresSnapshot {
        url_redacted: Some(redact_pg_url(&cfg.postgres.url)),
        write_pool_size: Some(cfg.postgres.write_pool_size),
        query_pool_size: Some(cfg.postgres.query_pool_size),
        heavy_query_pool_size: Some(cfg.postgres.heavy_query_pool_size),
        admin_pool_size: Some(cfg.postgres.admin_pool_size),
        slow_query_ms: Some(cfg.postgres.slow_query_ms),
      },
    })
  }
}

/// Redact credentials from a Postgres URL for diagnostic snapshots.
///
/// Splits the authority at the LAST `@` (a password may itself contain `@`)
/// and handles schemeless URLs too — `user:secret@host/db` without `://` must
/// not pass through verbatim.
fn redact_pg_url(url: &str) -> String {
  let (proto, rest) = match url.find("://") {
    Some(proto_end) => (&url[..proto_end + 3], &url[proto_end + 3..]),
    None => ("", url),
  };
  let authority_end = rest.find(['/', '?']).unwrap_or(rest.len());
  if let Some(at_pos) = rest[..authority_end].rfind('@') {
    return format!("{proto}***:***{}", &rest[at_pos..]);
  }
  url.to_owned()
}

impl Default for ApiOptions {
  fn default() -> Self {
    Self::from_config(&ApiConfig::default())
  }
}

impl ApiOptions {
  fn from_config(cfg: &ApiConfig) -> Self {
    Self {
      enabled: true,
      bind: Some(cfg.bind.clone()),
      cors_origin: cfg.cors_origin.clone(),
    }
  }
}

#[cfg(test)]
mod tests {
  use super::{Options, StartupMode};

  fn default_args() -> crate::init::Args {
    crate::init::Args {
      datadir: None,
      loglevel: 2,
      logconsole: false,
      docs_only: false,
    }
  }

  #[test]
  fn options_require_overlay_protx_rpc() {
    let cfg = crate::config::Config {
      overlays: Some(crate::config::OverlaysConfig {
        enable_protx: true,
        enable_chainlock: false,
      }),
      ..crate::config::Config::default()
    };
    let err = Options::resolve(&default_args(), &cfg).expect_err("rpc should be required");
    assert!(matches!(
      err,
      crate::error::DaemonError::Config(message)
      if message.contains("overlays.enable_protx requires core.rpc.enabled")
    ));
  }

  #[test]
  fn options_resolve_docs_only_mode_without_source_validation() {
    let args = crate::init::Args {
      docs_only: true,
      ..default_args()
    };
    let cfg = crate::config::Config {
      live: Some(crate::config::LiveConfig {
        enable_mempool: true,
        ..crate::config::LiveConfig::default()
      }),
      ..crate::config::Config::default()
    };
    let options = Options::resolve(&args, &cfg).expect("docs-only should bypass source validation");
    assert_eq!(options.startup_mode, StartupMode::DocsOnly);
  }
}
