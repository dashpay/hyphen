//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! RocksDB block store with blocks/heights/meta/constants column families.

use std::path::Path;

use rocksdb::{ColumnFamilyDescriptor, Options, DB};

use crate::error::DaemonError;

pub const DEFAULT_MAX_OPEN_FILES: i32 = 64;

pub const DEFAULT_MAX_BACKGROUND_JOBS: i32 = 2;

pub const DEFAULT_WRITE_BUFFER_SIZE_MB: usize = 64;

/// Column family storing raw block bytes keyed by block hash.
pub const CF_BLOCKS: &str = "blocks";
/// Column family storing block hashes keyed by height.
pub const CF_HEIGHTS: &str = "heights";
/// Column family storing singleton metadata values.
pub const CF_META: &str = "meta";
/// Column family storing fixed persisted-data constants.
pub const CF_CONSTANTS: &str = "constants";

/// Metadata key containing the canonical tip hash.
pub const META_TIP_HASH: &[u8] = b"tip_hash";
/// Metadata key containing the canonical tip height.
pub const META_TIP_HEIGHT: &[u8] = b"tip_height";

const CONST_CHAIN_P2PKH_VERSION: &[u8] = b"chain.p2pkh_version";
const CONST_CHAIN_P2SH_VERSION: &[u8] = b"chain.p2sh_version";

const BYTES_PER_MEBIBYTE: usize = 1024 * 1024;

/// Read the canonical tip hash from the metadata column family.
pub fn get_tip_hash(db: &DB) -> Option<[u8; 32]> {
  match try_get_tip_hash(db) {
    Ok(hash) => hash,
    Err(error) => {
      tracing::warn!("read RocksDB tip hash: {error}");
      None
    }
  }
}

pub fn try_get_tip_hash(db: &DB) -> Result<Option<[u8; 32]>, DaemonError> {
  let Some(meta_cf) = db.cf_handle(CF_META) else {
    return Ok(None);
  };
  let Some(val) = db
    .get_cf(meta_cf, META_TIP_HASH)
    .map_err(|e| DaemonError::Db(format!("read RocksDB tip hash: {e}")))?
  else {
    return Ok(None);
  };
  if val.len() != 32 {
    return Ok(None);
  }
  let mut hash = [0u8; 32];
  hash.copy_from_slice(&val);
  Ok(Some(hash))
}

/// Read the canonical tip height from the metadata column family.
pub fn get_tip_height(db: &DB) -> Option<u32> {
  match try_get_tip_height(db) {
    Ok(height) => height,
    Err(error) => {
      tracing::warn!("read RocksDB tip height: {error}");
      None
    }
  }
}

pub fn try_get_tip_height(db: &DB) -> Result<Option<u32>, DaemonError> {
  let meta_cf = db
    .cf_handle(CF_META)
    .ok_or_else(|| DaemonError::Db("read RocksDB tip height: meta CF missing".to_owned()))?;
  let Some(val) = db
    .get_cf(meta_cf, META_TIP_HEIGHT)
    .map_err(|e| DaemonError::Db(format!("read RocksDB tip height: {e}")))?
  else {
    return Ok(None);
  };
  if val.len() != 4 {
    return Err(DaemonError::Db(format!(
      "read RocksDB tip height: expected 4 bytes, got {}",
      val.len()
    )));
  }
  Ok(Some(u32::from_be_bytes([val[0], val[1], val[2], val[3]])))
}

/// Look up the canonical block hash recorded for `height`.
pub fn get_hash_for_height(db: &DB, height: u32) -> Option<[u8; 32]> {
  match try_get_hash_for_height(db, height) {
    Ok(hash) => hash,
    Err(error) => {
      tracing::warn!("read RocksDB height mapping: {error}");
      None
    }
  }
}

pub fn try_get_hash_for_height(db: &DB, height: u32) -> Result<Option<[u8; 32]>, DaemonError> {
  let heights_cf = db.cf_handle(CF_HEIGHTS).ok_or_else(|| {
    DaemonError::Db(format!(
      "read RocksDB height {height} mapping: heights CF missing"
    ))
  })?;
  let Some(val) = db
    .get_cf(heights_cf, height.to_be_bytes())
    .map_err(|e| DaemonError::Db(format!("read RocksDB height {height} mapping: {e}")))?
  else {
    return Ok(None);
  };
  if val.len() != 32 {
    return Err(DaemonError::Db(format!(
      "read RocksDB height {height} mapping: expected 32 bytes, got {}",
      val.len()
    )));
  }
  let mut hash = [0u8; 32];
  hash.copy_from_slice(&val);
  Ok(Some(hash))
}

/// Queue a tip-hash update in a RocksDB write batch.
pub fn put_tip_hash(
  batch: &mut rocksdb::WriteBatch,
  meta_cf: &rocksdb::ColumnFamily,
  hash: &[u8; 32],
) {
  batch.put_cf(meta_cf, META_TIP_HASH, hash);
}

