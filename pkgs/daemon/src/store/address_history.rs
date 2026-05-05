//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Address-history secondary store.

use std::collections::HashSet;

use rocksdb::{Direction, IteratorMode};

use crate::error::DaemonError;
use crate::index::BlockCtx;
use crate::store::{self, prevout, RecoveryState};

const TXID_LEN: usize = 32;
const HEIGHT_BYTES: usize = 4;
const POS_BYTES: usize = 4;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressHistoryRow {
  pub txid: Vec<u8>,
  pub block_height: i32,
  pub block_pos: i32,
}

pub fn key(address: &str, block_height: i32, block_pos: i32, txid: &[u8]) -> Vec<u8> {
  let mut key = Vec::with_capacity(address.len() + 1 + 4 + 4 + txid.len());
  key.extend_from_slice(address.as_bytes());
  key.push(0);
  key.extend_from_slice(&block_height.to_be_bytes());
  key.extend_from_slice(&block_pos.to_be_bytes());
  key.extend_from_slice(txid);
  key
}

fn prefix(address: &str) -> Vec<u8> {
  let mut prefix = Vec::with_capacity(address.len() + 1);
  prefix.extend_from_slice(address.as_bytes());
  prefix.push(0);
  prefix
}

fn upper_bound_key(address: &str) -> Vec<u8> {
  let mut key = prefix(address);
  key.extend_from_slice(&[0xFF; HEIGHT_BYTES + POS_BYTES + TXID_LEN]);
  key
}

fn decode_row(address: &str, key: &[u8]) -> Result<AddressHistoryRow, DaemonError> {
  let prefix_len = address.len() + 1;
  let expected_len = prefix_len + HEIGHT_BYTES + POS_BYTES + TXID_LEN;
  if key.len() != expected_len {
    return Err(DaemonError::Db(format!(
      "decode address history row: expected key len {expected_len}, got {}",
      key.len()
    )));
  }
  let block_height = i32::from_be_bytes(
    key[prefix_len..prefix_len + HEIGHT_BYTES]
      .try_into()
      .map_err(|_| DaemonError::Db("decode address history block height".to_owned()))?,
  );
  let block_pos = i32::from_be_bytes(
    key[prefix_len + HEIGHT_BYTES..prefix_len + HEIGHT_BYTES + POS_BYTES]
      .try_into()
      .map_err(|_| DaemonError::Db("decode address history block position".to_owned()))?,
  );
  let txid = key[prefix_len + HEIGHT_BYTES + POS_BYTES..].to_vec();
  Ok(AddressHistoryRow {
    txid,
    block_height,
    block_pos,
  })
}

pub fn page_rows(
  db: &rocksdb::DB,
  address: &str,
  max_height: i32,
  limit: usize,
  before: Option<(i32, i32, &[u8])>,
) -> Result<Vec<AddressHistoryRow>, DaemonError> {
  let data_cf = db
    .cf_handle(store::CF_DATA)
    .ok_or_else(|| DaemonError::Db("address history store data CF missing".to_owned()))?;
  let prefix = prefix(address);
  let start_key = before
    .map(|(block_height, block_pos, txid)| key(address, block_height, block_pos, txid))
    .unwrap_or_else(|| upper_bound_key(address));
  let mut rows = Vec::new();
  let iter = db.iterator_cf(data_cf, IteratorMode::From(&start_key, Direction::Reverse));

  for item in iter {
    let (raw_key, _) =
      item.map_err(|e| DaemonError::Db(format!("iterate address history store rows: {e}")))?;
    if !raw_key.starts_with(&prefix) {
      break;
    }
    if before
      .as_ref()
      .is_some_and(|_| raw_key.as_ref() == start_key.as_slice())
    {
      continue;
    }
    let row = decode_row(address, &raw_key)?;
    if row.block_height > max_height {
      continue;
    }
    rows.push(row);
    if rows.len() >= limit {
      break;
    }
  }

  Ok(rows)
}

pub fn keys_exist(db: &rocksdb::DB, keys: &[Vec<u8>]) -> Result<Vec<bool>, DaemonError> {
  if keys.is_empty() {
    return Ok(Vec::new());
  }
  let data_cf = db
    .cf_handle(store::CF_DATA)
    .ok_or_else(|| DaemonError::Db("address history store data CF missing".to_owned()))?;
  let rows = db.batched_multi_get_cf(data_cf, keys.iter().map(std::vec::Vec::as_slice), false);
  let mut found = Vec::with_capacity(rows.len());
  for row in rows {
    found.push(
      row
        .map_err(|e| DaemonError::Db(format!("read address history store row: {e}")))?
        .is_some(),
    );
  }
  Ok(found)
}

