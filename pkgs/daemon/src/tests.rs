//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared test support exposed behind the `tests` feature gate.

use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use rocksdb::DB;

use crate::rest::{AppState, IndexCapabilities, LiveCapabilities, OverlayCapabilities};
use crate::sql::SqlPool;

static TEST_DB_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Open one single-connection PostgreSQL pool when the test URL is configured.
///
/// Returns `Ok(None)` when `HYPHEN_TEST_DATABASE_URL` is unset.
pub async fn pg_pool() -> Result<Option<SqlPool>, Box<dyn std::error::Error>> {
  let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
    return Ok(None);
  };
  let database_url = database_url
    .into_string()
    .map_err(|_| "invalid database url")?;
  let pool = crate::sql::connect_test_pool(&database_url).await?;
  Ok(Some(pool))
}

/// Build one minimal REST app state for handler tests.
pub fn app_state() -> AppState {
  let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_nanos())
    .unwrap_or_default();
  let counter = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
  let db_path = std::env::temp_dir().join(format!("hyphen-rest-test-{timestamp}-{counter}"));
  let db = DB::open_default(&db_path).expect("rocksdb should open");
  let _ = fs::remove_dir_all(&db_path);

  AppState {
    pool: SqlPool::connect_lazy("postgres://localhost/unused").expect("lazy pool should build"),
    address_pool: SqlPool::connect_lazy("postgres://localhost/unused")
      .expect("lazy pool should build"),
    db: Arc::new(db),
    secondary_stores: None,
    readiness_cache: Arc::new(crate::readiness::ReadinessCache::new(false, 1)),
    spent_cache: Arc::new(crate::index::spent::cache::SpentQueryCache::disabled()),
    address_hot_cache_threshold: crate::config::DEFAULT_ADDRESS_CACHE_HOT_THRESHOLD,
    address_summary_catchup_lag: crate::config::DEFAULT_ADDRESS_SUMMARY_CATCHUP_LAG,
    p2pkh_version: 76,
    p2sh_version: 16,
    indices: IndexCapabilities {
      address: false,
      validator: false,
      metrics: false,
    },
    live: LiveCapabilities {
      mempool: false,
      islock: false,
      network: false,
      governance: false,
      stream: false,
    },
    overlays: OverlayCapabilities {
      protx: false,
      chainlock: false,
    },
    recent_mempool_txs: 0,
    stream_hub: None,
    stream_heartbeat_secs: 1,
  }
}

/// Build one REST app state with the provided pools and capability flags.
pub fn app_state_with_pool(
  pool: SqlPool,
  live: LiveCapabilities,
  overlays: OverlayCapabilities,
  recent_mempool_txs: usize,
) -> AppState {
  let mut state = app_state();
  state.address_pool = pool.clone();
  state.pool = pool;
  state.live = live;
  state.overlays = overlays;
  state.recent_mempool_txs = recent_mempool_txs;
  state
}

/// Open one temporary RocksDB block store for external daemon tests.
pub fn open_temp_db() -> Result<DB, crate::error::DaemonError> {
  let path = unique_temp_dir("hyphen-tests-rocksdb");
  crate::db::open(&path, None)
}

/// Allocate one unique temporary datadir path for daemon-side tests.
pub fn temp_datadir(prefix: &str) -> PathBuf {
  unique_temp_dir(prefix)
}

/// Build one minimal valid coinbase block fixture for daemon tests.
pub fn minimal_block_bytes(nonce: u32) -> Vec<u8> {
  let mut buf = Vec::new();
  buf.extend_from_slice(&4_i32.to_le_bytes());
  buf.extend_from_slice(&[0xAA; 32]);
  buf.extend_from_slice(&[0xBB; 32]);
  buf.extend_from_slice(&1_700_000_000_u32.to_le_bytes());
  buf.extend_from_slice(&0x1A0F_FFFF_u32.to_le_bytes());
  buf.extend_from_slice(&nonce.to_le_bytes());
  buf.push(0x01);
  buf.extend_from_slice(&1_i32.to_le_bytes());
  buf.push(0x01);
  buf.extend_from_slice(&[0x00; 32]);
  buf.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
  buf.push(0x04);
  buf.extend_from_slice(&[nonce as u8, 0x02, 0x03, 0x04]);
  buf.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
  buf.push(0x01);
  buf.extend_from_slice(&50_0000_0000_i64.to_le_bytes());
  buf.push(0x00);
  buf.extend_from_slice(&0_u32.to_le_bytes());
  buf
}

