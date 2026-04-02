//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block-scoped transaction write planning.

use std::sync::Mutex;

use dash_primitives::script;
use dash_primitives::tx::transaction::Transaction;
use dash_primitives::types::tx::TxType;

use crate::error::DaemonError;
use crate::index::batch::BatchSizer;
use crate::index::runner::ApplyStagePlanSubstage;

use super::{sql, BlockCtx};

const DEFAULT_TX_WRITE_CHUNK_SIZE: usize = 1_000;
const TX_OUTPUT_FINAL_APPLY_CHUNK_SIZE: usize = 512;

pub(crate) struct TransactionBatchPlan {
  last_height: u32,
  last_hash: [u8; 32],
  chunk_size: usize,
  transactions: TransactionRows,
  inputs: InputRows,
  outputs: OutputRows,
  special_txs: Vec<SpecialTxRow>,
}

impl TransactionBatchPlan {
  pub(crate) fn height(&self) -> u32 {
    self.last_height
  }

  pub(crate) fn build(
    block: &BlockCtx,
    p2pkh_version: u8,
    p2sh_version: u8,
    batch_sizer: &Mutex<BatchSizer>,
  ) -> Result<Self, DaemonError> {
    let mut transactions = TransactionRows::default();
    let mut inputs = InputRows::default();
    let mut outputs = OutputRows::default();
    let mut special_txs = Vec::new();
    let block_height = block.height() as i32;

    for (pos, transaction) in block.block().transactions.iter().enumerate() {
      let (start, end) = block.tx_byte_range(pos).ok_or_else(|| {
        DaemonError::Index(format!("tx {pos} missing byte range at {}", block.height()))
      })?;
      let txid = block
        .txid(pos)
        .ok_or_else(|| DaemonError::Index(format!("tx {pos} missing txid at {}", block.height())))?
        .to_vec();
      transactions.push(
        transaction,
        txid.clone(),
        block_height,
        pos as i32,
        end - start,
        block.height(),
      )?;
      inputs.push(transaction, &txid);
      outputs.push(
        transaction,
        &txid,
        block_height,
        p2pkh_version,
        p2sh_version,
      );
      special_txs.extend(SpecialTxRow::decode(transaction, txid, block.height(), pos));
    }

    Ok(Self {
      last_height: block.height(),
      last_hash: block.block_hash(),
      chunk_size: current_chunk_size(batch_sizer),
      transactions,
      inputs,
      outputs,
      special_txs,
    })
  }

