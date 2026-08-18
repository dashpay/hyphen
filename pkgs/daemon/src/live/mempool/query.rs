//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Query-time mempool enrichment for public REST surfaces.

use std::collections::HashMap;

use dash_primitives::Transaction as DecodedTransaction;
use hyphen_db::HyphenDb;
use serde_json::Value;

use crate::error::{DaemonError, RestErrorKind};
use crate::live::state::{LiveState, MempoolTx, MempoolTxInput, MempoolTxOutput};
use crate::rest::openapi::{
  TxDetail, TxInput, TxLive, TxOutput, TxScriptPubKey, TxScriptSig, TxTags,
};
use crate::utils::{self, HashEndian};

/// Decode and enrich one mempool transaction into an in-RAM [`MempoolTx`]. Input
/// provenance (prev address/value) is resolved from the storage engine's
/// committed outputs.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when the transaction cannot be decoded and
/// `DaemonError::Index` when a prevout lookup fails.
pub async fn normalize_tx(
  db: &HyphenDb,
  txid_bytes: &[u8],
  raw_tx: &[u8],
  p2pkh_version: u8,
  p2sh_version: u8,
  first_seen_at: i64,
  last_seen_at: i64,
) -> Result<MempoolTx, DaemonError> {
  let tx = decode_tx(raw_tx)?;
  let prevout_map = load_prevouts(db, &tx).await?;
  let value_out = tx
    .outputs
    .iter()
    .map(|output| output.value.to_sat() as i64)
    .sum::<i64>();
  let tx_type = crate::tx::tx_type_str(tx.tx_type).to_owned();
  let payload = decode_payload(&tx, txid_bytes);
  let fee = tx
    .inputs
    .iter()
    .filter(|input| !input.prevout.hash.is_null())
    .try_fold(0i64, |sum, input| {
      let key = (
        input.prevout.hash.as_bytes().to_vec(),
        input.prevout.index as i32,
      );
      prevout_map
        .get(&key)
        .and_then(|entry| entry.value)
        .map(|value| sum + value)
        .ok_or(())
    })
    .ok()
    .map(|value_in| value_in - value_out);

  let inputs = tx
    .inputs
    .iter()
    .enumerate()
    .map(|(index, input)| {
      let is_coinbase = input.prevout.hash.is_null();
      let key = (
        input.prevout.hash.as_bytes().to_vec(),
        input.prevout.index as i32,
      );
      let prev = prevout_map.get(&key);
      MempoolTxInput {
        vin_index: index as i32,
        sequence: i64::from(input.sequence),
        prev_txid: (!is_coinbase).then(|| input.prevout.hash.as_bytes().to_vec()),
        prev_vout: (!is_coinbase).then_some(input.prevout.index as i32),
        coinbase: is_coinbase.then(|| input.script_sig.0.clone()),
        script_sig: input.script_sig.0.clone(),
        prev_address: prev.and_then(|entry| entry.address.clone()),
        prev_value: prev.and_then(|entry| entry.value),
      }
    })
    .collect::<Vec<_>>();

  let outputs = tx
    .outputs
    .iter()
    .enumerate()
    .map(|(index, output)| MempoolTxOutput {
      vout_index: index as i32,
      value: output.value.to_sat() as i64,
      script_pubkey: output.script_pubkey.as_bytes().to_vec(),
      address: dash_script::derive_address(
        output.script_pubkey.as_bytes(),
        p2pkh_version,
        p2sh_version,
      ),
    })
    .collect::<Vec<_>>();

  Ok(MempoolTx {
    txid: txid_bytes.to_vec(),
    raw_tx: raw_tx.to_vec(),
    version: tx.version,
    tx_type,
    lock_time: i64::from(tx.lock_time),
    size: raw_tx.len() as i32,
    value_out,
    input_count: tx.inputs.len() as i32,
    output_count: tx.outputs.len() as i32,
    fee,
    payload: (!payload.is_null()).then_some(payload),
    inputs,
    outputs,
    first_seen_at,
    last_seen_at,
  })
}

