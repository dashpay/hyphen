//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Prevout lookup secondary store.

use std::collections::HashMap;

use dash_primitives::script;

use crate::error::DaemonError;
use crate::index::BlockCtx;
use crate::store::{self, RecoveryState};

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize, PartialEq, Eq)]
pub struct PrevoutValue {
  pub output_height: i32,
  pub address: Option<String>,
  pub value: i64,
  pub spent_txid: Option<Vec<u8>>,
  pub spent_vin: Option<i32>,
  pub spent_height: Option<i32>,
}

pub(crate) fn encode_value(value: &PrevoutValue) -> Result<Vec<u8>, DaemonError> {
  let address_bytes = value.address.as_deref().map(str::as_bytes).unwrap_or(&[]);
  let spent_txid = value.spent_txid.as_deref().unwrap_or(&[]);
  let mut encoded =
    Vec::with_capacity(4 + 1 + 4 + address_bytes.len() + 8 + 1 + spent_txid.len() + 1 + 4 + 1 + 4);
  encoded.extend_from_slice(&value.output_height.to_be_bytes());
  encoded.push(u8::from(value.address.is_some()));
  encoded.extend_from_slice(&(address_bytes.len() as u32).to_be_bytes());
  encoded.extend_from_slice(address_bytes);
  encoded.extend_from_slice(&value.value.to_be_bytes());
  encoded.push(u8::from(value.spent_txid.is_some()));
  if value.spent_txid.is_some() {
    encoded.extend_from_slice(spent_txid);
  }
  encoded.push(u8::from(value.spent_vin.is_some()));
  if let Some(vin) = value.spent_vin {
    encoded.extend_from_slice(&vin.to_be_bytes());
  }
  encoded.push(u8::from(value.spent_height.is_some()));
  if let Some(height) = value.spent_height {
    encoded.extend_from_slice(&height.to_be_bytes());
  }
  Ok(encoded)
}

pub(crate) fn decode_value(raw: &[u8]) -> Result<PrevoutValue, DaemonError> {
  fn take<'a>(raw: &'a [u8], offset: &mut usize, len: usize) -> Result<&'a [u8], DaemonError> {
    let end = offset.saturating_add(len);
    if end > raw.len() {
      return Err(DaemonError::Db(
        "decode prevout store row: truncated value".to_owned(),
      ));
    }
    let slice = &raw[*offset..end];
    *offset = end;
    Ok(slice)
  }

  let mut offset = 0;
  let output_height = i32::from_be_bytes(
    take(raw, &mut offset, 4)?
      .try_into()
      .map_err(|_| DaemonError::Db("decode prevout store row: output height".to_owned()))?,
  );
  let has_address = take(raw, &mut offset, 1)?[0] != 0;
  let address_len = u32::from_be_bytes(
    take(raw, &mut offset, 4)?
      .try_into()
      .map_err(|_| DaemonError::Db("decode prevout store row: address length".to_owned()))?,
  ) as usize;
  let address = if has_address {
    Some(
      std::str::from_utf8(take(raw, &mut offset, address_len)?)
        .map_err(|e| DaemonError::Db(format!("decode prevout store address utf8: {e}")))?
        .to_owned(),
    )
  } else {
    let _ = take(raw, &mut offset, address_len)?;
    None
  };
  let value = i64::from_be_bytes(
    take(raw, &mut offset, 8)?
      .try_into()
      .map_err(|_| DaemonError::Db("decode prevout store row: value".to_owned()))?,
  );
  let spent_txid = if take(raw, &mut offset, 1)?[0] != 0 {
    Some(take(raw, &mut offset, 32)?.to_vec())
  } else {
    None
  };
  let spent_vin = if take(raw, &mut offset, 1)?[0] != 0 {
    Some(i32::from_be_bytes(
      take(raw, &mut offset, 4)?
        .try_into()
        .map_err(|_| DaemonError::Db("decode prevout store row: spent vin".to_owned()))?,
    ))
  } else {
    None
  };
  let spent_height = if take(raw, &mut offset, 1)?[0] != 0 {
    Some(i32::from_be_bytes(
      take(raw, &mut offset, 4)?
        .try_into()
        .map_err(|_| DaemonError::Db("decode prevout store row: spent height".to_owned()))?,
    ))
  } else {
    None
  };
  if offset != raw.len() {
    return Err(DaemonError::Db(
      "decode prevout store row: trailing bytes".to_owned(),
    ));
  }
  Ok(PrevoutValue {
    output_height,
    address,
    value,
    spent_txid,
    spent_vin,
    spent_height,
  })
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrevoutRow {
  pub prev_txid: Vec<u8>,
  pub prev_vout: i32,
  pub prev_height: i32,
  pub prev_addr: Option<String>,
  pub prev_value: i64,
  pub prev_spent_txid: Option<Vec<u8>>,
  pub prev_spent_vin: Option<i32>,
  pub prev_spent_height: Option<i32>,
}