/// Queue a tip-height update in a RocksDB write batch.
pub fn put_tip_height(
  batch: &mut rocksdb::WriteBatch,
  meta_cf: &rocksdb::ColumnFamily,
  height: u32,
) {
  batch.put_cf(meta_cf, META_TIP_HEIGHT, height.to_be_bytes());
}

/// Remove both tip metadata keys in one write batch.
pub fn delete_tip(batch: &mut rocksdb::WriteBatch, meta_cf: &rocksdb::ColumnFamily) {
  batch.delete_cf(meta_cf, META_TIP_HASH);
  batch.delete_cf(meta_cf, META_TIP_HEIGHT);
}

/// Opens RocksDB at `datadir/blocks.db` with four column families.
/// Applies tunables from `RocksdbConfig` if provided.
///
/// # Errors
///
/// Returns `DaemonError::Db` when RocksDB cannot be opened or one of the
/// configured options is rejected by the backend.
pub fn open(datadir: &Path, cfg: Option<&crate::config::RocksdbConfig>) -> Result<DB, DaemonError> {
  let db_path = datadir.join("blocks.db");
  let mut opts = Options::default();
  opts.create_if_missing(true);
  opts.create_missing_column_families(true);

  let cfg = cfg.cloned().unwrap_or_default();
  opts.set_write_buffer_size(cfg.write_buffer_size_mb * BYTES_PER_MEBIBYTE);
  opts.set_max_background_jobs(cfg.max_background_jobs);
  opts.set_max_open_files(cfg.max_open_files);

  let cfs = vec![
    ColumnFamilyDescriptor::new(CF_BLOCKS, Options::default()),
    ColumnFamilyDescriptor::new(CF_HEIGHTS, Options::default()),
    ColumnFamilyDescriptor::new(CF_META, Options::default()),
    ColumnFamilyDescriptor::new(CF_CONSTANTS, Options::default()),
  ];
  DB::open_cf_descriptors(&opts, &db_path, cfs).map_err(|e| DaemonError::Db(e.to_string()))
}

/// All column family names.
pub const CF_NAMES: &[&str] = &[CF_BLOCKS, CF_HEIGHTS, CF_META, CF_CONSTANTS];

/// Verify that persisted-data-affecting constants still match runtime config.
///
/// Missing constants are initialized from the current config on first start.
///
/// # Errors
///
/// Returns `DaemonError::Config` when a stored value differs from the current
/// runtime config. Returns `DaemonError::Db` when the constants column family
/// is missing or contains malformed data.
pub fn verify_fixed_constants(
  db: &DB,
  chain: &crate::config::ChainConfig,
) -> Result<(), DaemonError> {
  let constants_cf = db
    .cf_handle(CF_CONSTANTS)
    .ok_or_else(|| DaemonError::Db("constants CF missing".to_owned()))?;
  let constants = [
    FixedConstant {
      key: CONST_CHAIN_P2PKH_VERSION,
      name: "chain.p2pkh_version",
      current: chain.p2pkh_version,
    },
    FixedConstant {
      key: CONST_CHAIN_P2SH_VERSION,
      name: "chain.p2sh_version",
      current: chain.p2sh_version,
    },
  ];
  let mut batch = rocksdb::WriteBatch::default();
  let mut has_missing = false;

  for constant in constants {
    match db
      .get_cf(constants_cf, constant.key)
      .map_err(|e| DaemonError::Db(format!("read persisted constant {}: {e}", constant.name)))?
    {
      Some(stored) => {
        let Some(&stored_value) = stored.first() else {
          return Err(DaemonError::Db(format!(
            "persisted constant {} is empty",
            constant.name
          )));
        };
        if stored.len() != 1 {
          return Err(DaemonError::Db(format!(
            "persisted constant {} has invalid length {}",
            constant.name,
            stored.len()
          )));
        }
        if stored_value != constant.current {
          return Err(DaemonError::Config(format!(
            "cannot start hyphen, configuration option \"{}\" was {}, now {}, start with --reindex or revert back to previous option",
            constant.name,
            stored_value,
            constant.current
          )));
        }
      }
      None => {
        batch.put_cf(constants_cf, constant.key, [constant.current]);
        has_missing = true;
      }
    }
  }

  if has_missing {
    db.write(batch)
      .map_err(|e| DaemonError::Db(format!("write persisted constants: {e}")))?;
  }

  Ok(())
}

#[derive(Clone, Copy, Debug)]
struct FixedConstant {
  key: &'static [u8],
  name: &'static str,
  current: u8,
}

/// Query a RocksDB integer property summed across all CFs.
pub fn cf_sum(db: &DB, prop: &str) -> Option<u64> {
  let mut total = 0u64;
  let mut any = false;
  for &name in CF_NAMES {
    if let Some(cf) = db.cf_handle(name) {
      if let Ok(Some(v)) = db.property_int_value_cf(cf, prop) {
        total += v;
        any = true;
      }
    }
  }
  if any {
    Some(total)
  } else {
    None
  }
}

/// Query a RocksDB integer property from the first CF that reports it.
pub fn cf_first(db: &DB, prop: &str) -> Option<u64> {
  CF_NAMES.iter().find_map(|&name| {
    db.cf_handle(name)
      .and_then(|cf| db.property_int_value_cf(cf, prop).ok().flatten())
  })
}