/// Build a near-mined transaction detail directly from raw bytes, resolving
/// input provenance from the storage engine.
///
/// # Errors
///
/// Returns `DaemonError` when the transaction cannot be decoded, a prevout
/// lookup fails.
pub async fn build_tx_value(
  db: &HyphenDb,
  txid_hex: &str,
  txid_bytes: &[u8],
  raw_tx: &[u8],
  p2pkh_version: u8,
  p2sh_version: u8,
  _islocked: bool,
) -> Result<TxDetail, DaemonError> {
  let tx = decode_tx(raw_tx)?;
  let prevout_map = load_prevouts(db, &tx).await?;
  let value_out = tx
    .outputs
    .iter()
    .map(|output| output.value.to_sat() as i64)
    .sum::<i64>();
  let stored_tx_type = crate::tx::tx_type_str(tx.tx_type);
  let public_tx_type = crate::tx::public_tx_type(
    stored_tx_type,
    tx.outputs
      .iter()
      .map(|output| output.script_pubkey.as_bytes()),
  );
  let payload = decode_payload(&tx, txid_bytes);

  let vin = tx
    .inputs
    .iter()
    .enumerate()
    .map(|(index, input)| {
      let key = (
        input.prevout.hash.as_bytes().to_vec(),
        input.prevout.index as i32,
      );
      let prev = prevout_map.get(&key);
      TxInput {
        n: index as i32,
        script_sig: TxScriptSig {
          hex: hex::encode(&input.script_sig.0),
        },
        sequence: i64::from(input.sequence),
        txid: Some(utils::view_hex(
          input.prevout.hash.as_bytes(),
          HashEndian::Ser,
        )),
        vout: Some(input.prevout.index as i32),
        coinbase: None,
        address: prev.and_then(|entry| entry.address.clone()),
        value: prev.and_then(|entry| entry.value),
      }
    })
    .collect::<Vec<TxInput>>();
  let vout = tx
    .outputs
    .iter()
    .enumerate()
    .map(|(index, output)| TxOutput {
      n: index as i32,
      value: output.value.to_sat() as i64,
      script_pub_key: TxScriptPubKey {
        hex: hex::encode(&output.script_pubkey.0),
      },
      address: dash_script::derive_address(
        output.script_pubkey.as_bytes(),
        p2pkh_version,
        p2sh_version,
      ),
      spent: false,
    })
    .collect::<Vec<TxOutput>>();

  Ok(TxDetail {
    as_of_height: None,
    txid: txid_hex.to_owned(),
    block_height: None,
    block_pos: None,
    version: tx.version,
    tx_type: public_tx_type.to_owned(),
    lock_time: i64::from(tx.lock_time),
    size: raw_tx.len() as i32,
    value_out,
    input_count: tx.inputs.len() as i32,
    output_count: tx.outputs.len() as i32,
    fee: None,
    tags: TxTags {
      mined: false,
      coinbase: false,
    },
    live: TxLive::default(),
    payload: (!payload.is_null()).then_some(payload),
    vin,
    vout,
  })
}

/// Build a near-mined transaction detail from the in-RAM mempool tx, if held.
pub fn build_tx_value_from_persisted(
  live_state: &LiveState,
  txid_hex: &str,
  txid_bytes: &[u8],
  islocked: bool,
) -> Option<TxDetail> {
  let tx = live_state.mempool_tx(txid_bytes)?;
  Some(mempool_tx_value(txid_hex, &tx, islocked))
}

