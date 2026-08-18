//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! PostgreSQL pool creation, split by workload. Private to the crate: the four
//! pools never leave `hyphen-db`.

use core::fmt;
use std::time::Duration;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

use crate::config::HyDbConfig;
use crate::error::HyDbError;
use crate::util::redact_url;

const WRITE_POOL_ACQUIRE_TIMEOUT_SECS: u64 = 10;
const QUERY_POOL_ACQUIRE_TIMEOUT_SECS: u64 = 2;
const ADMIN_POOL_ACQUIRE_TIMEOUT_SECS: u64 = 2;
const APP_PREFIX: &str = "hyphend/";

/// The daemon's Postgres pools, split by workload so a slow analytical query
/// can never starve the ingest writer.
#[derive(Clone)]
pub(crate) struct HyDbPools {
  /// Single-writer ingest path.
  pub write: PgPool,
  /// Public, latency-sensitive reads.
  pub query: PgPool,
  /// Heavy analytical reads (rich list, ranges).
  pub heavy_query: PgPool,
  /// Maintenance and diagnostics.
  pub admin: PgPool,
}

impl HyDbPools {
  pub(crate) async fn connect(cfg: &HyDbConfig) -> Result<Self, HyDbError> {
    Ok(Self {
      write: named_pool(
        cfg,
        "write",
        cfg.write_pool_size,
        WRITE_POOL_ACQUIRE_TIMEOUT_SECS,
      )
      .await?,
      query: named_pool(
        cfg,
        "query",
        cfg.query_pool_size,
        QUERY_POOL_ACQUIRE_TIMEOUT_SECS,
      )
      .await?,
      heavy_query: named_pool(
        cfg,
        "heavy_query",
        cfg.heavy_query_pool_size,
        QUERY_POOL_ACQUIRE_TIMEOUT_SECS,
      )
      .await?,
      admin: named_pool(
        cfg,
        "admin",
        cfg.admin_pool_size,
        ADMIN_POOL_ACQUIRE_TIMEOUT_SECS,
      )
      .await?,
    })
  }
}

impl fmt::Debug for HyDbPools {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("HyDbPools").finish()
  }
}

async fn named_pool(
  cfg: &HyDbConfig,
  pool_name: &str,
  pool_size: usize,
  acquire_timeout_secs: u64,
) -> Result<PgPool, HyDbError> {
  use sqlx::ConnectOptions as _;

  tracing::info!(
    url = %redact_url(&cfg.url),
    pool = pool_name,
    pool_size,
    "creating postgres pool"
  );
  let slow_threshold = Duration::from_millis(cfg.slow_query_ms);
  let opts = parse_connect_options(&cfg.url)?
    .application_name(&format!("{APP_PREFIX}{pool_name}"))
    .log_statements(tracing::log::LevelFilter::Debug)
    .log_slow_statements(tracing::log::LevelFilter::Warn, slow_threshold);
  let pool = PgPoolOptions::new()
    .max_connections(pool_size as u32)
    .acquire_timeout(Duration::from_secs(acquire_timeout_secs))
    .connect_with(opts)
    .await
    .map_err(|e| HyDbError::pg(&format!("failed to connect {pool_name} pool"), e))?;
  tracing::info!(pool = pool_name, "postgres pool created");
  Ok(pool)
}

/// Parse a connection URL into `PgConnectOptions`, handling the libpq
/// `?host=/path` Unix-socket form that sqlx does not accept natively.
fn parse_connect_options(url: &str) -> Result<PgConnectOptions, HyDbError> {
  if let Ok(opts) = url.parse::<PgConnectOptions>() {
    return Ok(opts);
  }

  let query_str = url.split_once('?').map(|(_, q)| q).unwrap_or("");
  let socket_dir = query_str
    .split('&')
    .filter_map(|pair| {
      let (k, v) = pair.split_once('=')?;
      (k == "host").then_some(v)
    })
    .next()
    .ok_or_else(|| {
      HyDbError::Config(format!(
        "postgres url has empty host and no ?host= query parameter: {url}"
      ))
    })?;

  let after_scheme = url.split_once("://").map(|(_, rest)| rest).unwrap_or(url);
  let authority_path = after_scheme
    .split_once('?')
    .map(|(a, _)| a)
    .unwrap_or(after_scheme);
  let (userinfo, host_path) = authority_path
    .split_once('@')
    .unwrap_or(("", authority_path));
  let (username, password) = userinfo.split_once(':').unwrap_or((userinfo, ""));
  let db = host_path.trim_start_matches('/');

  let mut opts = PgConnectOptions::new().socket(socket_dir);
  if !db.is_empty() {
    opts = opts.database(db);
  }
  if !username.is_empty() {
    opts = opts.username(username);
  }
  if !password.is_empty() {
    opts = opts.password(password);
  }
  for pair in query_str.split('&') {
    if let Some(("port", v)) = pair.split_once('=') {
      if let Ok(p) = v.parse::<u16>() {
        opts = opts.port(p);
      }
    }
  }
  Ok(opts)
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn parses_tcp_url() {
    assert!(parse_connect_options("postgresql://u:p@127.0.0.1:5432/db").is_ok());
  }

  #[test]
  fn parses_unix_socket_url() {
    let opts = parse_connect_options("postgresql://u:p@/db?host=/var/run/postgresql&port=5433");
    assert!(opts.is_ok());
  }

  #[test]
  fn empty_host_without_socket_errors() {
    assert!(parse_connect_options("postgresql://u:p@/db").is_err());
  }
}
