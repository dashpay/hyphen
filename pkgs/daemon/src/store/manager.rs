//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Secondary store ownership and diagnostics.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use rocksdb::{IteratorMode, DB};
use tracing::info;

use crate::error::DaemonError;
use crate::policy::log::log_threshold_met;
use crate::store::{self, read_meta, StoreHandle, StoreId, StoreTarget};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, Default)]
pub struct SecondaryStoreDiag {
  pub name: String,
  pub required: bool,
  pub tip_height: Option<u32>,
  pub recovery_state: Option<String>,
  pub recovery_height: Option<u32>,
  pub live_sst_bytes: Option<u64>,
  pub estimate_num_keys: Option<u64>,
  pub actual_disk_bytes: Option<u64>,
  #[serde(default)]
  pub cf_sizes: Vec<(String, u64)>,
}

#[derive(Debug, Clone)]
pub struct SecondaryStoreSet {
  datadir: PathBuf,
  stores: HashMap<StoreId, StoreHandle>,
}

impl SecondaryStoreSet {
  pub fn open(
    datadir: &Path,
    chain: &crate::config::ChainConfig,
    cfg: Option<&crate::config::RocksdbConfig>,
  ) -> Result<Self, DaemonError> {
    let mut stores = HashMap::new();
    for id in StoreId::ALL {
      let handle = store::open(datadir, id, chain, cfg)?;
      stores.insert(id, handle);
    }
    Ok(Self {
      datadir: datadir.to_path_buf(),
      stores,
    })
  }

  pub fn datadir(&self) -> &Path {
    &self.datadir
  }

  pub fn handle(&self, id: StoreId) -> Result<&StoreHandle, DaemonError> {
    self
      .stores
      .get(&id)
      .ok_or_else(|| DaemonError::Db(format!("store {} not opened", id.as_str())))
  }

  pub fn db(&self, id: StoreId) -> Result<&Arc<DB>, DaemonError> {
    Ok(self.handle(id)?.db())
  }

  pub fn diagnostics(&self) -> Result<Vec<SecondaryStoreDiag>, DaemonError> {
    let mut diags = Vec::new();
    for id in StoreId::ALL {
      let handle = self.handle(id)?;
      let db = handle.db();
      let meta = read_meta(db)?;
      let path = self
        .datadir
        .join("stores")
        .join(format!("{}.db", id.as_str()));
      diags.push(SecondaryStoreDiag {
        name: id.as_str().to_owned(),
        required: handle.required(),
        tip_height: meta.tip_height,
        recovery_state: meta.recovery_state.map(|state| state.as_str().to_owned()),
        recovery_height: meta.recovery_height,
        live_sst_bytes: store::cf_sum(db, "rocksdb.live-sst-files-size"),
        estimate_num_keys: store::cf_sum(db, "rocksdb.estimate-num-keys"),
        actual_disk_bytes: Some(store::walk_dir_bytes(&path)),
        cf_sizes: store::cf_sizes(db),
      });
    }
    Ok(diags)
  }

  pub fn meta(&self, id: StoreId) -> Result<crate::store::StoreMeta, DaemonError> {
    self.handle(id)?.meta()
  }
}

pub fn rebuild_target(
  raw_db: &DB,
  datadir: &Path,
  chain: &crate::config::ChainConfig,
  cfg: Option<&crate::config::RocksdbConfig>,
  target: StoreTarget,
) -> Result<SecondaryStoreSet, DaemonError> {
  let started = Instant::now();
  info!(target = ?target, "Starting secondary store rebuild");
  for &id in target.ids() {
    store::reset_store_dir(datadir, id)?;
  }
  let stores = SecondaryStoreSet::open(datadir, chain, cfg)?;
  let raw_tip = crate::db::get_tip_height(raw_db);
  replay_raw_range(raw_db, &stores, chain, 0, raw_tip)?;
  info!(
    target = ?target,
    raw_tip,
    elapsed_ms = started.elapsed().as_millis() as u64,
    "Finished secondary store rebuild"
  );
  Ok(stores)
}