  pub(crate) async fn apply(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    reporter: Option<&crate::index::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    self
      .transactions
      .insert(tx, self.last_height, self.chunk_size, reporter)
      .await?;
    self
      .inputs
      .insert(tx, self.last_height, self.chunk_size, reporter)
      .await?;
    self
      .outputs
      .insert(tx, self.last_height, self.chunk_size, reporter)
      .await?;
    if !self.special_txs.is_empty() {
      let chunk_total = self.special_txs.len().div_ceil(self.chunk_size);
      for (index, chunk) in self.special_txs.chunks(self.chunk_size).enumerate() {
        if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
          return Err(DaemonError::Index("forward run interrupted".to_owned()));
        }
        if let Some(reporter) = reporter {
          reporter.activity(
            ApplyStagePlanSubstage::TransactionSpecialPayloads,
            Some((index + 1) as u32),
            Some(chunk_total as u32),
          );
        }
        let txids: Vec<&[u8]> = chunk.iter().map(|row| row.txid.as_slice()).collect();
        let heights: Vec<i32> = chunk.iter().map(|row| row.block_height).collect();
        let tx_types: Vec<crate::sql::types::TxTypeDb> =
          chunk.iter().map(|row| row.tx_type).collect();
        let pro_tx_hashes: Vec<Option<&[u8]>> =
          chunk.iter().map(|row| row.pro_tx_hash.as_deref()).collect();
        let payloads: Vec<&serde_json::Value> = chunk.iter().map(|row| &row.payload).collect();
        sql::insert_special_tx_payloads(tx, &heights, &txids, &tx_types, &pro_tx_hashes, &payloads)
          .await?;
        if let Some(reporter) = reporter {
          reporter.progress(
            ApplyStagePlanSubstage::TransactionSpecialPayloads,
            Some((index + 1) as u32),
            Some(chunk_total as u32),
          );
        }
      }
    }
    Ok(())
  }

  pub(crate) fn apply_prevout_store(
    &self,
    db: &rocksdb::DB,
    _p2pkh_version: u8,
    _p2sh_version: u8,
  ) -> Result<(), DaemonError> {
    let data_cf = db
      .cf_handle(crate::store::CF_DATA)
      .ok_or_else(|| DaemonError::Db("prevout store data CF missing".to_owned()))?;
    let meta_cf = db
      .cf_handle(crate::store::CF_META)
      .ok_or_else(|| DaemonError::Db("prevout store meta CF missing".to_owned()))?;
    let mut batch = rocksdb::WriteBatch::default();
    let mut rows_by_key = std::collections::HashMap::new();
    crate::store::mark_recovery_state(
      db,
      crate::store::RecoveryState::Rebuilding,
      Some(self.last_height),
    )?;
    for ((((((txid, vout), value), addr), _height), _script), _addr_type) in self
      .outputs
      .txids
      .iter()
      .zip(self.outputs.vouts.iter().copied())
      .zip(self.outputs.values.iter().copied())
      .zip(self.outputs.addrs.iter())
      .zip(self.outputs.heights.iter().copied())
      .zip(self.outputs.scripts.iter())
      .zip(self.outputs.addr_types.iter())
    {
      let prev_key = crate::store::prevout::key(txid, vout);
      let row = crate::store::prevout::PrevoutValue {
        output_height: self.last_height as i32,
        address: addr.clone(),
        value,
        spent_txid: None,
        spent_vin: None,
        spent_height: None,
      };
      batch.put_cf(
        data_cf,
        &prev_key,
        crate::store::prevout::encode_value(&row)?,
      );
      rows_by_key.insert(prev_key, row);
    }
    let mut missing_prev_keys = Vec::new();
    let mut missing_prev_txids = Vec::new();
    let mut missing_prev_vouts = Vec::new();
    for (prev_txid, prev_vout) in self
      .inputs
      .prev_txids
      .iter()
      .zip(self.inputs.prev_vouts.iter())
    {
      let (Some(prev_txid), Some(prev_vout)) = (prev_txid, prev_vout) else {
        continue;
      };
      let prev_key = crate::store::prevout::key(prev_txid, *prev_vout);
      if !rows_by_key.contains_key(&prev_key) {
        missing_prev_keys.push(prev_key);
        missing_prev_txids.push(prev_txid.clone());
        missing_prev_vouts.push(*prev_vout);
      }
    }
    if !missing_prev_keys.is_empty() {
      for (prev_key, row) in missing_prev_keys
        .iter()
        .zip(crate::store::prevout::load_rows(
          db,
          &missing_prev_txids,
          &missing_prev_vouts,
        )?)
      {
        rows_by_key
          .entry(prev_key.clone())
          .or_insert(crate::store::prevout::PrevoutValue {
            output_height: row.prev_height,
            address: row.prev_addr,
            value: row.prev_value,
            spent_txid: row.prev_spent_txid,
            spent_vin: row.prev_spent_vin,
            spent_height: row.prev_spent_height,
          });
      }
    }
    for (((txid, vin), prev_txid), prev_vout) in self
      .inputs
      .txids
      .iter()
      .zip(self.inputs.vins.iter().copied())
      .zip(self.inputs.prev_txids.iter())
      .zip(self.inputs.prev_vouts.iter())
    {
      let (Some(prev_txid), Some(prev_vout)) = (prev_txid, prev_vout) else {
        continue;
      };
      let prev_key = crate::store::prevout::key(prev_txid, *prev_vout);
      let row = rows_by_key
        .get_mut(&prev_key)
        .ok_or_else(|| DaemonError::Db("prevout store row cache missing".to_owned()))?;
      row.spent_txid = Some(txid.clone());
      row.spent_vin = Some(vin);
      row.spent_height = Some(self.last_height as i32);
      batch.put_cf(data_cf, prev_key, crate::store::prevout::encode_value(row)?);
    }
    crate::store::set_clean(
      &mut batch,
      meta_cf,
      Some(self.last_height),
      Some(self.last_hash),
    );
    db.write(batch)
      .map_err(|e| DaemonError::Db(format!("write prevout store connect batch: {e}")))?;
    Ok(())
  }
}

