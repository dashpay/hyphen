//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! PostgreSQL pool creation with Unix socket support.

#[cfg(feature = "bench")]
pub mod bench;
#[cfg(test)]
mod bench_coverage;
pub mod index;
pub mod init;
pub mod live;
pub mod progress;
pub mod segment;
pub mod snapshot;
pub mod sync;
pub mod types;
pub mod undo;
pub mod verbs;
pub use init as bootstrap;
pub use segment as partition;
pub use sync as query;

use sqlx::postgres::{PgConnectOptions, PgPoolOptions};
use sqlx::PgPool;

use crate::config::PostgresConfig;
use crate::error::DaemonError;

pub const DEFAULT_SLOW_QUERY_MS: u64 = 500;

pub const DEFAULT_WRITE_POOL_SIZE: usize = 20;
pub const DEFAULT_QUERY_POOL_SIZE: usize = 8;
pub const DEFAULT_HEAVY_QUERY_POOL_SIZE: usize = 4;
pub const DEFAULT_ADMIN_POOL_SIZE: usize = 4;

pub const DEFAULT_POSTGRES_URL: &str = "postgresql://hyphen:hyphen@127.0.0.1:5432/hyphen";

const WRITE_POOL_ACQUIRE_TIMEOUT_SECS: u64 = 10;
const QUERY_POOL_ACQUIRE_TIMEOUT_SECS: u64 = 2;
const ADMIN_POOL_ACQUIRE_TIMEOUT_SECS: u64 = 2;
const DAEMON_PG_APP_PREFIX: &str = "hyphend/";

/// Shared PostgreSQL pool type for daemon callers that are outside `crate::sql`.
pub type SqlPool = sqlx::PgPool;

/// Shared PostgreSQL transaction type for daemon callers that are outside `crate::sql`.
pub type SqlTx<'a> = sqlx::Transaction<'a, sqlx::Postgres>;

/// Shared PostgreSQL connection type for daemon callers that are outside `crate::sql`.
pub type SqlConnection = sqlx::PgConnection;

/// Named PostgreSQL pools split by daemon workload.
#[derive(Clone)]
pub struct PgPools {
  pub write: PgPool,
  pub query: PgPool,
  pub heavy_query: PgPool,
  pub admin: PgPool,
}

impl std::fmt::Debug for PgPools {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("PgPools").finish()
  }
}

