//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared integration-test harness: an ephemeral database per test, so runs are
//! isolated and parallel-safe. `HYPHEN_TEST_DATABASE_URL` overrides the admin
//! connection; when unset the suite targets the default local server — tests
//! ALWAYS run and fail loudly if no database is reachable, rather than
//! silently returning green (which is exactly how the original search bug
//! survived).

#![allow(dead_code)] // each test binary uses a different subset

use std::sync::atomic::{AtomicU32, Ordering};

use hyphen_db::{HyDbConfig, HyphenDb};
use sqlx::PgPool;

pub const ENV: &str = "HYPHEN_TEST_DATABASE_URL";

/// Local trust-auth server used when `HYPHEN_TEST_DATABASE_URL` is unset.
pub const DEFAULT_TEST_URL: &str = "postgresql://postgres@127.0.0.1:5432/postgres";

static COUNTER: AtomicU32 = AtomicU32::new(0);

/// Split a connection URL into (prefix-through-last-slash, dbname).
fn split_url(url: &str) -> Option<(&str, &str)> {
  let slash = url.rfind('/')?;
  if slash < 8 {
    return None; // guard against the scheme's `//`
  }
  Some((&url[..=slash], &url[slash + 1..]))
}

/// An ephemeral database that force-drops itself on [`TempDb::drop_now`].
pub struct TempDb {
  admin: PgPool,
  name: String,
  url: String,
}

impl TempDb {
  /// Provision a fresh database on the configured (or default local) server.
  /// Returns `Some` on success and errors — loudly failing the test — when no
  /// server is reachable. The `Option` shape is kept for call-site stability.
  pub async fn create() -> Result<Option<Self>, Box<dyn std::error::Error>> {
    let base = std::env::var(ENV).unwrap_or_else(|_| DEFAULT_TEST_URL.to_owned());
    let Some((prefix, _)) = split_url(&base) else {
      return Err(format!("cannot parse {ENV}: {base}").into());
    };
    let name = format!(
      "hyphen_test_{}_{}",
      std::process::id(),
      COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    let admin = PgPool::connect(&base).await?;
    sqlx::query(&format!("CREATE DATABASE {name}"))
      .execute(&admin)
      .await?;
    Ok(Some(Self {
      admin,
      url: format!("{prefix}{name}"),
      name,
    }))
  }

  /// Connect a `HyphenDb` (applies migrations) to the ephemeral database.
  pub async fn open(&self) -> Result<HyphenDb, Box<dyn std::error::Error>> {
    let cfg = HyDbConfig {
      url: self.url.clone(),
      write_pool_size: 2,
      query_pool_size: 2,
      heavy_query_pool_size: 1,
      admin_pool_size: 1,
      ..HyDbConfig::default()
    };
    Ok(HyphenDb::connect(cfg).await?)
  }

  /// A raw pool to the ephemeral database, for seeding.
  pub async fn seed_pool(&self) -> Result<PgPool, Box<dyn std::error::Error>> {
    Ok(PgPool::connect(&self.url).await?)
  }

  /// Drop the database, terminating any lingering connections.
  pub async fn drop_now(&self) {
    let _ = sqlx::query(&format!(
      "DROP DATABASE IF EXISTS {} WITH (FORCE)",
      self.name
    ))
    .execute(&self.admin)
    .await;
  }
}