#[derive(Default)]
struct TransactionRows {
  txids: Vec<Vec<u8>>,
  heights: Vec<i32>,
  positions: Vec<i32>,
  versions: Vec<i32>,
  types: Vec<String>,
  lock_times: Vec<i32>,
  sizes: Vec<i32>,
  values_out: Vec<i64>,
  input_counts: Vec<i32>,
  output_counts: Vec<i32>,
}

impl TransactionRows {
  fn push(
    &mut self,
    transaction: &Transaction,
    txid: Vec<u8>,
    block_height: i32,
    position: i32,
    byte_len: usize,
    height: u32,
  ) -> Result<(), DaemonError> {
    let value_out = transaction.outputs.iter().map(|output| output.value).sum();
    self.txids.push(txid);
    self.heights.push(block_height);
    self.positions.push(position);
    self.versions.push(transaction.version.into());
    self
      .types
      .push(crate::tx::tx_type_str(transaction.tx_type).to_owned());
    self
      .lock_times
      .push(i32::try_from(transaction.lock_time).map_err(|_| {
        DaemonError::Index(format!("tx {position} lock_time exceeds i32 at {height}"))
      })?);
    self.sizes.push(byte_len as i32);
    self.values_out.push(value_out);
    self.input_counts.push(transaction.inputs.len() as i32);
    self.output_counts.push(transaction.outputs.len() as i32);
    Ok(())
  }

  async fn insert(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    last_height: u32,
    chunk_size: usize,
    reporter: Option<&crate::index::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    let chunk_total = self.txids.len().div_ceil(chunk_size);
    for (index, start) in (0..self.txids.len()).step_by(chunk_size).enumerate() {
      if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
        return Err(DaemonError::Index("forward run interrupted".to_owned()));
      }
      let end = (start + chunk_size).min(self.txids.len());
      if let Some(reporter) = reporter {
        reporter.activity(
          ApplyStagePlanSubstage::TransactionRows,
          Some((index + 1) as u32),
          Some(chunk_total as u32),
        );
      }
      sql::insert_transactions(
        tx,
        last_height,
        &self.txids[start..end],
        &self.heights[start..end],
        &self.positions[start..end],
        &self.versions[start..end],
        &self.types[start..end],
        &self.lock_times[start..end],
        &self.sizes[start..end],
        &self.values_out[start..end],
        &self.input_counts[start..end],
        &self.output_counts[start..end],
      )
      .await?;
      if let Some(reporter) = reporter {
        reporter.progress(
          ApplyStagePlanSubstage::TransactionRows,
          Some((index + 1) as u32),
          Some(chunk_total as u32),
        );
      }
    }
    Ok(())
  }
}

#[derive(Default)]
struct InputRows {
  txids: Vec<Vec<u8>>,
  vins: Vec<i32>,
  prev_txids: Vec<Option<Vec<u8>>>,
  prev_vouts: Vec<Option<i32>>,
  scripts: Vec<Vec<u8>>,
  sequences: Vec<i64>,
}

impl InputRows {
  fn push(&mut self, transaction: &Transaction, txid: &[u8]) {
    for (vin, input) in transaction.inputs.iter().enumerate() {
      self.txids.push(txid.to_vec());
      self.vins.push(vin as i32);
      if input.prevout.hash.0 == [0u8; 32] {
        self.prev_txids.push(None);
        self.prev_vouts.push(None);
      } else {
        self
          .prev_txids
          .push(Some(input.prevout.hash.as_bytes().to_vec()));
        self.prev_vouts.push(Some(input.prevout.index as i32));
      }
      self.scripts.push(input.script_sig.as_bytes().to_vec());
      self.sequences.push(input.sequence as i64);
    }
  }