pub fn verify_target(
  raw_db: &DB,
  datadir: &Path,
  chain: &crate::config::ChainConfig,
  cfg: Option<&crate::config::RocksdbConfig>,
  target: StoreTarget,
) -> Result<(), DaemonError> {
  let started = Instant::now();
  info!(target = ?target, "Starting secondary store verify");
  let tmp_root = unique_temp_dir("hyphen-store-verify");
  std::fs::create_dir_all(&tmp_root).map_err(|e| DaemonError::Io {
    context: "create temp store verify dir",
    source: e,
  })?;
  let scratch = rebuild_target(raw_db, &tmp_root, chain, cfg, StoreTarget::All)?;
  let live = SecondaryStoreSet::open(datadir, chain, cfg)?;
  for &id in target.ids() {
    compare_store_contents(live.db(id)?, scratch.db(id)?, id)?;
  }
  std::fs::remove_dir_all(&tmp_root).map_err(|e| DaemonError::Io {
    context: "remove temp store verify dir",
    source: e,
  })?;
  info!(
    target = ?target,
    elapsed_ms = started.elapsed().as_millis() as u64,
    "Finished secondary store verify"
  );
  Ok(())
}

pub fn verify_recent(
  raw_db: &DB,
  stores: &SecondaryStoreSet,
  expected_tip: Option<u32>,
  chain: &crate::config::ChainConfig,
  depth: u32,
) -> Result<(), DaemonError> {
  let started = Instant::now();
  let Some(raw_tip) = crate::db::get_tip_height(raw_db) else {
    return Ok(());
  };
  let Some(expected_tip) = expected_tip else {
    return Ok(());
  };
  for id in StoreId::ALL {
    let meta = stores.meta(id)?;
    let state = meta
      .recovery_state
      .unwrap_or(crate::store::RecoveryState::Clean);
    if state != crate::store::RecoveryState::Clean {
      return Err(DaemonError::Index(format!(
        "secondary store {} is not clean at startup",
        id.as_str()
      )));
    }
    let Some(store_tip) = meta.tip_height else {
      return Err(DaemonError::Index(format!(
        "secondary store {} tip {:?} does not cover expected index tip {expected_tip}",
        id.as_str(),
        meta.tip_height
      )));
    };
    if store_tip < expected_tip {
      return Err(DaemonError::Index(format!(
        "secondary store {} tip {:?} does not cover expected index tip {expected_tip}",
        id.as_str(),
        meta.tip_height
      )));
    }
    if store_tip > raw_tip {
      return Err(DaemonError::Index(format!(
        "secondary store {} tip {store_tip} exceeds raw tip {raw_tip}",
        id.as_str()
      )));
    }
  }
  if expected_tip > raw_tip {
    return Err(DaemonError::Index(format!(
      "secondary store expected tip {expected_tip} exceeds raw tip {raw_tip}"
    )));
  }
  let start = expected_tip.saturating_sub(depth.saturating_sub(1));
  info!(
    start_height = start,
    expected_tip,
    raw_tip,
    verify_depth = expected_tip.saturating_sub(start) + 1,
    "Starting secondary store startup verify"
  );
  verify_recent_prevouts(
    raw_db,
    stores.db(StoreId::PrevoutLookup)?,
    start,
    expected_tip,
  )?;
  verify_recent_address_history(
    raw_db,
    stores.db(StoreId::PrevoutLookup)?,
    stores.db(StoreId::AddressHistory)?,
    start,
    expected_tip,
    chain,
  )?;
  info!(
    start_height = start,
    expected_tip,
    raw_tip,
    verify_depth = expected_tip.saturating_sub(start) + 1,
    elapsed_ms = started.elapsed().as_millis() as u64,
    "Finished secondary store startup verify"
  );
  Ok(())
}

fn replay_raw_range(
  raw_db: &DB,
  stores: &SecondaryStoreSet,
  chain: &crate::config::ChainConfig,
  start: u32,
  end: Option<u32>,
) -> Result<(), DaemonError> {
  let Some(end) = end else {
    return Ok(());
  };
  let prevout_db = stores.db(StoreId::PrevoutLookup)?;
  let history_db = stores.db(StoreId::AddressHistory)?;
  let mut last_progress_log: Option<(u32, Instant)> = None;
  for height in start..=end {
    let block = store::load_block_ctx(raw_db, height)?;
    let should_log_progress = log_threshold_met(last_progress_log, height, end);
    if should_log_progress {
      info!(
        indexed_height = height,
        raw_tip = end,
        remaining_blocks = end.saturating_sub(height),
        rebuild_pct = ((height.saturating_sub(start) + 1) * 100) / (end.saturating_sub(start) + 1),
        "secondary store rebuild progress from raw blocks"
      );
      last_progress_log = Some((height, Instant::now()));
    }
    crate::store::prevout::connect_block(
      prevout_db,
      &block,
      chain.p2pkh_version,
      chain.p2sh_version,
    )?;
    crate::store::address_history::connect_block_from_raw(
      history_db,
      prevout_db,
      &block,
      chain.p2pkh_version,
      chain.p2sh_version,
    )?;
  }
  Ok(())
}

