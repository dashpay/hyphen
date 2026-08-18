//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Connection harness for Postgres-fidelity tests. Reads
//! `HYPHEN_TEST_DATABASE_URL`, falling back to the default local trust-auth
//! server so DB-backed tests always run — and fail loudly when no database is
//! reachable — instead of silently skipping.

use crate::config::HyDbConfig;
use crate::HyphenDb;

/// The env var naming the test database.
pub const TEST_DB_URL_ENV: &str = "HYPHEN_TEST_DATABASE_URL";

/// Local trust-auth server used when [`TEST_DB_URL_ENV`] is unset.
pub const DEFAULT_TEST_URL: &str = "postgresql://postgres@127.0.0.1:5432/postgres";

/// Connect a [`HyphenDb`] to the test database.
///
/// # Errors
///
/// Propagates connection/migration failure.
pub async fn connect() -> crate::HyDbResult<Option<HyphenDb>> {
  let url = std::env::var(TEST_DB_URL_ENV).unwrap_or_else(|_| DEFAULT_TEST_URL.to_owned());
  let cfg = HyDbConfig {
    client: crate::HyDbClientCfg {
      url,
      // Keep tests light: a couple of connections per workload.
      write: crate::HyDbWriteCfg {
        size: 2,
        ..crate::HyDbWriteCfg::default()
      },
      query: crate::HyDbQueryCfg {
        size: 2,
        ..crate::HyDbQueryCfg::default()
      },
      heavy: crate::HyDbHeavyCfg {
        size: 1,
        ..crate::HyDbHeavyCfg::default()
      },
      admin: crate::HyDbAdminCfg {
        size: 1,
        ..crate::HyDbAdminCfg::default()
      },
      ..crate::HyDbClientCfg::default()
    },
    ..HyDbConfig::default()
  };
  HyphenDb::connect(cfg).await.map(Some)
}
