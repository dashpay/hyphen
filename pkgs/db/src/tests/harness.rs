//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Connection harness for Postgres-fidelity tests. The target database is
//! named by `HYPHEN_TEST_DATABASE_URL` and nothing else: connecting applies
//! migrations, so there is deliberately no default and no way to aim it at a
//! server's shared databases.
//!
//! It previously defaulted to `postgresql://postgres@127.0.0.1:5432/postgres`
//! — the maintenance database of whatever server was configured. That default
//! never fired, because the function has no callers, but the first caller to
//! appear would have migrated `postgres` itself. Both halves of that hazard
//! are now closed: the fallback is gone, and a URL naming a reserved database
//! is refused before a connection is opened.

use crate::config::HyDbConfig;
use crate::error::HyDbError;
use crate::HyphenDb;

/// The env var naming the test database.
pub const TEST_DB_URL_ENV: &str = "HYPHEN_TEST_DATABASE_URL";

/// Databases every Postgres server owns. A test may not migrate any of them:
/// `postgres` is the maintenance database (and the one an admin connection
/// lands in by default), and the templates seed every future `CREATE DATABASE`.
pub const RESERVED_DATABASES: [&str; 3] = ["postgres", "template0", "template1"];

/// The database name a connection URL targets, if it names one.
///
/// Path-only parsing on purpose: the URL may carry credentials or query
/// parameters, and only the last path segment (minus any `?`/`#` tail) selects
/// the database.
fn database_name(url: &str) -> Option<&str> {
  let after_scheme = url.split_once("://")?.1;
  let path = after_scheme.split_once('/')?.1;
  let name = path.split_once(['?', '#']).map_or(path, |(head, _)| head);
  (!name.is_empty()).then_some(name)
}

/// Whether `url` targets a database a test is allowed to migrate.
///
/// # Errors
///
/// Returns [`HyDbError::Config`] when the URL names no database, or names one
/// of [`RESERVED_DATABASES`].
fn check_target(url: &str) -> Result<(), HyDbError> {
  let Some(name) = database_name(url) else {
    return Err(HyDbError::Config(format!(
      "{TEST_DB_URL_ENV} must name a database (got {url:?}); connecting applies \
       migrations, so the server's default database is not an acceptable target"
    )));
  };
  if RESERVED_DATABASES.contains(&name) {
    return Err(HyDbError::Config(format!(
      "{TEST_DB_URL_ENV} targets the reserved database {name:?}; connecting \
       applies migrations, so point it at a throwaway database instead"
    )));
  }
  Ok(())
}

/// Connect a [`HyphenDb`] to the database named by [`TEST_DB_URL_ENV`].
///
/// # Errors
///
/// Returns [`HyDbError::Config`] when the env var is unset, or names a
/// database no test may migrate (see [`check_target`]). Otherwise propagates
/// connection/migration failure.
pub async fn connect() -> crate::HyDbResult<HyphenDb> {
  let url = std::env::var(TEST_DB_URL_ENV).map_err(|_| {
    HyDbError::Config(format!(
      "{TEST_DB_URL_ENV} must be set to the throwaway database this harness \
       may migrate; there is no default"
    ))
  })?;
  check_target(&url)?;
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
  HyphenDb::connect(cfg).await
}

#[cfg(test)]
mod tests {
  use super::{check_target, database_name};

  #[test]
  fn reserved_targets_are_refused() {
    // The retired default, and the templates that would seed every database
    // created afterwards.
    for url in [
      "postgresql://postgres@127.0.0.1:5432/postgres",
      "postgresql://postgres@127.0.0.1:5432/template0",
      "postgresql://postgres:pw@host:5432/template1?sslmode=disable",
    ] {
      assert!(check_target(url).is_err(), "{url} must be refused");
    }
  }

  #[test]
  fn a_url_without_a_database_is_refused() {
    // No path segment: libpq would fall back to the role name, which for the
    // trust-auth `postgres` superuser is the maintenance database again.
    for url in [
      "postgresql://postgres@127.0.0.1:5432",
      "postgresql://postgres@127.0.0.1:5432/",
      "postgresql://postgres@127.0.0.1:5432/?sslmode=disable",
    ] {
      assert!(check_target(url).is_err(), "{url} must be refused");
    }
  }

  #[test]
  fn a_throwaway_database_is_accepted() {
    for url in [
      "postgresql://postgres@127.0.0.1:5432/hyphen_test_1",
      "postgres://postgres@127.0.0.1:5432/hyphen_test_1?application_name=x",
      // `postgres` as the ROLE, not the database, is fine.
      "postgresql://postgres:pw@host/postgres_scratch",
    ] {
      assert!(check_target(url).is_ok(), "{url} must be accepted");
    }
    assert_eq!(
      database_name("postgresql://postgres@127.0.0.1:5432/hyphen_test_1"),
      Some("hyphen_test_1")
    );
  }
}