/// Parse a PostgreSQL connection URL into `PgConnectOptions`.
///
/// sqlx does not support the libpq `?host=/path` query parameter
/// for Unix sockets. When the URL contains an empty host we extract
/// the socket directory from the query string and build the options
/// manually.
fn parse_connect_options(url: &str) -> Result<PgConnectOptions, DaemonError> {
  if let Ok(opts) = url.parse::<PgConnectOptions>() {
    return Ok(opts);
  }

  let query_str = url.split_once('?').map(|(_, q)| q).unwrap_or("");

  let socket_dir = query_str
    .split('&')
    .filter_map(|pair| {
      let (k, v) = pair.split_once('=')?;
      if k == "host" {
        Some(v)
      } else {
        None
      }
    })
    .next();

  let socket_dir = socket_dir.ok_or_else(|| {
    DaemonError::Config(format!(
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
    if let Some((k, v)) = pair.split_once('=') {
      if k == "port" {
        if let Ok(p) = v.parse::<u16>() {
          opts = opts.port(p);
        }
      }
    }
  }

  Ok(opts)
}

/// Redact password from a PostgreSQL URL for logging.
fn redact_url(url: &str) -> String {
  if let Some(scheme_end) = url.find("://") {
    let after = &url[scheme_end + 3..];
    if let Some(at) = after.find('@') {
      let userinfo = &after[..at];
      if let Some(colon) = userinfo.find(':') {
        let user = &userinfo[..colon];
        let rest = &after[at..];
        return format!("{}://{}:***{}", &url[..scheme_end], user, rest);
      }
    }
  }
  url.to_owned()
}

async fn create_named_pool(
  cfg: &PostgresConfig,
  pool_name: &str,
  pool_size: usize,
  acquire_timeout_secs: u64,
) -> Result<PgPool, DaemonError> {
  use sqlx::ConnectOptions as _;

  let safe_url = redact_url(&cfg.url);
  tracing::info!(
    url = %safe_url,
    pool = pool_name,
    pool_size,
    "creating postgres pool"
  );
  let slow_threshold = std::time::Duration::from_millis(cfg.slow_query_ms);
  let opts = parse_connect_options(&cfg.url)?
    .application_name(&format!("{DAEMON_PG_APP_PREFIX}{pool_name}"))
    .log_statements(tracing::log::LevelFilter::Debug)
    .log_slow_statements(tracing::log::LevelFilter::Warn, slow_threshold);
  let pool = PgPoolOptions::new()
    .max_connections(pool_size as u32)
    .acquire_timeout(std::time::Duration::from_secs(acquire_timeout_secs))
    .connect_with(opts)
    .await
    .map_err(|e| DaemonError::Pg(format!("failed to connect {pool_name} pool: {e}")))?;
  tracing::info!(pool = pool_name, "postgres pool created");
  Ok(pool)
}

/// Cancel active PostgreSQL statements owned by daemon pools.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the cancellation query.
pub async fn cancel_daemon_backends(admin_pool: &PgPool) -> Result<u64, DaemonError> {
  let cancelled: i64 = sqlx::query_scalar(
    "SELECT COUNT(*)
       FROM (
         SELECT pg_cancel_backend(pid)
           FROM pg_stat_activity
          WHERE pid <> pg_backend_pid()
            AND application_name LIKE $1
            AND state = 'active'
       ) AS cancelled",
  )
  .bind(format!("{DAEMON_PG_APP_PREFIX}%"))
  .fetch_one(admin_pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("cancel daemon postgres backends: {e}")))?;
  Ok(cancelled.max(0) as u64)
}

/// Create the daemon's PostgreSQL pools for writes, public queries, and admin reads.
///
/// # Errors
///
/// Returns `DaemonError::Config` when the URL cannot be parsed, or
/// `DaemonError::Pg` when any pool cannot connect.
pub async fn create_pools(cfg: &PostgresConfig) -> Result<PgPools, DaemonError> {
  Ok(PgPools {
    write: create_named_pool(
      cfg,
      "write",
      cfg.write_pool_size,
      WRITE_POOL_ACQUIRE_TIMEOUT_SECS,
    )
    .await?,
    query: create_named_pool(
      cfg,
      "query",
      cfg.query_pool_size,
      QUERY_POOL_ACQUIRE_TIMEOUT_SECS,
    )
    .await?,
    heavy_query: create_named_pool(
      cfg,
      "heavy_query",
      cfg.heavy_query_pool_size,
      QUERY_POOL_ACQUIRE_TIMEOUT_SECS,
    )
    .await?,
    admin: create_named_pool(
      cfg,
      "admin",
      cfg.admin_pool_size,
      ADMIN_POOL_ACQUIRE_TIMEOUT_SECS,
    )
    .await?,
  })
}

#[cfg(any(test, all(feature = "tests", feature = "daemon")))]
/// Create a single-connection PostgreSQL pool for daemon tests.
///
/// # Errors
///
/// Returns `sqlx::Error` when the pool cannot connect.
pub async fn connect_test_pool(url: &str) -> Result<SqlPool, sqlx::Error> {
  PgPoolOptions::new().max_connections(1).connect(url).await
}

#[cfg(test)]
mod tests {
  use std::fs;
  use std::path::{Path, PathBuf};

  const SQL_PATTERNS: &[&str] = &[
    "sqlx::query",
    "sqlx::query_as",
    "sqlx::query_scalar",
    "query!(",
    "query_as!(",
    "query_scalar!(",
  ];

  #[test]
  fn runtime_sql_isolation_stays_inside_sql_modules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&root, &mut files);

    let offenders: Vec<String> = files
      .into_iter()
      .filter(|path| file_contains_sql(path))
      .filter(|path| !allowed_sql_file(path))
      .map(|path| path.display().to_string())
      .collect();

    assert!(
      offenders.is_empty(),
      "runtime SQL must stay inside sql modules; test-only exception files must be reviewed explicitly: {}",
      offenders.join(", ")
    );
  }

  #[test]
  fn sqlx_references_stay_inside_sql_modules() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&root, &mut files);

    let offenders: Vec<String> = files
      .into_iter()
      .filter(|path| file_contains_sqlx_reference(path))
      .filter(|path| !allowed_sql_file(path))
      .map(|path| path.display().to_string())
      .collect();

    assert!(
      offenders.is_empty(),
      "sqlx references must stay inside sql modules: {}",
      offenders.join(", ")
    );
  }

  fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
      return;
    };
    for entry in entries.flatten() {
      let path = entry.path();
      if path.is_dir() {
        collect_rs_files(&path, out);
      } else if path.extension().is_some_and(|ext| ext == "rs") {
        out.push(path);
      }
    }
  }

  fn file_contains_sql(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
      return false;
    };
    SQL_PATTERNS
      .iter()
      .any(|pattern| contents.contains(pattern))
  }

  fn file_contains_sqlx_reference(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
      return false;
    };
    contents.contains("sqlx::") || contents.contains("use sqlx")
  }

  fn allowed_sql_file(path: &Path) -> bool {
    let rel = path
      .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
      .unwrap_or(path);

    if rel.starts_with("sql") {
      return true;
    }
    false
  }
}
