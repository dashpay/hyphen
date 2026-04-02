//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Secondary RocksDB stores derived from the raw block store.

pub mod address_history;
pub mod manager;
pub mod prevout;

use std::path::{Path, PathBuf};
use std::sync::Arc;

use rocksdb::{ColumnFamilyDescriptor, Options, DB};
use serde::{Deserialize, Serialize};

use crate::db::{
  DEFAULT_MAX_BACKGROUND_JOBS, DEFAULT_MAX_OPEN_FILES, DEFAULT_WRITE_BUFFER_SIZE_MB,
};
use crate::error::DaemonError;
use crate::index::BlockCtx;

pub const CF_DATA: &str = "data";
pub const CF_META: &str = "meta";
pub const CF_CONSTANTS: &str = "constants";
pub const CF_NAMES: &[&str] = &[CF_DATA, CF_META, CF_CONSTANTS];

pub const META_TIP_HASH: &[u8] = b"tip_hash";
pub const META_TIP_HEIGHT: &[u8] = b"tip_height";
pub const META_RECOVERY_STATE: &[u8] = b"recovery_state";
pub const META_RECOVERY_HEIGHT: &[u8] = b"recovery_height";
pub const META_SCHEMA_VERSION: &[u8] = b"schema_version";

const CONST_CHAIN_P2PKH_VERSION: &[u8] = b"chain.p2pkh_version";
const CONST_CHAIN_P2SH_VERSION: &[u8] = b"chain.p2sh_version";
const BYTES_PER_MEBIBYTE: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreId {
  AddressHistory,
  PrevoutLookup,
}

impl StoreId {
  pub const ALL: [Self; 2] = [Self::AddressHistory, Self::PrevoutLookup];

