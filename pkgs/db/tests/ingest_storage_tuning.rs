//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! `[db.ingest]`'s storage-parameter overrides against a live server.
//!
//! Two claims, and the first is the one that matters most: a daemon that was
//! never configured for tuning must leave the schema exactly as the migration
//! created it. The numbers in `0001_schema.sql` were measured (A1, B3, B5) and
//! a configuration surface that quietly re-states them is a configuration
//! surface that can quietly get them wrong.

mod common;

use common::TempDb;
use hyphen_db::tuning::{storage_plan, HyDbIngestCfg, HyDbTuningProfile};
use sqlx::{Executor as _, Row as _};

type TestResult = Result<(), Box<dyn std::error::Error>>;

async fn temp_db() -> Result<TempDb, Box<dyn std::error::Error>> {
  TempDb::create().await
}

/// `reloptions` for a table, as a sorted list of `key=value` strings.
async fn reloptions(pool: &sqlx::PgPool, table: &str) -> Vec<String> {
  let row = sqlx::query(
    "SELECT COALESCE(reloptions, '{}')::text[] AS opts FROM pg_class WHERE relname = $1",
  )
  .bind(table)
  .fetch_one(pool)
  .await
  .ok();
  let mut opts: Vec<String> = row
    .and_then(|row| row.try_get::<Vec<String>, _>("opts").ok())
    .unwrap_or_default();
  opts.sort();
  opts
}

/// A default configuration emits no DDL, so the storage parameters on a
/// freshly-connected database are precisely the committed ones.
#[tokio::test]
async fn default_config_leaves_the_committed_storage_parameters_untouched() -> TestResult {
  let db = temp_db().await?;
  let result = untouched(&db).await;
  db.drop_now().await;
  result
}

async fn untouched(db: &TempDb) -> TestResult {
  let _facade = db.open().await?;
  let pool = db.seed_pool().await?;

  assert_eq!(
    reloptions(&pool, "address_stats").await,
    vec!["fillfactor=80".to_owned()],
    "A1/B5's measured fillfactor must survive a default connect"
  );
  assert_eq!(
    reloptions(&pool, "utxo_set").await,
    vec![
      "autovacuum_vacuum_cost_delay=2".to_owned(),
      "autovacuum_vacuum_cost_limit=2000".to_owned(),
      "autovacuum_vacuum_scale_factor=0.02".to_owned(),
      "fillfactor=70".to_owned(),
    ],
    "B3's measured autovacuum tuning must survive a default connect"
  );
  assert!(
    storage_plan(&HyDbIngestCfg::default(), false).is_empty(),
    "and the plan that produced that must be empty, not merely idempotent"
  );

  pool.close().await;
  Ok(())
}

/// An override is real DDL that the server accepts and that changes exactly
/// the parameters named — everything else stays at the committed value.
#[tokio::test]
async fn an_override_changes_only_the_parameters_it_names() -> TestResult {
  let db = temp_db().await?;
  let result = override_lands(&db).await;
  db.drop_now().await;
  result
}

async fn override_lands(db: &TempDb) -> TestResult {
  let _facade = db.open().await?;
  let pool = db.seed_pool().await?;

  // The sync profile, as `auto` would resolve it once A3 reports stepped mode.
  let cfg = HyDbIngestCfg {
    tuning_profile: HyDbTuningProfile::Sync,
    ..HyDbIngestCfg::default()
  };
  for change in storage_plan(&cfg, false) {
    pool.execute(change.sql.as_str()).await?;
  }

  assert_eq!(
    reloptions(&pool, "utxo_set").await,
    vec![
      "autovacuum_analyze_scale_factor=0.05".to_owned(),
      "autovacuum_vacuum_cost_delay=2".to_owned(),
      "autovacuum_vacuum_cost_limit=2000".to_owned(),
      "autovacuum_vacuum_scale_factor=0.2".to_owned(),
      "fillfactor=70".to_owned(),
    ],
    "the sync profile relaxes the vacuum factor and keeps analyze running, \
     leaving the cost pacing and the fillfactor alone"
  );
  assert_eq!(
    reloptions(&pool, "address_stats").await,
    vec!["fillfactor=80".to_owned()],
    "a table the schema gives no autovacuum parameters must not acquire any"
  );

  pool.close().await;
  Ok(())
}