fn compare_store_contents(left: &DB, right: &DB, id: StoreId) -> Result<(), DaemonError> {
  let left_cf = left
    .cf_handle(store::CF_DATA)
    .ok_or_else(|| DaemonError::Db("left store data CF missing".to_owned()))?;
  let right_cf = right
    .cf_handle(store::CF_DATA)
    .ok_or_else(|| DaemonError::Db("right store data CF missing".to_owned()))?;
  let mut left_iter = left.iterator_cf(left_cf, IteratorMode::Start);
  let mut right_iter = right.iterator_cf(right_cf, IteratorMode::Start);
  loop {
    match (left_iter.next(), right_iter.next()) {
      (None, None) => return Ok(()),
      (Some(Ok((left_key, left_val))), Some(Ok((right_key, right_val)))) => {
        if left_key.as_ref() != right_key.as_ref() || left_val.as_ref() != right_val.as_ref() {
          return Err(DaemonError::Index(format!(
            "secondary store {} differs from rebuilt scratch contents",
            id.as_str()
          )));
        }
      }
      (Some(Err(e)), _) | (_, Some(Err(e))) => {
        return Err(DaemonError::Db(format!(
          "iterate store {}: {e}",
          id.as_str()
        )));
      }
      _ => {
        return Err(DaemonError::Index(format!(
          "secondary store {} key coverage differs from rebuilt scratch contents",
          id.as_str()
        )));
      }
    }
  }
}

fn verify_recent_prevouts(
  raw_db: &DB,
  prevout_db: &DB,
  start: u32,
  end: u32,
) -> Result<(), DaemonError> {
  let mut last_progress_log: Option<(u32, Instant)> = None;
  for height in start..=end {
    if log_threshold_met(last_progress_log, height, end) {
      info!(
        phase = "prevout",
        indexed_height = height,
        verify_end = end,
        remaining_blocks = end.saturating_sub(height),
        verify_pct = ((height.saturating_sub(start) + 1) * 100) / (end.saturating_sub(start) + 1),
        "secondary store startup verify progress"
      );
      last_progress_log = Some((height, Instant::now()));
    }
    let block = store::load_block_ctx(raw_db, height)?;
    for (pos, transaction) in block.block().transactions.iter().enumerate() {
      let txid = block.txid(pos).ok_or_else(|| {
        DaemonError::Index(format!("tx {pos} missing txid at {}", block.height()))
      })?;
      let prev_txids = std::iter::repeat_with(|| txid.to_vec())
        .take(transaction.outputs.len())
        .collect::<Vec<_>>();
      let prev_vouts = (0..i32::try_from(transaction.outputs.len()).map_err(|_| {
        DaemonError::Index("verify recent prevout output count overflow".to_owned())
      })?)
        .collect::<Vec<_>>();
      let rows = crate::store::prevout::load_rows(prevout_db, &prev_txids, &prev_vouts)?;
      for (vout, output) in transaction.outputs.iter().enumerate() {
        let value = rows.get(vout).ok_or_else(|| {
          DaemonError::Index(format!(
            "startup verify missing prevout row for {}:{} at {}",
            hex::encode(txid),
            vout,
            height
          ))
        })?;
        if value.prev_value != output.value {
          return Err(DaemonError::Index(format!(
            "startup verify prevout value mismatch for {}:{} at {}",
            hex::encode(txid),
            vout,
            height
          )));
        }
      }
    }
  }
  Ok(())
}