pub fn key(prev_txid: &[u8], prev_vout: i32) -> Vec<u8> {
  let mut key = Vec::with_capacity(36);
  key.extend_from_slice(prev_txid);
  key.extend_from_slice(&prev_vout.to_be_bytes());
  key
}

fn decode_key(raw: &[u8]) -> Result<(&[u8], i32), DaemonError> {
  if raw.len() != 36 {
    return Err(DaemonError::Db(format!(
      "decode prevout store key: expected 36 bytes, got {}",
      raw.len()
    )));
  }
  let prev_vout = i32::from_be_bytes(
    raw[32..36]
      .try_into()
      .map_err(|_| DaemonError::Db("decode prevout store key vout".to_owned()))?,
  );
  Ok((&raw[..32], prev_vout))
}

fn load_values_by_key(
  db: &rocksdb::DB,
  prev_keys: &[Vec<u8>],
  sorted_input: bool,
) -> Result<Vec<PrevoutValue>, DaemonError> {
  if prev_keys.is_empty() {
    return Ok(Vec::new());
  }
  let data_cf = db
    .cf_handle(store::CF_DATA)
    .ok_or_else(|| DaemonError::Db("prevout store data CF missing".to_owned()))?;
  let rows = db.batched_multi_get_cf(
    data_cf,
    prev_keys.iter().map(std::vec::Vec::as_slice),
    sorted_input,
  );
  let mut values = Vec::with_capacity(rows.len());
  for (prev_key, raw) in prev_keys.iter().zip(rows) {
    let raw = raw.map_err(|e| DaemonError::Db(format!("read prevout store row: {e}")))?;
    let Some(raw) = raw else {
      let (prev_txid, prev_vout) = decode_key(prev_key)?;
      return Err(DaemonError::Index(format!(
        "missing prevout store row for {}:{}",
        hex::encode(prev_txid),
        prev_vout
      )));
    };
    values.push(decode_value(&raw)?);
  }
  Ok(values)
}

fn load_rows_with_ordering(
  db: &rocksdb::DB,
  prev_txids: &[Vec<u8>],
  prev_vouts: &[i32],
  sorted_input: bool,
) -> Result<Vec<PrevoutRow>, DaemonError> {
  let prev_keys = prev_txids
    .iter()
    .zip(prev_vouts.iter().copied())
    .map(|(prev_txid, prev_vout)| key(prev_txid, prev_vout))
    .collect::<Vec<_>>();
  let values = load_values_by_key(db, &prev_keys, sorted_input)?;
  let mut rows = Vec::with_capacity(values.len());
  for ((prev_txid, prev_vout), value) in prev_txids
    .iter()
    .zip(prev_vouts.iter().copied())
    .zip(values)
  {
    rows.push(PrevoutRow {
      prev_txid: prev_txid.clone(),
      prev_vout,
      prev_height: value.output_height,
      prev_addr: value.address,
      prev_value: value.value,
      prev_spent_txid: value.spent_txid,
      prev_spent_vin: value.spent_vin,
      prev_spent_height: value.spent_height,
    });
  }
  Ok(rows)
}