  async fn insert(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    last_height: u32,
    chunk_size: usize,
    reporter: Option<&crate::index::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    let chunk_total = self.txids.len().div_ceil(chunk_size);
    for (index, start) in (0..self.txids.len()).step_by(chunk_size).enumerate() {
      if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
        return Err(DaemonError::Index("forward run interrupted".to_owned()));
      }
      let end = (start + chunk_size).min(self.txids.len());
      if let Some(reporter) = reporter {
        reporter.activity(
          ApplyStagePlanSubstage::TransactionInputs,
          Some((index + 1) as u32),
          Some(chunk_total as u32),
        );
      }
      sql::insert_tx_inputs(
        tx,
        last_height,
        &self.txids[start..end],
        &self.vins[start..end],
        &self.prev_txids[start..end],
        &self.prev_vouts[start..end],
        &self.scripts[start..end],
        &self.sequences[start..end],
      )
      .await?;
      if let Some(reporter) = reporter {
        reporter.progress(
          ApplyStagePlanSubstage::TransactionInputs,
          Some((index + 1) as u32),
          Some(chunk_total as u32),
        );
      }
    }
    Ok(())
  }
}

#[derive(Default)]
struct OutputRows {
  txids: Vec<Vec<u8>>,
  vouts: Vec<i32>,
  heights: Vec<i32>,
  values: Vec<i64>,
  scripts: Vec<Vec<u8>>,
  addrs: Vec<Option<String>>,
  addr_types: Vec<Option<String>>,
}

impl OutputRows {
  fn push(
    &mut self,
    transaction: &Transaction,
    txid: &[u8],
    height: i32,
    p2pkh_version: u8,
    p2sh_version: u8,
  ) {
    for (vout, output) in transaction.outputs.iter().enumerate() {
      let script_bytes = output.script_pubkey.as_bytes();
      self.txids.push(txid.to_vec());
      self.vouts.push(vout as i32);
      self.heights.push(height);
      self.values.push(output.value);
      self.scripts.push(script_bytes.to_vec());
      self.addrs.push(script::derive_address(
        script_bytes,
        p2pkh_version,
        p2sh_version,
      ));
      self
        .addr_types
        .push(match script::classify_script(script_bytes) {
          script::ScriptKind::P2pkh => Some("p2pkh".to_owned()),
          script::ScriptKind::P2sh => Some("p2sh".to_owned()),
          script::ScriptKind::P2pk => Some("p2pk".to_owned()),
          _ => None,
        });
    }
  }

  async fn insert(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    last_height: u32,
    chunk_size: usize,
    reporter: Option<&crate::index::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    sql::prepare_tx_outputs_staging(tx).await?;
    let stage_chunk_total = self.txids.len().div_ceil(chunk_size);
    for (index, start) in (0..self.txids.len()).step_by(chunk_size).enumerate() {
      if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
        return Err(DaemonError::Index("forward run interrupted".to_owned()));
      }
      let end = (start + chunk_size).min(self.txids.len());
      if let Some(reporter) = reporter {
        reporter.activity(
          ApplyStagePlanSubstage::TransactionOutputsStage,
          Some((index + 1) as u32),
          Some(stage_chunk_total as u32),
        );
      }
      sql::stage_tx_outputs(
        tx,
        &self.txids[start..end],
        &self.vouts[start..end],
        &self.heights[start..end],
        &self.values[start..end],
        &self.scripts[start..end],
        &self.addrs[start..end],
        &self.addr_types[start..end],
      )
      .await?;
      if let Some(reporter) = reporter {
        reporter.progress(
          ApplyStagePlanSubstage::TransactionOutputsStage,
          Some((index + 1) as u32),
          Some(stage_chunk_total as u32),
        );
      }
    }

    let staged_outputs = sql::staged_tx_outputs_count(tx).await? as usize;
    let apply_chunk_total = staged_outputs.div_ceil(TX_OUTPUT_FINAL_APPLY_CHUNK_SIZE);
    for (index, start) in (0..staged_outputs)
      .step_by(TX_OUTPUT_FINAL_APPLY_CHUNK_SIZE)
      .enumerate()
    {
      if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
        return Err(DaemonError::Index("forward run interrupted".to_owned()));
      }
      let end = (start + TX_OUTPUT_FINAL_APPLY_CHUNK_SIZE).min(staged_outputs);
      if let Some(reporter) = reporter {
        reporter.activity(
          ApplyStagePlanSubstage::TransactionOutputsApply,
          Some((index + 1) as u32),
          Some(apply_chunk_total as u32),
        );
      }
      sql::apply_staged_tx_outputs_chunk(tx, last_height, start as i64, end as i64).await?;
      if let Some(reporter) = reporter {
        reporter.progress(
          ApplyStagePlanSubstage::TransactionOutputsApply,
          Some((index + 1) as u32),
          Some(apply_chunk_total as u32),
        );
      }
    }
    Ok(())
  }
}