fn verify_recent_address_history(
  raw_db: &DB,
  prevout_db: &DB,
  history_db: &DB,
  start: u32,
  end: u32,
  chain: &crate::config::ChainConfig,
) -> Result<(), DaemonError> {
  let mut last_progress_log: Option<(u32, Instant)> = None;
  for height in start..=end {
    if log_threshold_met(last_progress_log, height, end) {
      info!(
        phase = "address_history",
        indexed_height = height,
        verify_end = end,
        remaining_blocks = end.saturating_sub(height),
        verify_pct = ((height.saturating_sub(start) + 1) * 100) / (end.saturating_sub(start) + 1),
        "secondary store startup verify progress"
      );
      last_progress_log = Some((height, Instant::now()));
    }
    let block = store::load_block_ctx(raw_db, height)?;
    let mut rows = Vec::new();
    for (pos, transaction) in block.block().transactions.iter().enumerate() {
      let txid = block.txid(pos).ok_or_else(|| {
        DaemonError::Index(format!("tx {pos} missing txid at {}", block.height()))
      })?;
      let mut touched = std::collections::BTreeSet::new();
      for output in &transaction.outputs {
        if let Some(addr) = dash_primitives::script::derive_address(
          output.script_pubkey.as_bytes(),
          chain.p2pkh_version,
          chain.p2sh_version,
        ) {
          touched.insert(addr);
        }
      }
      let prev_txids: Vec<Vec<u8>> = transaction
        .inputs
        .iter()
        .filter(|input| input.prevout.hash.0 != [0u8; 32])
        .map(|input| input.prevout.hash.as_bytes().to_vec())
        .collect();
      let prev_vouts: Vec<i32> = transaction
        .inputs
        .iter()
        .filter(|input| input.prevout.hash.0 != [0u8; 32])
        .map(|input| input.prevout.index as i32)
        .collect();
      for row in crate::store::prevout::load_rows(prevout_db, &prev_txids, &prev_vouts)? {
        if let Some(addr) = row.prev_addr {
          touched.insert(addr);
        }
      }
      for addr in touched {
        rows.push(crate::store::address_history::key(
          &addr,
          height as i32,
          pos as i32,
          txid,
        ));
      }
    }
    for found in crate::store::address_history::keys_exist(history_db, &rows)? {
      if !found {
        return Err(DaemonError::Index(
          "startup verify missing address history key".to_owned(),
        ));
      }
    }
  }
  Ok(())
}

fn unique_temp_dir(prefix: &str) -> PathBuf {
  let timestamp = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .map(|d| d.as_nanos())
    .unwrap_or_default();
  std::env::temp_dir().join(format!("{prefix}-{timestamp}"))
}

#[cfg(test)]
mod tests {
  use rocksdb::DB;

  use super::{rebuild_target, verify_recent, verify_target, SecondaryStoreSet};

  fn seed_raw_block(db: &DB, height: u32, raw: &[u8]) -> Result<(), Box<dyn std::error::Error>> {
    let blocks_cf = db
      .cf_handle(crate::db::CF_BLOCKS)
      .ok_or("blocks CF missing")?;
    let heights_cf = db
      .cf_handle(crate::db::CF_HEIGHTS)
      .ok_or("heights CF missing")?;
    let meta_cf = db.cf_handle(crate::db::CF_META).ok_or("meta CF missing")?;
    let hash = dash_primitives::hash::block_hash(raw)?.0;
    let mut batch = rocksdb::WriteBatch::default();
    batch.put_cf(blocks_cf, hash, raw);
    batch.put_cf(heights_cf, height.to_be_bytes(), hash);
    crate::db::put_tip_hash(&mut batch, meta_cf, &hash);
    crate::db::put_tip_height(&mut batch, meta_cf, height);
    db.write(batch)?;
    Ok(())
  }

  #[test]
  fn verify_recent_accepts_empty_raw_store() -> Result<(), Box<dyn std::error::Error>> {
    let datadir = std::env::temp_dir().join(format!(
      "hyphen-store-manager-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
    ));
    std::fs::create_dir_all(&datadir)?;
    let raw = crate::db::open(&datadir, None)?;
    let stores = SecondaryStoreSet::open(&datadir, &crate::config::ChainConfig::default(), None)?;
    verify_recent(
      &raw,
      &stores,
      None,
      &crate::config::ChainConfig::default(),
      500,
    )?;
    std::fs::remove_dir_all(&datadir)?;
    Ok(())
  }