fn mempool_tx_value(txid_hex: &str, tx: &MempoolTx, _islocked: bool) -> TxDetail {
  let public_tx_type = crate::tx::public_tx_type(
    &tx.tx_type,
    tx.outputs
      .iter()
      .map(|output| output.script_pubkey.as_slice()),
  );
  let vin = tx
    .inputs
    .iter()
    .map(|input| TxInput {
      n: input.vin_index,
      script_sig: TxScriptSig {
        hex: hex::encode(&input.script_sig),
      },
      sequence: input.sequence,
      address: input.prev_address.clone(),
      value: input.prev_value,
      txid: input
        .prev_txid
        .as_ref()
        .map(|txid| utils::view_hex(txid, HashEndian::Ser)),
      vout: input.prev_vout,
      coinbase: input.coinbase.as_ref().map(hex::encode),
    })
    .collect::<Vec<TxInput>>();
  let vout = tx
    .outputs
    .iter()
    .map(|output| TxOutput {
      n: output.vout_index,
      value: output.value,
      script_pub_key: TxScriptPubKey {
        hex: hex::encode(&output.script_pubkey),
      },
      address: output.address.clone(),
      spent: false,
    })
    .collect::<Vec<TxOutput>>();

  TxDetail {
    as_of_height: None,
    txid: txid_hex.to_owned(),
    block_height: None,
    block_pos: None,
    version: tx.version,
    tx_type: public_tx_type.to_owned(),
    lock_time: tx.lock_time,
    size: tx.size,
    value_out: tx.value_out,
    input_count: tx.input_count,
    output_count: tx.output_count,
    fee: tx.fee,
    tags: TxTags {
      mined: false,
      coinbase: false,
    },
    live: TxLive::default(),
    payload: tx.payload.clone(),
    vin,
    vout,
  }
}

#[derive(Debug, Clone)]
struct PrevOutput {
  address: Option<String>,
  value: Option<i64>,
}

fn decode_tx(raw_tx: &[u8]) -> Result<DecodedTransaction, DaemonError> {
  bitcoin_consensus_encoding::decode_from_slice::<DecodedTransaction>(raw_tx).map_err(|e| {
    DaemonError::RestSource {
      message: format!("decode mempool tx: {e}"),
      kind: RestErrorKind::Other,
    }
  })
}

fn decode_payload(tx: &DecodedTransaction, txid_bytes: &[u8]) -> Value {
  if matches!(
    tx.tx_type,
    dash_primitives::TxType::Spend | dash_primitives::TxType::Unknown(_)
  ) {
    return Value::Null;
  }

  match dash_primitives::payload::SpecialPayload::decode(tx.tx_type, &tx.extra_payload) {
    Ok(payload) => {
      let (_, payload_json) = crate::tx::payload_for_storage(&payload, txid_bytes);
      crate::tx::enrich_api(crate::tx::tx_type_str(tx.tx_type), &payload_json)
    }
    Err(_) => Value::Null,
  }
}

/// Resolve the consumed outputs of a mempool transaction from committed chain
/// data in the storage engine.
async fn load_prevouts(
  db: &HyphenDb,
  tx: &DecodedTransaction,
) -> Result<HashMap<(Vec<u8>, i32), PrevOutput>, DaemonError> {
  let mut out = HashMap::new();
  for input in &tx.inputs {
    if input.prevout.hash.is_null() {
      continue;
    }
    let prev_txid = input.prevout.hash.as_bytes().to_vec();
    let vout = input.prevout.index as i32;
    let hash = hyphen_db::HyDbHash::from_slice(&prev_txid)
      .map_err(|e| DaemonError::Index(format!("mempool prevout hash: {e}")))?;
    let detail = db
      .txs()
      .get(hash)
      .await
      .map_err(|e| DaemonError::Index(format!("mempool prevout lookup: {e}")))?;
    if let Some(output) = detail.as_ref().and_then(|detail| {
      detail
        .outputs
        .iter()
        .find(|output| output.vout_index == vout)
    }) {
      out.insert(
        (prev_txid, vout),
        PrevOutput {
          address: output.address.as_ref().map(|address| address.0.clone()),
          value: Some(output.value),
        },
      );
    }
  }
  Ok(out)
}