fn current_chunk_size(batch_sizer: &Mutex<BatchSizer>) -> usize {
  batch_sizer
    .lock()
    .map(|sizer| sizer.current.max(1) as usize)
    .unwrap_or(DEFAULT_TX_WRITE_CHUNK_SIZE)
}

struct SpecialTxRow {
  block_height: i32,
  txid: Vec<u8>,
  tx_type: crate::sql::types::TxTypeDb,
  pro_tx_hash: Option<Vec<u8>>,
  payload: serde_json::Value,
}

impl SpecialTxRow {
  fn decode(
    transaction: &Transaction,
    txid: Vec<u8>,
    height: u32,
    position: usize,
  ) -> Option<Self> {
    if matches!(transaction.tx_type, TxType::Normal | TxType::Unknown(_)) {
      return None;
    }

    match dash_primitives::tx::SpecialPayload::decode(
      transaction.tx_type,
      &transaction.extra_payload,
    ) {
      Ok(payload) => {
        let (pro_tx_hash, payload_json) = crate::tx::payload_for_storage(&payload, &txid);
        Some(Self {
          block_height: height as i32,
          txid,
          tx_type: transaction.tx_type.into(),
          pro_tx_hash,
          payload: payload_json,
        })
      }
      Err(error) => {
        tracing::warn!(
          height,
          pos = position,
          "failed to decode special tx payload: {error}"
        );
        None
      }
    }
  }
}

#[cfg(test)]
mod tests {
  use super::{OutputRows, TransactionBatchPlan};
  use crate::store::{self, StoreId};
  use dash_primitives::hash;
  use dash_primitives::tx::transaction::Transaction;
  use dash_primitives::types::common::{Script, TxHash};
  use dash_primitives::types::tx::{OutPoint, TxIn, TxOut, TxType};
  use std::sync::Mutex;

  fn sample_tx(script_pubkey: Vec<u8>) -> Transaction {
    Transaction {
      version: 3,
      tx_type: TxType::Normal,
      inputs: vec![TxIn {
        prevout: OutPoint {
          hash: TxHash([0u8; 32]),
          index: u32::MAX,
        },
        script_sig: Script(Vec::new()),
        sequence: u32::MAX,
      }],
      outputs: vec![TxOut {
        value: 42,
        script_pubkey: Script(script_pubkey),
      }],
      lock_time: 0,
      extra_payload: Vec::new(),
    }
  }

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
  fn output_rows_inline_address_and_type() {
    let mut rows = OutputRows::default();
    let script = vec![
      0x76, 0xa9, 0x14, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
      0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x88, 0xac,
    ];

    rows.push(&sample_tx(script.clone()), &[7u8; 32], 10, 76, 16);

    assert_eq!(rows.scripts, vec![script]);
    assert_eq!(rows.addr_types, vec![Some("p2pkh".to_owned())]);
    assert!(rows.addrs[0].is_some());
  }

  #[test]
  fn apply_prevout_store_marks_same_block_outputs_spent() -> Result<(), Box<dyn std::error::Error>>
  {
    let datadir = crate::tests::temp_datadir("hyphen-tx-batch-prevout");
    let store = store::open(
      &datadir,
      StoreId::PrevoutLookup,
      &crate::config::ChainConfig::default(),
      None,
    )?;
    let db = store.db();
    let block = crate::index::BlockCtx::from_raw(0, same_block_spend_bytes())?;
    let plan = TransactionBatchPlan::build(
      &block,
      crate::config::DEFAULT_P2PKH_VERSION,
      crate::config::DEFAULT_P2SH_VERSION,
      &Mutex::new(crate::index::batch::BatchSizer::new(1, 1, 1, 1)),
    )?;

    plan.apply_prevout_store(
      db,
      crate::config::DEFAULT_P2PKH_VERSION,
      crate::config::DEFAULT_P2SH_VERSION,
    )?;

    let first_txid = block.txid(0).ok_or("missing first txid")?;
    let second_txid = block.txid(1).ok_or("missing second txid")?;
    let rows = crate::store::prevout::load_rows(db, &[first_txid.to_vec()], &[0])?;
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
}