  pub fn as_str(self) -> &'static str {
    match self {
      Self::AddressHistory => "address_history",
      Self::PrevoutLookup => "prevout_lookup",
    }
  }

  pub fn schema_version(self) -> u32 {
    match self {
      Self::AddressHistory => 1,
      Self::PrevoutLookup => 2,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StoreTarget {
  All,
  AddressHistory,
  PrevoutLookup,
}

impl StoreTarget {
  pub fn ids(self) -> &'static [StoreId] {
    match self {
      Self::All => &StoreId::ALL,
      Self::AddressHistory => &[StoreId::AddressHistory],
      Self::PrevoutLookup => &[StoreId::PrevoutLookup],
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryState {
  Clean,
  Rebuilding,
  Verifying,
  Disconnecting,
}

impl RecoveryState {
  pub fn as_str(self) -> &'static str {
    match self {
      Self::Clean => "clean",
      Self::Rebuilding => "rebuilding",
      Self::Verifying => "verifying",
      Self::Disconnecting => "disconnecting",
    }
  }

  pub fn parse(raw: &[u8]) -> Option<Self> {
    match raw {
      b"clean" => Some(Self::Clean),
      b"rebuilding" => Some(Self::Rebuilding),
      b"verifying" => Some(Self::Verifying),
      b"disconnecting" => Some(Self::Disconnecting),
      _ => None,
    }
  }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct StoreMeta {
  pub tip_height: Option<u32>,
  pub tip_hash: Option<[u8; 32]>,
  pub recovery_state: Option<RecoveryState>,
  pub recovery_height: Option<u32>,
}

#[derive(Debug, Clone)]
pub struct StoreHandle {
  id: StoreId,
  db: Arc<DB>,
  required: bool,
}

impl StoreHandle {
  pub fn new(id: StoreId, db: DB, required: bool) -> Self {
    Self {
      id,
      db: Arc::new(db),
      required,
    }
  }

  pub fn id(&self) -> StoreId {
    self.id
  }

  pub fn db(&self) -> &Arc<DB> {
    &self.db
  }

  pub fn required(&self) -> bool {
    self.required
  }

  pub fn meta(&self) -> Result<StoreMeta, DaemonError> {
    read_meta(&self.db)
  }
}

pub fn open(
  datadir: &Path,
  id: StoreId,
  chain: &crate::config::ChainConfig,
  cfg: Option<&crate::config::RocksdbConfig>,
) -> Result<StoreHandle, DaemonError> {
  let root = datadir.join("stores");
  std::fs::create_dir_all(&root).map_err(|e| DaemonError::Io {
    context: "create store root",
    source: e,
  })?;
  let db_path = root.join(format!("{}.db", id.as_str()));
  let mut opts = Options::default();
  opts.create_if_missing(true);
  opts.create_missing_column_families(true);

  let cfg = cfg.cloned().unwrap_or(crate::config::RocksdbConfig {
    write_buffer_size_mb: DEFAULT_WRITE_BUFFER_SIZE_MB,
    max_background_jobs: DEFAULT_MAX_BACKGROUND_JOBS,
    max_open_files: DEFAULT_MAX_OPEN_FILES,
  });
  opts.set_write_buffer_size(cfg.write_buffer_size_mb * BYTES_PER_MEBIBYTE);
  opts.set_max_background_jobs(cfg.max_background_jobs);
  opts.set_max_open_files(cfg.max_open_files);

  let cfs = vec![
    ColumnFamilyDescriptor::new(CF_DATA, Options::default()),
    ColumnFamilyDescriptor::new(CF_META, Options::default()),
    ColumnFamilyDescriptor::new(CF_CONSTANTS, Options::default()),
  ];
  let db =
    DB::open_cf_descriptors(&opts, &db_path, cfs).map_err(|e| DaemonError::Db(e.to_string()))?;
  verify_fixed_constants(&db, chain)?;
  verify_schema_version(&db, id)?;
  Ok(StoreHandle::new(id, db, true))
}

pub fn store_root(datadir: &Path) -> PathBuf {
  datadir.join("stores")
}

pub fn store_path(datadir: &Path, id: StoreId) -> PathBuf {
  store_root(datadir).join(format!("{}.db", id.as_str()))
}

pub fn put_tip_hash(
  batch: &mut rocksdb::WriteBatch,
  meta_cf: &rocksdb::ColumnFamily,
  hash: &[u8; 32],
) {
  batch.put_cf(meta_cf, META_TIP_HASH, hash);
}

pub fn put_tip_height(
  batch: &mut rocksdb::WriteBatch,
  meta_cf: &rocksdb::ColumnFamily,
  height: u32,
) {
  batch.put_cf(meta_cf, META_TIP_HEIGHT, height.to_be_bytes());
}

pub fn put_recovery_state(
  batch: &mut rocksdb::WriteBatch,
  meta_cf: &rocksdb::ColumnFamily,
  state: RecoveryState,
  height: Option<u32>,
) {
  batch.put_cf(meta_cf, META_RECOVERY_STATE, state.as_str().as_bytes());
  match height {
    Some(height) => batch.put_cf(meta_cf, META_RECOVERY_HEIGHT, height.to_be_bytes()),
    None => batch.delete_cf(meta_cf, META_RECOVERY_HEIGHT),
  }
}

pub fn mark_recovery_state(
  db: &DB,
  state: RecoveryState,
  height: Option<u32>,
) -> Result<(), DaemonError> {
  let meta_cf = db
    .cf_handle(CF_META)
    .ok_or_else(|| DaemonError::Db("store meta CF missing".to_owned()))?;
  let mut batch = rocksdb::WriteBatch::default();
  put_recovery_state(&mut batch, meta_cf, state, height);
  db.write(batch)
    .map_err(|e| DaemonError::Db(format!("write store recovery state: {e}")))
}

pub fn clear_tip(batch: &mut rocksdb::WriteBatch, meta_cf: &rocksdb::ColumnFamily) {
  batch.delete_cf(meta_cf, META_TIP_HASH);
  batch.delete_cf(meta_cf, META_TIP_HEIGHT);
}

pub fn set_clean(
  batch: &mut rocksdb::WriteBatch,
  meta_cf: &rocksdb::ColumnFamily,
  tip_height: Option<u32>,
  tip_hash: Option<[u8; 32]>,
) {
  clear_tip(batch, meta_cf);
  if let Some(hash) = tip_hash.as_ref() {
    put_tip_hash(batch, meta_cf, hash);
  }
  if let Some(height) = tip_height {
    put_tip_height(batch, meta_cf, height);
  }
  put_recovery_state(batch, meta_cf, RecoveryState::Clean, None);
}

pub fn reset_store_dir(datadir: &Path, id: StoreId) -> Result<(), DaemonError> {
  let path = store_path(datadir, id);
  if path.exists() {
    std::fs::remove_dir_all(&path).map_err(|e| DaemonError::Io {
      context: "remove store directory",
      source: e,
    })?;
  }
  Ok(())
}

pub fn read_meta(db: &DB) -> Result<StoreMeta, DaemonError> {
  let meta_cf = db
    .cf_handle(CF_META)
    .ok_or_else(|| DaemonError::Db("store meta CF missing".to_owned()))?;
  let tip_hash = db
    .get_cf(meta_cf, META_TIP_HASH)
    .map_err(|e| DaemonError::Db(format!("read store tip hash: {e}")))?
    .map(|raw| {
      if raw.len() != 32 {
        return Err(DaemonError::Db(
          "store tip hash has invalid length".to_owned(),
        ));
      }
      let mut hash = [0_u8; 32];
      hash.copy_from_slice(&raw);
      Ok(hash)
    })
    .transpose()?;
  let tip_height = db
    .get_cf(meta_cf, META_TIP_HEIGHT)
    .map_err(|e| DaemonError::Db(format!("read store tip height: {e}")))?
    .map(|raw| {
      if raw.len() != 4 {
        return Err(DaemonError::Db(
          "store tip height has invalid length".to_owned(),
        ));
      }
      Ok(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
    })
    .transpose()?;
  let recovery_state = db
    .get_cf(meta_cf, META_RECOVERY_STATE)
    .map_err(|e| DaemonError::Db(format!("read store recovery state: {e}")))?
    .map(|raw| {
      RecoveryState::parse(&raw)
        .ok_or_else(|| DaemonError::Db("store recovery state is invalid".to_owned()))
    })
    .transpose()?;
  let recovery_height = db
    .get_cf(meta_cf, META_RECOVERY_HEIGHT)
    .map_err(|e| DaemonError::Db(format!("read store recovery height: {e}")))?
    .map(|raw| {
      if raw.len() != 4 {
        return Err(DaemonError::Db(
          "store recovery height has invalid length".to_owned(),
        ));
      }
      Ok(u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]))
    })
    .transpose()?;
  Ok(StoreMeta {
    tip_height,
    tip_hash,
    recovery_state,
    recovery_height,
  })
}

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
  any.then_some(total)
}

pub fn cf_first(db: &DB, prop: &str) -> Option<u64> {
  CF_NAMES.iter().find_map(|&name| {
    db.cf_handle(name)
      .and_then(|cf| db.property_int_value_cf(cf, prop).ok().flatten())
  })
}

pub fn cf_sizes(db: &DB) -> Vec<(String, u64)> {
  CF_NAMES
    .iter()
    .filter_map(|&name| {
      db.cf_handle(name).and_then(|cf| {
        db.property_int_value_cf(cf, "rocksdb.live-sst-files-size")
          .ok()
          .flatten()
          .map(|bytes| (name.to_owned(), bytes))
      })
    })
    .collect()
}

pub fn walk_dir_bytes(path: &Path) -> u64 {
  let Ok(rd) = std::fs::read_dir(path) else {
    return 0;
  };
  rd.flatten().fold(0_u64, |acc, entry| {
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

pub fn load_block_ctx(raw_db: &DB, height: u32) -> Result<BlockCtx, DaemonError> {
  let Some(hash) = crate::db::get_hash_for_height(raw_db, height) else {
    return Err(DaemonError::Index(format!(
      "raw store missing canonical hash at height {height}"
    )));
  };
  let blocks_cf = raw_db
    .cf_handle(crate::db::CF_BLOCKS)
    .ok_or_else(|| DaemonError::Db("raw blocks CF missing".to_owned()))?;
  let Some(raw) = raw_db
    .get_cf(blocks_cf, hash)
    .map_err(|e| DaemonError::Db(format!("read raw block {height}: {e}")))?
  else {
    return Err(DaemonError::Index(format!(
      "raw store missing block payload at height {height}"
    )));
  };
  BlockCtx::from_raw(height, raw.to_vec())
}

pub fn verify_fixed_constants(
  db: &DB,
  chain: &crate::config::ChainConfig,
) -> Result<(), DaemonError> {
  let constants_cf = db
    .cf_handle(CF_CONSTANTS)
    .ok_or_else(|| DaemonError::Db("store constants CF missing".to_owned()))?;
  let constants = [
    (
      CONST_CHAIN_P2PKH_VERSION,
      "chain.p2pkh_version",
      chain.p2pkh_version,
    ),
    (
      CONST_CHAIN_P2SH_VERSION,
      "chain.p2sh_version",
      chain.p2sh_version,
    ),
  ];
  let mut batch = rocksdb::WriteBatch::default();
  let mut has_missing = false;

  for (key, name, current) in constants {
    match db
      .get_cf(constants_cf, key)
      .map_err(|e| DaemonError::Db(format!("read store constant {name}: {e}")))?
    {
      Some(stored) => {
        if stored.len() != 1 {
          return Err(DaemonError::Db(format!(
            "store constant {name} has invalid length {}",
            stored.len()
          )));
        }
        if stored[0] != current {
          return Err(DaemonError::Config(format!(
            "cannot start hyphen, store configuration option \"{name}\" was {}, now {}, rebuild the store or revert back to previous option",
            stored[0],
            current
          )));
        }
      }
      None => {
        batch.put_cf(constants_cf, key, [current]);
        has_missing = true;
      }
    }
  }
  if has_missing {
    db.write(batch)
      .map_err(|e| DaemonError::Db(format!("write store constants: {e}")))?;
  }
  Ok(())
}

fn verify_schema_version(db: &DB, id: StoreId) -> Result<(), DaemonError> {
  let meta_cf = db
    .cf_handle(CF_META)
    .ok_or_else(|| DaemonError::Db("store meta CF missing".to_owned()))?;
  match db
    .get_cf(meta_cf, META_SCHEMA_VERSION)
    .map_err(|e| DaemonError::Db(format!("read store schema version: {e}")))?
  {
    Some(raw) => {
      if raw.len() != 4 {
        return Err(DaemonError::Db(
          "store schema version has invalid length".to_owned(),
        ));
      }
      let stored = u32::from_be_bytes([raw[0], raw[1], raw[2], raw[3]]);
      if stored != id.schema_version() {
        return Err(DaemonError::Config(format!(
          "store {} schema version was {}, now {}, rebuild the store",
          id.as_str(),
          stored,
          id.schema_version()
        )));
      }
    }
    None => {
      let mut batch = rocksdb::WriteBatch::default();
      batch.put_cf(
        meta_cf,
        META_SCHEMA_VERSION,
        id.schema_version().to_be_bytes(),
      );
      batch.put_cf(
        meta_cf,
        META_RECOVERY_STATE,
        RecoveryState::Clean.as_str().as_bytes(),
      );
      db.write(batch)
        .map_err(|e| DaemonError::Db(format!("write store schema version: {e}")))?;
    }
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use std::time::{SystemTime, UNIX_EPOCH};

  use super::{mark_recovery_state, open, read_meta, RecoveryState, StoreId};

  fn temp_datadir(prefix: &str) -> std::path::PathBuf {
    let nanos = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .map(|d| d.as_nanos())
      .unwrap_or_default();
    std::env::temp_dir().join(format!("{prefix}-{nanos}"))
  }

  #[test]
  fn store_open_initializes_clean_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let datadir = temp_datadir("hyphen-store-open");
    std::fs::create_dir_all(&datadir)?;
    let handle = open(
      &datadir,
      StoreId::PrevoutLookup,
      &crate::config::ChainConfig::default(),
      None,
    )?;
    let meta = read_meta(handle.db())?;
    assert_eq!(meta.tip_height, None);
    assert_eq!(meta.tip_hash, None);
    assert_eq!(meta.recovery_state, Some(RecoveryState::Clean));
    assert_eq!(meta.recovery_height, None);
    std::fs::remove_dir_all(&datadir)?;
    Ok(())
  }

  #[test]
  fn mark_recovery_state_persists_dirty_metadata_before_followup_write(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let datadir = temp_datadir("hyphen-store-dirty-meta");
    std::fs::create_dir_all(&datadir)?;
    let handle = open(
      &datadir,
      StoreId::AddressHistory,
      &crate::config::ChainConfig::default(),
      None,
    )?;
    mark_recovery_state(handle.db(), RecoveryState::Rebuilding, Some(42))?;
    let meta = read_meta(handle.db())?;
    assert_eq!(meta.recovery_state, Some(RecoveryState::Rebuilding));
    assert_eq!(meta.recovery_height, Some(42));
    std::fs::remove_dir_all(&datadir)?;
    Ok(())
  }
}