  #[test]
  fn rebuild_and_verify_store_round_trip_one_block() -> Result<(), Box<dyn std::error::Error>> {
    let datadir = std::env::temp_dir().join(format!(
      "hyphen-store-manager-rebuild-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
    ));
    std::fs::create_dir_all(&datadir)?;
    let raw = crate::db::open(&datadir, None)?;
    seed_raw_block(&raw, 0, &crate::tests::minimal_block_bytes(7))?;
    let stores = rebuild_target(
      &raw,
      &datadir,
      &crate::config::ChainConfig::default(),
      None,
      crate::store::StoreTarget::All,
    )?;
    assert_eq!(
      stores
        .meta(crate::store::StoreId::PrevoutLookup)?
        .tip_height,
      Some(0)
    );
    assert_eq!(
      stores
        .meta(crate::store::StoreId::AddressHistory)?
        .tip_height,
      Some(0)
    );
    verify_recent(
      &raw,
      &stores,
      Some(0),
      &crate::config::ChainConfig::default(),
      500,
    )?;
    drop(stores);
    verify_target(
      &raw,
      &datadir,
      &crate::config::ChainConfig::default(),
      None,
      crate::store::StoreTarget::All,
    )?;
    std::fs::remove_dir_all(&datadir)?;
    Ok(())
  }

  #[test]
  fn verify_recent_rejects_corrupted_secondary_store() -> Result<(), Box<dyn std::error::Error>> {
    let datadir = std::env::temp_dir().join(format!(
      "hyphen-store-manager-corrupt-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
    ));
    std::fs::create_dir_all(&datadir)?;
    let raw = crate::db::open(&datadir, None)?;
    seed_raw_block(&raw, 0, &crate::tests::minimal_block_bytes(9))?;
    let stores = rebuild_target(
      &raw,
      &datadir,
      &crate::config::ChainConfig::default(),
      None,
      crate::store::StoreTarget::All,
    )?;
    let prevout_db = stores.db(crate::store::StoreId::PrevoutLookup)?;
    let data_cf = prevout_db
      .cf_handle(crate::store::CF_DATA)
      .ok_or("prevout data CF missing")?;
    let mut iter = prevout_db.iterator_cf(data_cf, rocksdb::IteratorMode::Start);
    let (key, _) = iter.next().ok_or("missing prevout row")??;
    prevout_db.put_cf(data_cf, key, b"not-json")?;
    let err = verify_recent(
      &raw,
      &stores,
      Some(0),
      &crate::config::ChainConfig::default(),
      500,
    )
    .expect_err("corruption should be rejected");
    assert!(err.to_string().contains("prevout store"));
    std::fs::remove_dir_all(&datadir)?;
    Ok(())
  }

  #[test]
  fn verify_recent_rejects_dirty_store_metadata() -> Result<(), Box<dyn std::error::Error>> {
    let datadir = std::env::temp_dir().join(format!(
      "hyphen-store-manager-dirty-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
    ));
    std::fs::create_dir_all(&datadir)?;
    let raw = crate::db::open(&datadir, None)?;
    seed_raw_block(&raw, 0, &crate::tests::minimal_block_bytes(13))?;
    let stores = rebuild_target(
      &raw,
      &datadir,
      &crate::config::ChainConfig::default(),
      None,
      crate::store::StoreTarget::All,
    )?;
    let history_db = stores.db(crate::store::StoreId::AddressHistory)?;
    let meta_cf = history_db
      .cf_handle(crate::store::CF_META)
      .ok_or("history meta CF missing")?;
    let mut batch = rocksdb::WriteBatch::default();
    crate::store::put_recovery_state(
      &mut batch,
      meta_cf,
      crate::store::RecoveryState::Disconnecting,
      Some(0),
    );
    history_db.write(batch)?;
    let err = verify_recent(
      &raw,
      &stores,
      Some(0),
      &crate::config::ChainConfig::default(),
      500,
    )
    .expect_err("dirty recovery state should be rejected");
    assert!(err.to_string().contains("not clean"));
    std::fs::remove_dir_all(&datadir)?;
    Ok(())
  }

  #[test]
  fn verify_recent_allows_secondary_store_tip_ahead_of_postgres(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let datadir = std::env::temp_dir().join(format!(
      "hyphen-store-manager-ahead-{}",
      std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)?
        .as_nanos()
    ));
    std::fs::create_dir_all(&datadir)?;
    let raw = crate::db::open(&datadir, None)?;
    seed_raw_block(&raw, 0, &crate::tests::minimal_block_bytes(21))?;
    seed_raw_block(&raw, 1, &crate::tests::minimal_block_bytes(22))?;
    let stores = rebuild_target(
      &raw,
      &datadir,
      &crate::config::ChainConfig::default(),
      None,
      crate::store::StoreTarget::All,
    )?;

    verify_recent(
      &raw,
      &stores,
      Some(0),
      &crate::config::ChainConfig::default(),
      500,
    )?;

    std::fs::remove_dir_all(&datadir)?;
    Ok(())
  }
}