pub fn load_rows(
  db: &rocksdb::DB,
  prev_txids: &[Vec<u8>],
  prev_vouts: &[i32],
) -> Result<Vec<PrevoutRow>, DaemonError> {
  load_rows_with_ordering(db, prev_txids, prev_vouts, false)
}

pub fn load_rows_sorted(
  db: &rocksdb::DB,
  prev_txids: &[Vec<u8>],
  prev_vouts: &[i32],
) -> Result<Vec<PrevoutRow>, DaemonError> {
  load_rows_with_ordering(db, prev_txids, prev_vouts, true)
}

pub fn load_rows_interruptible<F>(
  db: &rocksdb::DB,
  prev_txids: &[Vec<u8>],
  prev_vouts: &[i32],
  window_size: usize,
  mut interrupt_requested: F,
) -> Result<Vec<PrevoutRow>, DaemonError>
where
  F: FnMut() -> bool,
{
  let window_size = window_size.max(1);
  let mut rows = Vec::with_capacity(prev_txids.len());
  for start in (0..prev_txids.len()).step_by(window_size) {
    if interrupt_requested() {
      return Err(DaemonError::Index("forward run interrupted".to_owned()));
    }
    let end = (start + window_size).min(prev_txids.len());
    rows.extend(load_rows_sorted(
      db,
      &prev_txids[start..end],
      &prev_vouts[start..end],
    )?);
  }
  Ok(rows)
}

pub fn connect_block(
  db: &rocksdb::DB,
  block: &BlockCtx,
  p2pkh_version: u8,
  p2sh_version: u8,
) -> Result<(), DaemonError> {
  let data_cf = db
    .cf_handle(store::CF_DATA)
    .ok_or_else(|| DaemonError::Db("prevout store data CF missing".to_owned()))?;
  let meta_cf = db
    .cf_handle(store::CF_META)
    .ok_or_else(|| DaemonError::Db("prevout store meta CF missing".to_owned()))?;
  let mut batch = rocksdb::WriteBatch::default();
  let block_height = block.height() as i32;
  store::mark_recovery_state(db, RecoveryState::Rebuilding, Some(block.height()))?;

  let mut rows_by_key: HashMap<Vec<u8>, PrevoutValue> = HashMap::new();
  for (pos, transaction) in block.block().transactions.iter().enumerate() {
    let txid = block
      .txid(pos)
      .ok_or_else(|| DaemonError::Index(format!("tx {pos} missing txid at {}", block.height())))?;
    for (vout, output) in transaction.outputs.iter().enumerate() {
      let prev_key = key(txid, vout as i32);
      let value = PrevoutValue {
        output_height: block_height,
        address: script::derive_address(
          output.script_pubkey.as_bytes(),
          p2pkh_version,
          p2sh_version,
        ),
        value: output.value,
        spent_txid: None,
        spent_vin: None,
        spent_height: None,
      };
      batch.put_cf(data_cf, &prev_key, encode_value(&value)?);
      rows_by_key.insert(prev_key, value);
    }
  }

  let mut missing_prev_keys = Vec::new();
  for transaction in &block.block().transactions {
    for input in &transaction.inputs {
      if input.prevout.hash.0 == [0u8; 32] {
        continue;
      }
      let prev_key = key(input.prevout.hash.as_bytes(), input.prevout.index as i32);
      if !rows_by_key.contains_key(&prev_key) {
        missing_prev_keys.push(prev_key);
      }
    }
  }
  if !missing_prev_keys.is_empty() {
    for (prev_key, value) in
      missing_prev_keys
        .iter()
        .zip(load_values_by_key(db, &missing_prev_keys, false)?)
    {
      rows_by_key.entry(prev_key.clone()).or_insert(value);
    }
  }

  for (pos, transaction) in block.block().transactions.iter().enumerate() {
    let spending_txid = block
      .txid(pos)
      .ok_or_else(|| DaemonError::Index(format!("tx {pos} missing txid at {}", block.height())))?;
    for (vin, input) in transaction.inputs.iter().enumerate() {
      if input.prevout.hash.0 == [0u8; 32] {
        continue;
      }
      let prev_key = key(input.prevout.hash.as_bytes(), input.prevout.index as i32);
      let value = rows_by_key
        .get_mut(&prev_key)
        .ok_or_else(|| DaemonError::Db("prevout spend row cache missing".to_owned()))?;
      value.spent_txid = Some(spending_txid.to_vec());
      value.spent_vin = Some(vin as i32);
      value.spent_height = Some(block_height);
    }
  }
  for (prev_key, value) in rows_by_key {
    batch.put_cf(data_cf, prev_key, encode_value(&value)?);
  }

  store::set_clean(
    &mut batch,
    meta_cf,
    Some(block.height()),
    Some(block.block_hash()),
  );
  db.write(batch)
    .map_err(|e| DaemonError::Db(format!("write prevout store connect batch: {e}")))?;
  Ok(())
}

