//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! `[db.ingest]`'s storage-parameter overrides against a live server.
//!
//! Two claims, and the first is the one that matters most: a daemon that was
//! never configured for tuning must end with the storage parameters the
//! migration committed. The numbers in `0001_schema.sql` were measured (A1, B3,
//! B5); the steady plan restates them so a store left on sync values is put
//! back.

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

/// A default configuration only restates the committed values (and clears the
/// one key the schema never declared), so the storage parameters on a
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
  assert_committed(&pool, "a default connect").await;

  // The steady plan is not empty: it restates the keys the sync profile can
  // write, so that leaving sync puts them back. Restating must change nothing.
  for change in storage_plan(&HyDbIngestCfg::default(), false) {
    pool.execute(change.sql.as_str()).await?;
  }
  assert_committed(&pool, "re-applying the default plan").await;

  pool.close().await;
  Ok(())
}

/// The committed storage parameters from `0001_schema.sql`, asserted on both
/// tuned tables.
async fn assert_committed(pool: &sqlx::PgPool, after: &str) {
  assert_eq!(
    reloptions(pool, "address_stats").await,
    vec!["fillfactor=80".to_owned()],
    "A1/B5's measured fillfactor must survive {after}"
  );
  assert_eq!(
    reloptions(pool, "utxo_set").await,
    vec![
      "autovacuum_vacuum_cost_delay=2".to_owned(),
      "autovacuum_vacuum_cost_limit=2000".to_owned(),
      "autovacuum_vacuum_scale_factor=0.02".to_owned(),
      "fillfactor=70".to_owned(),
    ],
    "B3's measured autovacuum tuning must survive {after}"
  );
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
      "autovacuum_analyze_scale_factor=0.5".to_owned(),
      "autovacuum_vacuum_cost_delay=2".to_owned(),
      "autovacuum_vacuum_cost_limit=2000".to_owned(),
      "autovacuum_vacuum_scale_factor=0.5".to_owned(),
      "fillfactor=70".to_owned(),
    ],
    "the sync profile relaxes the vacuum and analyze factors together, \
     leaving the cost pacing and the fillfactor alone"
  );
  assert_eq!(
    reloptions(&pool, "address_stats").await,
    vec!["fillfactor=80".to_owned()],
    "a table the schema gives no autovacuum parameters must not acquire any"
  );

  // Leaving sync must put the committed values back, not leave the relaxed
  // factors on disk.
  for change in storage_plan(&HyDbIngestCfg::default(), false) {
    pool.execute(change.sql.as_str()).await?;
  }
  assert_committed(&pool, "leaving the sync profile").await;

  pool.close().await;
  Ok(())
}