/// Collect live SST sizes for each known column family.
pub fn cf_sizes(db: &DB) -> Vec<(String, u64)> {
  CF_NAMES
    .iter()
    .filter_map(|&name| {
      db.cf_handle(name).and_then(|cf| {
        db.property_int_value_cf(cf, "rocksdb.live-sst-files-size")
          .ok()
          .flatten()
          .map(|bytes| (name.to_string(), bytes))
      })
    })
    .collect()
}

/// Recursively sum file sizes under `path`.
pub fn walk_dir_bytes(path: &Path) -> u64 {
  let Ok(rd) = std::fs::read_dir(path) else {
    return 0;
  };
  rd.flatten().fold(0u64, |acc, entry| {
    let Ok(meta) = entry.metadata() else {
      return acc;
    };
    if meta.is_dir() {
      acc + walk_dir_bytes(&entry.path())
    } else {
      acc + meta.len()
    }
  })
}

/// Build a diagnostic snapshot from the current RocksDB state.
pub fn rocksdb_diag(db: &DB, db_path: &Path) -> hyphend::wire::msg::RocksDbDiag {
  hyphend::wire::msg::RocksDbDiag {
    live_sst_bytes: cf_sum(db, "rocksdb.live-sst-files-size"),
    mem_table_bytes: cf_sum(db, "rocksdb.cur-size-all-mem-tables"),
    block_cache_usage: cf_first(db, "rocksdb.block-cache-usage"),
    block_cache_capacity: cf_first(db, "rocksdb.block-cache-capacity"),
    estimate_num_keys: cf_sum(db, "rocksdb.estimate-num-keys"),
    compaction_pending: cf_first(db, "rocksdb.compaction-pending").map(|v| v > 0),
    num_running_compactions: cf_first(db, "rocksdb.num-running-compactions"),
    write_buffer_size_mb: None,
    max_background_jobs: None,
    max_open_files: None,
    cf_sizes: cf_sizes(db),
    actual_disk_bytes: Some(walk_dir_bytes(db_path)),
  }
}

#[cfg(test)]
mod tests {
  use std::path::PathBuf;
  use std::sync::atomic::{AtomicU64, Ordering};
  use std::time::{SystemTime, UNIX_EPOCH};

  use super::{
    open, verify_fixed_constants, CF_CONSTANTS, CONST_CHAIN_P2PKH_VERSION, CONST_CHAIN_P2SH_VERSION,
  };
  use crate::config::ChainConfig;
  use crate::error::DaemonError;

  #[test]
  fn verify_fixed_constants_initializes_missing_values() -> Result<(), DaemonError> {
    let db = open_test_db()?;
    let chain = ChainConfig::default();

    verify_fixed_constants(&db, &chain)?;

    let constants_cf = db
      .cf_handle(CF_CONSTANTS)
      .ok_or_else(|| DaemonError::Db("constants CF missing".to_owned()))?;
    assert_eq!(
      db.get_cf(constants_cf, CONST_CHAIN_P2PKH_VERSION)
        .map_err(|e| DaemonError::Db(format!("read p2pkh constant: {e}")))?,
      Some(vec![chain.p2pkh_version])
    );
    assert_eq!(
      db.get_cf(constants_cf, CONST_CHAIN_P2SH_VERSION)
        .map_err(|e| DaemonError::Db(format!("read p2sh constant: {e}")))?,
      Some(vec![chain.p2sh_version])
    );

    Ok(())
  }

  #[test]
  fn verify_fixed_constants_accepts_matching_restart() -> Result<(), DaemonError> {
    let db = open_test_db()?;
    let chain = ChainConfig::default();

    verify_fixed_constants(&db, &chain)?;
    verify_fixed_constants(&db, &chain)?;

    Ok(())
  }

  #[test]
  fn verify_fixed_constants_rejects_changed_value() -> Result<(), DaemonError> {
    let db = open_test_db()?;
    let chain = ChainConfig::default();
    verify_fixed_constants(&db, &chain)?;

    let changed = ChainConfig {
      p2pkh_version: chain.p2pkh_version.wrapping_add(1),
      ..chain
    };
    let err = verify_fixed_constants(&db, &changed)
      .expect_err("changed persisted constant should be rejected");
    assert!(matches!(
      err,
      DaemonError::Config(message)
        if message.contains("chain.p2pkh_version")
          && message.contains("--reindex")
    ));

    Ok(())
  }

  fn open_test_db() -> Result<rocksdb::DB, DaemonError> {
    let mut path = std::env::temp_dir();
    path.push(unique_db_name());
    open(&path, None)
  }

  fn unique_db_name() -> PathBuf {
    static NEXT_DB_ID: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|duration| duration.as_nanos())
      .unwrap_or(0);
    let unique_id = NEXT_DB_ID.fetch_add(1, Ordering::Relaxed);
    PathBuf::from(format!("hyphend-db-test-{nanos}-{unique_id}"))
  }
}