pub fn disconnect_block(
  db: &rocksdb::DB,
  block: &BlockCtx,
  next_tip: Option<u32>,
  next_hash: Option<[u8; 32]>,
) -> Result<(), DaemonError> {
  let data_cf = db
    .cf_handle(store::CF_DATA)
    .ok_or_else(|| DaemonError::Db("prevout store data CF missing".to_owned()))?;
  let meta_cf = db
    .cf_handle(store::CF_META)
    .ok_or_else(|| DaemonError::Db("prevout store meta CF missing".to_owned()))?;
  let mut batch = rocksdb::WriteBatch::default();
  store::mark_recovery_state(db, RecoveryState::Disconnecting, Some(block.height()))?;
  let mut prev_keys = Vec::new();
  for transaction in block.block().transactions.iter().rev() {
    for input in transaction.inputs.iter().rev() {
      if input.prevout.hash.0 == [0u8; 32] {
        continue;
      }
      prev_keys.push(key(
        input.prevout.hash.as_bytes(),
        input.prevout.index as i32,
      ));
    }
  }
  let prev_values = load_values_by_key(db, &prev_keys, false)?;
  let mut rows_by_key = HashMap::with_capacity(prev_keys.len());
  for (prev_key, value) in prev_keys.into_iter().zip(prev_values) {
    rows_by_key.insert(prev_key, value);
  }

  for (pos, transaction) in block.block().transactions.iter().enumerate().rev() {
    for (vout, _) in transaction.outputs.iter().enumerate() {
      let txid = block.txid(pos).ok_or_else(|| {
        DaemonError::Index(format!("tx {pos} missing txid at {}", block.height()))
      })?;
      batch.delete_cf(data_cf, key(txid, vout as i32));
    }
    for input in transaction.inputs.iter().rev() {
      if input.prevout.hash.0 == [0u8; 32] {
        continue;
      }
      let prev_key = key(input.prevout.hash.as_bytes(), input.prevout.index as i32);
      let mut value = rows_by_key.remove(&prev_key).ok_or_else(|| {
        DaemonError::Index(format!(
          "missing prevout store row for {}:{} during disconnect at {}",
          hex::encode(input.prevout.hash.0),
          input.prevout.index,
          block.height()
        ))
      })?;
      value.spent_txid = None;
      value.spent_vin = None;
      value.spent_height = None;
      batch.put_cf(data_cf, prev_key, encode_value(&value)?);
    }
  }

  store::set_clean(&mut batch, meta_cf, next_tip, next_hash);
  db.write(batch)
    .map_err(|e| DaemonError::Db(format!("write prevout store disconnect batch: {e}")))?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use super::*;
  use dash_primitives::hash;

  fn p2pkh_script(tag: u8) -> Vec<u8> {
    let mut script = vec![0x76, 0xA9, 0x14];
    script.extend(std::iter::repeat_n(tag, 20));
    script.extend_from_slice(&[0x88, 0xAC]);
    script
  }

  fn coinbase_tx(script: &[u8], nonce: u8, value: i64) -> Vec<u8> {
    let mut tx = Vec::new();
    tx.extend_from_slice(&1_i32.to_le_bytes());
    tx.push(0x01);
    tx.extend_from_slice(&[0x00; 32]);
    tx.extend_from_slice(&u32::MAX.to_le_bytes());
    tx.push(0x01);
    tx.push(nonce);
    tx.extend_from_slice(&u32::MAX.to_le_bytes());
    tx.push(0x01);
    tx.extend_from_slice(&value.to_le_bytes());
    tx.push(script.len() as u8);
    tx.extend_from_slice(script);
    tx.extend_from_slice(&0_u32.to_le_bytes());
    tx
  }

  fn spend_tx(prev_txid: [u8; 32], prev_vout: u32, script: &[u8], value: i64) -> Vec<u8> {
    let mut tx = Vec::new();
    tx.extend_from_slice(&1_i32.to_le_bytes());
    tx.push(0x01);
    tx.extend_from_slice(&prev_txid);
    tx.extend_from_slice(&prev_vout.to_le_bytes());
    tx.push(0x00);
    tx.extend_from_slice(&u32::MAX.to_le_bytes());
    tx.push(0x01);
    tx.extend_from_slice(&value.to_le_bytes());
    tx.push(script.len() as u8);
    tx.extend_from_slice(script);
    tx.extend_from_slice(&0_u32.to_le_bytes());
    tx
  }

  fn same_block_spend_bytes() -> Vec<u8> {
    let first = coinbase_tx(&p2pkh_script(0x11), 0x2A, 50_0000_0000);
    let first_txid = hash::tx_hash(&first).0;
    let second = spend_tx(first_txid, 0, &p2pkh_script(0x22), 49_9999_0000);

    let mut raw = Vec::new();
    raw.extend_from_slice(&4_i32.to_le_bytes());
    raw.extend_from_slice(&[0xAA; 32]);
    raw.extend_from_slice(&[0xBB; 32]);
    raw.extend_from_slice(&1_700_000_000_u32.to_le_bytes());
    raw.extend_from_slice(&0x1A0F_FFFF_u32.to_le_bytes());
    raw.extend_from_slice(&7_u32.to_le_bytes());
    raw.push(0x02);
    raw.extend_from_slice(&first);
    raw.extend_from_slice(&second);
    raw
  }

  #[test]
  fn connect_block_resolves_prevouts_created_earlier_in_same_block(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let datadir = crate::tests::temp_datadir("hyphen-prevout-same-block");
    let store = crate::store::open(
      &datadir,
      crate::store::StoreId::PrevoutLookup,
      &crate::config::ChainConfig::default(),
      None,
    )?;
    let db = store.db();
    let block = crate::index::BlockCtx::from_raw(0, same_block_spend_bytes())?;

    connect_block(db, &block, 0x4c, 0x10)?;

    let first_txid = block.txid(0).ok_or("missing first txid")?;
    let second_txid = block.txid(1).ok_or("missing second txid")?;
    let rows = load_rows(db, &[first_txid.to_vec()], &[0])?;
    assert_eq!(rows.len(), 1);
    assert_eq!(
      rows[0].prev_spent_txid.as_deref(),
      Some(second_txid.as_slice())
    );
    assert_eq!(rows[0].prev_spent_vin, Some(0));
    assert_eq!(rows[0].prev_spent_height, Some(0));

    std::fs::remove_dir_all(datadir)?;
    Ok(())
  }

  #[test]
  fn prevout_value_binary_codec_round_trips() -> Result<(), Box<dyn std::error::Error>> {
    let value = PrevoutValue {
      output_height: 4,
      address: Some("Xcodec111111111111111111111111111".to_owned()),
      value: 42,
      spent_txid: Some(vec![7_u8; 32]),
      spent_vin: Some(3),
      spent_height: Some(9),
    };
    let encoded = encode_value(&value)?;
    let decoded = decode_value(&encoded)?;
    assert_eq!(decoded, value);
    Ok(())
  }

  #[test]
  fn disconnect_block_rejects_missing_prevout_rows() -> Result<(), Box<dyn std::error::Error>> {
    let datadir = crate::tests::temp_datadir("hyphen-prevout-disconnect-missing");
    let store = crate::store::open(
      &datadir,
      crate::store::StoreId::PrevoutLookup,
      &crate::config::ChainConfig::default(),
      None,
    )?;
    let db = store.db();
    let block = crate::index::BlockCtx::from_raw(0, same_block_spend_bytes())?;

    connect_block(db, &block, 0x4c, 0x10)?;

    let data_cf = db
      .cf_handle(crate::store::CF_DATA)
      .ok_or("prevout store data CF missing")?;
    let first_txid = block.txid(0).ok_or("missing first txid")?;
    db.delete_cf(data_cf, key(first_txid, 0))?;

    let error = disconnect_block(db, &block, None, None)
      .expect_err("disconnect should fail closed when a spent prevout row is missing");
    assert!(
      error.to_string().contains("missing prevout store row"),
      "unexpected error: {error}"
    );

    std::fs::remove_dir_all(datadir)?;
    Ok(())
  }

  #[test]
  fn load_rows_reads_bulk_prevout_rows() -> Result<(), Box<dyn std::error::Error>> {
    let datadir = crate::tests::temp_datadir("hyphen-prevout-load-rows");
    let store = crate::store::open(
      &datadir,
      crate::store::StoreId::PrevoutLookup,
      &crate::config::ChainConfig::default(),
      None,
    )?;
    let db = store.db();
    let block = crate::index::BlockCtx::from_raw(0, same_block_spend_bytes())?;

    connect_block(db, &block, 0x4c, 0x10)?;

    let rows = load_rows(
      db,
      &[
        block.txid(0).ok_or("missing first txid")?.to_vec(),
        block.txid(1).ok_or("missing second txid")?.to_vec(),
      ],
      &[0, 0],
    )?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].prev_height, 0);
    assert_eq!(rows[1].prev_height, 0);

    std::fs::remove_dir_all(datadir)?;
    Ok(())
  }

  #[test]
  fn load_rows_interruptible_stops_between_windows() -> Result<(), Box<dyn std::error::Error>> {
    let datadir = crate::tests::temp_datadir("hyphen-prevout-interruptible");
    let store = crate::store::open(
      &datadir,
      crate::store::StoreId::PrevoutLookup,
      &crate::config::ChainConfig::default(),
      None,
    )?;
    let db = store.db();
    let data_cf = db
      .cf_handle(crate::store::CF_DATA)
      .ok_or("prevout data CF missing")?;
    let meta_cf = db
      .cf_handle(crate::store::CF_META)
      .ok_or("prevout meta CF missing")?;
    let mut batch = rocksdb::WriteBatch::default();
    let mut prev_txids = Vec::new();
    let mut prev_vouts = Vec::new();
    for index in 0..4_i32 {
      let txid = vec![u8::try_from(index + 1)?; 32];
      batch.put_cf(
        data_cf,
        key(&txid, index),
        encode_value(&PrevoutValue {
          output_height: 1,
          address: Some(format!("Xinterrupt{index}")),
          value: 42,
          spent_txid: None,
          spent_vin: None,
          spent_height: None,
        })?,
      );
      prev_txids.push(txid);
      prev_vouts.push(index);
    }
    crate::store::set_clean(&mut batch, meta_cf, Some(1), Some([0x22; 32]));
    db.write(batch)?;

    let mut checks = 0_u32;
    let err = load_rows_interruptible(db, &prev_txids, &prev_vouts, 2, || {
      checks += 1;
      checks > 1
    })
    .expect_err("interrupt should stop between windows");
    assert!(err.to_string().contains("forward run interrupted"));

    std::fs::remove_dir_all(datadir)?;
    Ok(())
  }
}