/// Populate one inclusive canonical height range in a temporary RocksDB block store.
pub fn populate_block_height_range(
  db: &DB,
  start: u32,
  end: u32,
) -> Result<(), crate::error::DaemonError> {
  let blocks_cf = db
    .cf_handle(crate::db::CF_BLOCKS)
    .ok_or_else(|| crate::error::DaemonError::Db("blocks CF missing".to_owned()))?;
  let heights_cf = db
    .cf_handle(crate::db::CF_HEIGHTS)
    .ok_or_else(|| crate::error::DaemonError::Db("heights CF missing".to_owned()))?;
  let meta_cf = db
    .cf_handle(crate::db::CF_META)
    .ok_or_else(|| crate::error::DaemonError::Db("meta CF missing".to_owned()))?;
  let mut batch = rocksdb::WriteBatch::default();

  for height in start..=end {
    let hash = [height as u8; 32];
    batch.put_cf(heights_cf, height.to_be_bytes(), hash);
    batch.put_cf(blocks_cf, hash, hash);
    crate::db::put_tip_hash(&mut batch, meta_cf, &hash);
    crate::db::put_tip_height(&mut batch, meta_cf, height);
  }

  db.write(batch)
    .map_err(|e| crate::error::DaemonError::Db(format!("populate test db: {e}")))
}

/// Write one canonical tip row into a temporary RocksDB block store.
pub fn write_tip(db: &DB, height: u32, hash: [u8; 32]) -> Result<(), crate::error::DaemonError> {
  let blocks_cf = db
    .cf_handle(crate::db::CF_BLOCKS)
    .ok_or_else(|| crate::error::DaemonError::Db("blocks CF missing".to_owned()))?;
  let heights_cf = db
    .cf_handle(crate::db::CF_HEIGHTS)
    .ok_or_else(|| crate::error::DaemonError::Db("heights CF missing".to_owned()))?;
  let meta_cf = db
    .cf_handle(crate::db::CF_META)
    .ok_or_else(|| crate::error::DaemonError::Db("meta CF missing".to_owned()))?;
  let mut batch = rocksdb::WriteBatch::default();
  batch.put_cf(heights_cf, height.to_be_bytes(), hash);
  batch.put_cf(blocks_cf, hash, hash);
  crate::db::put_tip_hash(&mut batch, meta_cf, &hash);
  crate::db::put_tip_height(&mut batch, meta_cf, height);
  db.write(batch)
    .map_err(|e| crate::error::DaemonError::Db(format!("seed test db tip: {e}")))
}

/// Remove one canonical height mapping from a temporary RocksDB block store.
pub fn delete_height(db: &DB, height: u32) -> Result<(), crate::error::DaemonError> {
  let heights_cf = db
    .cf_handle(crate::db::CF_HEIGHTS)
    .ok_or_else(|| crate::error::DaemonError::Db("heights CF missing".to_owned()))?;
  let mut batch = rocksdb::WriteBatch::default();
  batch.delete_cf(heights_cf, height.to_be_bytes());
  db.write(batch)
    .map_err(|e| crate::error::DaemonError::Db(format!("delete test db height: {e}")))
}

/// Read the canonical tip height from a temporary RocksDB block store.
pub fn tip_height(db: &DB) -> Option<u32> {
  crate::db::get_tip_height(db)
}

/// Read the canonical tip hash from a temporary RocksDB block store.
pub fn tip_hash(db: &DB) -> Option<[u8; 32]> {
  crate::db::get_tip_hash(db)
}

/// Read the canonical hash stored for one height in a temporary RocksDB block store.
pub fn hash_for_height(db: &DB, height: u32) -> Option<[u8; 32]> {
  crate::db::get_hash_for_height(db, height)
}

/// Verify that a temporary RocksDB store covers the indexed PostgreSQL block height.
pub async fn verify_rocks_covers_pg(
  pool: &SqlPool,
  db: &Arc<DB>,
) -> Result<(), crate::error::DaemonError> {
  crate::recovery::verify_rocks_covers_pg(pool, db).await
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
  let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_nanos())
    .unwrap_or_default();
  let counter = TEST_DB_COUNTER.fetch_add(1, Ordering::Relaxed);
  std::env::temp_dir().join(format!("{prefix}-{timestamp}-{counter}"))
}