pub fn insert_rows(
  db: &rocksdb::DB,
  addrs: &[String],
  txids: &[Vec<u8>],
  block_heights: &[i32],
  block_positions: &[i32],
  tip_height: u32,
  tip_hash: [u8; 32],
) -> Result<(), DaemonError> {
  let data_cf = db
    .cf_handle(store::CF_DATA)
    .ok_or_else(|| DaemonError::Db("address history store data CF missing".to_owned()))?;
  let meta_cf = db
    .cf_handle(store::CF_META)
    .ok_or_else(|| DaemonError::Db("address history store meta CF missing".to_owned()))?;
  let mut batch = rocksdb::WriteBatch::default();
  store::mark_recovery_state(db, RecoveryState::Rebuilding, Some(tip_height))?;
  for (((addr, txid), block_height), block_pos) in addrs
    .iter()
    .zip(txids.iter())
    .zip(block_heights.iter().copied())
    .zip(block_positions.iter().copied())
  {
    batch.put_cf(data_cf, key(addr, block_height, block_pos, txid), []);
  }
  store::set_clean(&mut batch, meta_cf, Some(tip_height), Some(tip_hash));
  db.write(batch)
    .map_err(|e| DaemonError::Db(format!("write address history store rows: {e}")))?;
  Ok(())
}

pub fn connect_block_from_raw(
  db: &rocksdb::DB,
  prevout_db: &rocksdb::DB,
  block: &BlockCtx,
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Result<(), DaemonError> {
  let mut addrs = Vec::new();
  let mut txids = Vec::new();
  let mut heights = Vec::new();
  let mut positions = Vec::new();
  let block_height = block.height() as i32;
  for (pos, transaction) in block.block().transactions.iter().enumerate() {
    let txid = block
      .txid(pos)
      .ok_or_else(|| DaemonError::Index(format!("tx {pos} missing txid at {}", block.height())))?;
    let block_pos = pos as i32;
    let mut touched = HashSet::new();

    for output in &transaction.outputs {
      if let Some(addr) =
        dash_script::derive_address(output.script_pubkey.as_bytes(), p2pkh_version, p2sh_version)
      {
        touched.insert(addr);
      }
    }

    let prev_txids: Vec<Vec<u8>> = transaction
      .inputs
      .iter()
      .filter(|input| !input.prevout.hash.is_null())
      .map(|input| input.prevout.hash.as_bytes().to_vec())
      .collect();
    let prev_vouts: Vec<i32> = transaction
      .inputs
      .iter()
      .filter(|input| !input.prevout.hash.is_null())
      .map(|input| input.prevout.index as i32)
      .collect();
    for row in prevout::load_rows(prevout_db, &prev_txids, &prev_vouts)? {
      if let Some(addr) = row.prev_addr {
        touched.insert(addr);
      }
    }

    let mut ordered = touched.into_iter().collect::<Vec<_>>();
    ordered.sort_unstable();
    for addr in ordered {
      addrs.push(addr);
      txids.push(txid.to_vec());
      heights.push(block_height);
      positions.push(block_pos);
    }
  }

  insert_rows(
    db,
    &addrs,
    &txids,
    &heights,
    &positions,
    block.height(),
    block.block_hash(),
  )
}

pub fn disconnect_block(
  db: &rocksdb::DB,
  prevout_db: &rocksdb::DB,
  block: &BlockCtx,
  p2pkh_version: u8,
  p2sh_version: u8,
  next_tip: Option<u32>,
  next_hash: Option<[u8; 32]>,
) -> Result<(), DaemonError> {
  let data_cf = db
    .cf_handle(store::CF_DATA)
    .ok_or_else(|| DaemonError::Db("address history store data CF missing".to_owned()))?;
  let meta_cf = db
    .cf_handle(store::CF_META)
    .ok_or_else(|| DaemonError::Db("address history store meta CF missing".to_owned()))?;
  let mut batch = rocksdb::WriteBatch::default();
  store::mark_recovery_state(db, RecoveryState::Disconnecting, Some(block.height()))?;

  let block_height = block.height() as i32;
  for (pos, transaction) in block.block().transactions.iter().enumerate().rev() {
    let txid = block
      .txid(pos)
      .ok_or_else(|| DaemonError::Index(format!("tx {pos} missing txid at {}", block.height())))?;
    let block_pos = pos as i32;
    let mut touched = HashSet::new();

    for output in &transaction.outputs {
      if let Some(addr) =
        dash_script::derive_address(output.script_pubkey.as_bytes(), p2pkh_version, p2sh_version)
      {
        touched.insert(addr);
      }
    }
    let prev_txids: Vec<Vec<u8>> = transaction
      .inputs
      .iter()
      .filter(|input| !input.prevout.hash.is_null())
      .map(|input| input.prevout.hash.as_bytes().to_vec())
      .collect();
    let prev_vouts: Vec<i32> = transaction
      .inputs
      .iter()
      .filter(|input| !input.prevout.hash.is_null())
      .map(|input| input.prevout.index as i32)
      .collect();
    for row in prevout::load_rows(prevout_db, &prev_txids, &prev_vouts)? {
      if let Some(addr) = row.prev_addr {
        touched.insert(addr);
      }
    }
    for addr in touched {
      batch.delete_cf(data_cf, key(&addr, block_height, block_pos, txid));
    }
  }

  store::set_clean(&mut batch, meta_cf, next_tip, next_hash);
  db.write(batch)
    .map_err(|e| DaemonError::Db(format!("write address history store disconnect batch: {e}")))?;
  Ok(())
}
