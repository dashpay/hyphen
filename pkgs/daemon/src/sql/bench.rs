//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared SQL benchmark setup and registration helpers.

pub mod index;
pub mod live;
pub mod spent;
pub mod validation;
pub mod write;

use std::time::{SystemTime, UNIX_EPOCH};

use sqlx::postgres::PgPoolOptions;
use sqlx::Connection;
use sqlx::Executor;

use crate::error::DaemonError;
use hyphend::bench::BenchDataset;

const BENCH_SCHEMA_VERSION: i32 = 2;
const BENCH_CONFIG_KEY_DATASET: &str = "dataset";
const SCHEMA_SQL: &str = include_str!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/../../schema/daemon_pg.sql"
));
const UNPRIMED_MESSAGE: &str =
  "Error: Database not primed for benchmark, did you forget to run --setup?";

/// Build one stable synthetic 32-byte identifier for benchmark seed data.
pub fn synthetic_hash(domain: u8, id: i64) -> Vec<u8> {
  let mut value = [0_u8; 32];
  value[0] = domain;
  value[1..9].copy_from_slice(&id.to_be_bytes());
  for (index, byte) in value[9..].iter_mut().enumerate() {
    *byte = domain.wrapping_add(index as u8).wrapping_mul(17);
  }
  value.to_vec()
}

/// Return the stable benchmark schema name for one dataset.
pub fn schema_name(dataset: BenchDataset) -> String {
  format!("bench_{}", dataset.as_str())
}

/// Prime the target database schema for timed benchmark runs.
///
/// # Errors
///
/// Returns an error when PostgreSQL connection, schema creation, or seed setup
/// fails.
pub async fn prime_database(database_url: &str, dataset: BenchDataset) -> Result<(), DaemonError> {
  let schema_name = schema_name(dataset);
  let mut admin = sqlx::PgConnection::connect(database_url)
    .await
    .map_err(|error| DaemonError::Pg(format!("connect benchmark admin: {error}")))?;
  admin
    .execute(format!("DROP SCHEMA IF EXISTS {schema_name} CASCADE").as_str())
    .await
    .map_err(|error| DaemonError::Pg(format!("drop benchmark schema {schema_name}: {error}")))?;
  admin
    .execute(format!("CREATE SCHEMA {schema_name}").as_str())
    .await
    .map_err(|error| DaemonError::Pg(format!("create benchmark schema {schema_name}: {error}")))?;
  install_config_store(&mut admin).await?;
  drop(admin);

  let pool = connect_pool(database_url, &schema_name, 4).await?;
  sqlx::raw_sql(SCHEMA_SQL)
    .execute(&pool)
    .await
    .map_err(|error| DaemonError::Pg(format!("apply benchmark schema: {error}")))?;
  crate::sql::segment::ensure_core_history_partitions(&pool).await?;
  install_manifest(&pool, dataset).await?;
  spent::seed(&pool, dataset).await?;
  index::seed(&pool, dataset).await?;
  live::seed(&pool, dataset).await?;
  write::seed(&pool, dataset).await?;
  sqlx::query("ANALYZE")
    .execute(&pool)
    .await
    .map_err(|error| DaemonError::Pg(format!("analyze benchmark schema: {error}")))?;
  let mut admin = sqlx::PgConnection::connect(database_url)
    .await
    .map_err(|error| DaemonError::Pg(format!("reconnect benchmark admin: {error}")))?;
  write_dataset_config(&mut admin, dataset).await?;
  Ok(())
}

/// Resolve the benchmark dataset to use for this run.
///
/// # Errors
///
/// Returns the exact requested unprimed error when no persisted setup exists
/// and no dataset override was supplied.
pub async fn resolve_dataset(
  database_url: &str,
  explicit: Option<BenchDataset>,
) -> Result<BenchDataset, DaemonError> {
  if let Some(dataset) = explicit {
    return Ok(dataset);
  }

  let mut admin = sqlx::PgConnection::connect(database_url)
    .await
    .map_err(|_| DaemonError::Config(UNPRIMED_MESSAGE.to_owned()))?;
  let value = read_dataset_config(&mut admin).await?;
  BenchDataset::parse(value.as_str()).map_err(|_| DaemonError::Config(UNPRIMED_MESSAGE.to_owned()))
}

/// Open the primed benchmark pool for one dataset.
///
/// # Errors
///
/// Returns the exact requested unprimed error when the benchmark schema has not
/// been set up already.
pub async fn open_primed_pool(
  database_url: &str,
  dataset: BenchDataset,
) -> Result<crate::sql::SqlPool, DaemonError> {
  let schema_name = schema_name(dataset);
  let pool = connect_pool(database_url, &schema_name, 8).await?;
  verify_manifest(&pool, dataset).await?;
  Ok(pool)
}

fn setup_epoch() -> i64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs() as i64
}

