//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Transaction records: the transaction itself, its inputs and outputs, its
//! special-tx payload blob, and the resolved prevout an input consumes.
//!
//! The per-row `INSERT`s that used to live on these records moved into the
//! base ingest stage, which loads a whole block's rows per table with one
//! `UNNEST` statement — a 5,000-input block costs a handful of round trips,
//! not tens of thousands. The records stay pure data carriers; only the rare
//! special-tx payload keeps its own insert here.

use super::sptx::HyDbVlEvent;
use super::PgTx;
use crate::error::{HyDbError, HyDbResult};
use crate::types::{HyDbAddrType, HyDbAddress, HyDbAmount, HyDbHash, HyDbOutpoint, HyDbTxType};

/// A decoded special-transaction payload, as structured JSON plus its indexing
/// key. The crate stores the JSON verbatim; the daemon owns its schema.
#[derive(Debug, Clone)]
pub struct HyDbPayload {
  pub protx_hash: Option<HyDbHash>,
  pub json: serde_json::Value,
}

/// One transaction's facts.
#[derive(Debug, Clone)]
pub struct HyDbTx {
  pub txid: HyDbHash,
  pub block_pos: u32,
  pub version: i32,
  pub tx_type: HyDbTxType,
  pub lock_time: u32,
  pub size: u32,
  pub value_out: HyDbAmount,
  pub inputs: Vec<HyDbTxIn>,
  pub outputs: Vec<HyDbTxOut>,
  /// Decoded special-tx payload, when this is a DIP2/DIP3 transaction.
  pub payload: Option<HyDbPayload>,
  /// Validator lifecycle events this transaction emits (ProReg/Up*/Rev).
  pub validator_events: Vec<HyDbVlEvent>,
}

impl HyDbTx {
  /// Insert the special-tx payload row, when present. Payloads are rare
  /// (validator/quorum transactions), so a per-row insert is fine.
  pub(crate) async fn insert_payload(&self, tx: &mut PgTx<'_>, height: i32) -> HyDbResult<()> {
    let Some(payload) = &self.payload else {
      return Ok(());
    };
    sqlx::query!(
      r#"INSERT INTO sptx_payloads (block_height, txid, tx_type, protx_hash, payload)
         VALUES ($1, $2, $3, $4, $5)"#,
      height,
      &self.txid.0[..],
      self.tx_type as _,
      payload.protx_hash.map(|h| h.0.to_vec()),
      payload.json,
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("insert payload", e))?;
    Ok(())
  }
}

/// One transaction input.
#[derive(Debug, Clone)]
pub struct HyDbTxIn {
  pub vin_index: u32,
  /// The outpoint consumed, or `None` for a coinbase input.
  pub prev_outpoint: Option<HyDbOutpoint>,
  pub coinbase: bool,
  pub script_sig: Vec<u8>,
  pub sequence: u32,
}

/// One transaction output.
#[derive(Debug, Clone)]
pub struct HyDbTxOut {
  pub vout: u32,
  pub value: HyDbAmount,
  pub script_pubkey: Vec<u8>,
  pub address: Option<HyDbAddress>,
  pub address_type: HyDbAddrType,
}

/// The resolved previous output a non-coinbase input consumes. The pipeline
/// looks it up in the UTXO set and threads it to the base stage so the input
/// row can record its inlined provenance (`prev_address`/`prev_value`).
#[derive(Clone)]
pub(crate) struct SpentOutput {
  pub address: Option<String>,
  pub value: i64,
}