async fn connect_pool(
  database_url: &str,
  schema_name: &str,
  max_connections: u32,
) -> Result<crate::sql::SqlPool, DaemonError> {
  let options = super::parse_connect_options(database_url)?;
  let schema_name = schema_name.to_owned();
  PgPoolOptions::new()
    .max_connections(max_connections)
    .after_connect(move |conn, _meta| {
      let schema_name = schema_name.clone();
      Box::pin(async move {
        conn
          .execute(format!("SET search_path TO {schema_name}").as_str())
          .await?;
        Ok(())
      })
    })
    .connect_with(options)
    .await
    .map_err(|error| DaemonError::Pg(format!("connect benchmark pool: {error}")))
}

async fn install_manifest(
  pool: &crate::sql::SqlPool,
  dataset: BenchDataset,
) -> Result<(), DaemonError> {
  sqlx::query(
    "CREATE TABLE IF NOT EXISTS bench_manifest (
       singleton_key BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton_key),
       schema_version INTEGER NOT NULL,
       dataset TEXT NOT NULL,
       seeded_at BIGINT NOT NULL
     )",
  )
  .execute(pool)
  .await
  .map_err(|error| DaemonError::Pg(format!("create benchmark manifest: {error}")))?;

  sqlx::query(
    "INSERT INTO bench_manifest (singleton_key, schema_version, dataset, seeded_at)
     VALUES (TRUE, $1, $2, $3)
     ON CONFLICT (singleton_key) DO UPDATE SET
       schema_version = EXCLUDED.schema_version,
       dataset = EXCLUDED.dataset,
       seeded_at = EXCLUDED.seeded_at",
  )
  .bind(BENCH_SCHEMA_VERSION)
  .bind(dataset.as_str())
  .bind(setup_epoch())
  .execute(pool)
  .await
  .map_err(|error| DaemonError::Pg(format!("write benchmark manifest: {error}")))?;
  Ok(())
}

async fn install_config_store(connection: &mut sqlx::PgConnection) -> Result<(), DaemonError> {
  sqlx::query(
    "CREATE TABLE IF NOT EXISTS public.bench_config (
       config_key TEXT PRIMARY KEY,
       config_value TEXT NOT NULL,
       updated_at BIGINT NOT NULL
     )",
  )
  .execute(connection)
  .await
  .map_err(|error| DaemonError::Pg(format!("create benchmark config store: {error}")))?;
  Ok(())
}

async fn write_dataset_config(
  connection: &mut sqlx::PgConnection,
  dataset: BenchDataset,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO public.bench_config (config_key, config_value, updated_at)
     VALUES ($1, $2, $3)
     ON CONFLICT (config_key) DO UPDATE SET
       config_value = EXCLUDED.config_value,
       updated_at = EXCLUDED.updated_at",
  )
  .bind(BENCH_CONFIG_KEY_DATASET)
  .bind(dataset.as_str())
  .bind(setup_epoch())
  .execute(connection)
  .await
  .map_err(|error| DaemonError::Pg(format!("write benchmark dataset config: {error}")))?;
  Ok(())
}

async fn read_dataset_config(connection: &mut sqlx::PgConnection) -> Result<String, DaemonError> {
  let row = sqlx::query_as::<_, (String,)>(
    "SELECT config_value FROM public.bench_config WHERE config_key = $1",
  )
  .bind(BENCH_CONFIG_KEY_DATASET)
  .fetch_optional(connection)
  .await
  .map_err(|_| DaemonError::Config(UNPRIMED_MESSAGE.to_owned()))?;

  let Some((value,)) = row else {
    return Err(DaemonError::Config(UNPRIMED_MESSAGE.to_owned()));
  };
  Ok(value)
}

async fn verify_manifest(
  pool: &crate::sql::SqlPool,
  dataset: BenchDataset,
) -> Result<(), DaemonError> {
  let row = sqlx::query_as::<_, (i32, String)>(
    "SELECT schema_version, dataset FROM bench_manifest WHERE singleton_key = TRUE",
  )
  .fetch_optional(pool)
  .await
  .map_err(|_| DaemonError::Config(UNPRIMED_MESSAGE.to_owned()))?;

  let Some((schema_version, seeded_dataset)) = row else {
    return Err(DaemonError::Config(UNPRIMED_MESSAGE.to_owned()));
  };
  if schema_version != BENCH_SCHEMA_VERSION || seeded_dataset != dataset.as_str() {
    return Err(DaemonError::Config(UNPRIMED_MESSAGE.to_owned()));
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use hyphend::bench::BenchDataset;

  #[test]
  fn schema_name_is_stable_per_dataset() {
    assert_eq!(super::schema_name(BenchDataset::Small), "bench_small");
    assert_eq!(super::schema_name(BenchDataset::Default), "bench_default");
    assert_eq!(super::schema_name(BenchDataset::Large), "bench_large");
  }

  #[test]
  fn dataset_parse_accepts_persisted_values() {
    assert_eq!(
      BenchDataset::parse("small").expect("small dataset should parse"),
      BenchDataset::Small
    );
    assert_eq!(
      BenchDataset::parse("default").expect("default dataset should parse"),
      BenchDataset::Default
    );
    assert_eq!(
      BenchDataset::parse("large").expect("large dataset should parse"),
      BenchDataset::Large
    );
  }
}
