//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Query-time mempool enrichment for public REST surfaces.

use std::collections::HashMap;
use std::convert::TryFrom;

use crate::sql::SqlPool;
use dash_primitives::Transaction as DecodedTransaction;
use serde_json::Value;

use crate::error::DaemonError;
use crate::rest::openapi::{
  TxDetail, TxInput, TxLive, TxOutput, TxScriptPubKey, TxScriptSig, TxTags,
};
use crate::utils::{self, HashEndian};

#[allow(dead_code)]
/// One address-scoped mempool transaction summary.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AddressTxMatch {
  pub txid: String,
  pub version: i32,
  pub tx_type: String,
  pub size: usize,
  pub value_out: i64,
  pub fee: Option<i64>,
  pub last_seen_at: i64,
  pub islock: bool,
}

#[allow(dead_code)]
/// Query options for one address-scoped mempool transaction page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AddressMatchQuery {
  pub limit: usize,
  pub offset: usize,
  pub recent_limit: usize,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub include_islock: bool,
}

/// One normalized mempool transaction snapshot ready for persistence.
#[derive(Debug, Clone, PartialEq)]
pub struct NormalizedMempoolTx {
  pub detail: crate::sql::live::mempool::MempoolTxDetailRow,
  pub inputs: Vec<crate::sql::live::mempool::MempoolTxInputRow>,
  pub outputs: Vec<crate::sql::live::mempool::MempoolTxOutputRow>,
  pub address_rows: Vec<crate::sql::live::mempool::MempoolAddressTxRow>,
}

/// Decode and enrich one mempool transaction into normalized persistence rows.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when transaction decoding fails and
/// `DaemonError::Pg` when prevout enrichment queries fail.
pub async fn normalize_tx(
  pool: &SqlPool,
  txid_bytes: &[u8],
  raw_tx: &[u8],
  p2pkh_version: u8,
  p2sh_version: u8,
  first_seen_at: i64,
  last_seen_at: i64,
) -> Result<NormalizedMempoolTx, DaemonError> {
  let tx = decode_tx(raw_tx)?;
  let prevout_map = load_prevouts(pool, &tx).await?;
  let value_out = tx.outputs.iter().map(|output| output.value.to_sat() as i64).sum::<i64>();
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

  let mut inputs = Vec::with_capacity(tx.inputs.len());
  let mut outputs = Vec::with_capacity(tx.outputs.len());
  let mut address_rows = Vec::new();

  for (index, input) in tx.inputs.iter().enumerate() {
    let is_coinbase = input.prevout.hash.is_null();
    let key = (
      input.prevout.hash.as_bytes().to_vec(),
      input.prevout.index as i32,
    );
    let prev = prevout_map.get(&key);
    if let Some(address) = prev.and_then(|entry| entry.address.clone()) {
      address_rows.push(crate::sql::live::mempool::MempoolAddressTxRow {
        address,
        txid: txid_bytes.to_vec(),
        last_seen_at,
      });
    }
    inputs.push(crate::sql::live::mempool::MempoolTxInputRow {
      txid: txid_bytes.to_vec(),
      vin_index: index as i32,
      sequence: i64::from(input.sequence),
      prev_txid: (!is_coinbase).then(|| input.prevout.hash.as_bytes().to_vec()),
      prev_vout: (!is_coinbase).then_some(input.prevout.index as i32),
      coinbase: is_coinbase.then(|| input.script_sig.0.clone()),
      script_sig: input.script_sig.0.clone(),
      prev_address: prev.and_then(|entry| entry.address.clone()),
      prev_value: prev.and_then(|entry| entry.value),
    });
  }

  for (index, output) in tx.outputs.iter().enumerate() {
    let address =
      dash_script::derive_address(output.script_pubkey.as_bytes(), p2pkh_version, p2sh_version);
    if let Some(address) = address.clone() {
      address_rows.push(crate::sql::live::mempool::MempoolAddressTxRow {
        address,
        txid: txid_bytes.to_vec(),
        last_seen_at,
      });
    }
    outputs.push(crate::sql::live::mempool::MempoolTxOutputRow {
      txid: txid_bytes.to_vec(),
      vout_index: index as i32,
      value: output.value.to_sat() as i64,
      script_pubkey: output.script_pubkey.0.clone(),
      address,
    });
  }

  address_rows.sort_by(|left, right| left.address.cmp(&right.address));
  address_rows.dedup_by(|left, right| left.address == right.address && left.txid == right.txid);

  Ok(NormalizedMempoolTx {
    detail: crate::sql::live::mempool::MempoolTxDetailRow {
      txid: txid_bytes.to_vec(),
      version: tx.version,
      tx_type,
      lock_time: i32::try_from(tx.lock_time)
        .map_err(|_| DaemonError::RestSource("mempool tx lock_time exceeds i32::MAX".to_owned()))?,
      size: raw_tx.len() as i32,
      value_out,
      input_count: tx.inputs.len() as i32,
      output_count: tx.outputs.len() as i32,
      fee,
      payload: (!payload.is_null()).then_some(payload),
      first_seen_at,
      last_seen_at,
    },
    inputs,
    outputs,
    address_rows,
  })
}

/// Build a near-mined transaction detail from one recent mempool transaction.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when transaction decoding fails and
/// `DaemonError::Pg` when prevout enrichment queries fail.
pub async fn build_tx_value(
  pool: &SqlPool,
  txid_hex: &str,
  txid_bytes: &[u8],
  raw_tx: &[u8],
  p2pkh_version: u8,
  p2sh_version: u8,
  _islocked: bool,
) -> Result<TxDetail, DaemonError> {
  let tx = decode_tx(raw_tx)?;
  let prevout_map = load_prevouts(pool, &tx).await?;
  let value_out = tx.outputs.iter().map(|output| output.value.to_sat() as i64).sum::<i64>();
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
      address: dash_script::derive_address(output.script_pubkey.as_bytes(), p2pkh_version, p2sh_version),
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
    lock_time: i32::try_from(tx.lock_time)
      .map_err(|_| DaemonError::RestSource("mempool tx lock_time exceeds i32::MAX".to_owned()))?,
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

/// Build a near-mined transaction detail from persisted normalized mempool rows.
pub async fn build_tx_value_from_persisted(
  pool: &SqlPool,
  txid_hex: &str,
  txid_bytes: &[u8],
  islocked: bool,
) -> Result<Option<TxDetail>, DaemonError> {
  let Some(detail) = crate::sql::live::mempool::tx_detail(pool, txid_bytes).await? else {
    return Ok(None);
  };
  let inputs = crate::sql::live::mempool::tx_inputs(pool, txid_bytes).await?;
  let outputs = crate::sql::live::mempool::tx_outputs(pool, txid_bytes).await?;

  Ok(Some(persisted_tx_value(
    txid_hex, &detail, &inputs, &outputs, islocked,
  )))
}

fn persisted_tx_value(
  txid_hex: &str,
  detail: &crate::sql::live::mempool::MempoolTxDetailRow,
  inputs: &[crate::sql::live::mempool::MempoolTxInputRow],
  outputs: &[crate::sql::live::mempool::MempoolTxOutputRow],
  _islocked: bool,
) -> TxDetail {
  let public_tx_type = crate::tx::public_tx_type(
    &detail.tx_type,
    outputs.iter().map(|output| output.script_pubkey.as_slice()),
  );
  let vin = inputs
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
  let vout = outputs
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
    version: detail.version,
    tx_type: public_tx_type.to_owned(),
    lock_time: detail.lock_time,
    size: detail.size,
    value_out: detail.value_out,
    input_count: detail.input_count,
    output_count: detail.output_count,
    fee: detail.fee,
    tags: TxTags {
      mined: false,
      coinbase: false,
    },
    live: TxLive::default(),
    payload: detail.payload.clone(),
    vin,
    vout,
  }
}

#[allow(dead_code)]
/// Find recent mempool transactions that touch one address.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when one cached mempool transaction fails
/// to decode and `DaemonError::Pg` when backing PostgreSQL queries fail.
pub async fn address_matches(
  pool: &SqlPool,
  address: &str,
  query: AddressMatchQuery,
) -> Result<Vec<AddressTxMatch>, DaemonError> {
  let persisted = crate::sql::live::mempool::address_txs_page(
    pool,
    address,
    query.limit as i64,
    query.offset as i64,
  )
  .await?;
  if !persisted.is_empty() {
    let mut matches = Vec::with_capacity(persisted.len());
    for row in persisted {
      let islock = if query.include_islock {
        crate::sql::live::islock::current_by_txid(pool, &row.txid)
          .await?
          .is_some()
      } else {
        false
      };
      matches.push(address_match_from_persisted_row(row, islock));
    }
    return Ok(matches);
  }

  let rows = crate::sql::live::mempool::recent_txs(pool, query.recent_limit as i64).await?;
  let mut matches = Vec::new();

  for row in rows {
    let tx = decode_tx(&row.raw_tx)?;
    let prevout_map = load_prevouts(pool, &tx).await?;
    let output_match = tx.outputs.iter().any(|output| {
      dash_script::derive_address(
        output.script_pubkey.as_bytes(),
        query.p2pkh_version,
        query.p2sh_version,
      )
      .is_some_and(|derived| derived == address)
    });
    let input_match = tx.inputs.iter().any(|input| {
      let key = (
        input.prevout.hash.as_bytes().to_vec(),
        input.prevout.index as i32,
      );
      prevout_map
        .get(&key)
        .and_then(|entry| entry.address.as_deref())
        .is_some_and(|derived| derived == address)
    });
    if !output_match && !input_match {
      continue;
    }

    let islock = if query.include_islock {
      crate::sql::live::islock::current_by_txid(pool, &row.txid)
        .await?
        .is_some()
    } else {
      false
    };
    matches.push(AddressTxMatch {
      txid: utils::view_hex(&row.txid, HashEndian::View),
      version: i32::from(tx.version),
      tx_type: crate::tx::tx_type_str(tx.tx_type).to_owned(),
      size: row.raw_tx.len(),
      value_out: tx.outputs.iter().map(|output| output.value.to_sat() as i64).sum::<i64>(),
      fee: None,
      last_seen_at: row.last_seen_at,
      islock,
    });
  }

  matches.sort_by(|left, right| right.last_seen_at.cmp(&left.last_seen_at));
  Ok(matches)
}

#[allow(dead_code)]
fn address_match_from_persisted_row(
  row: crate::sql::live::mempool::MempoolTxDetailRow,
  islock: bool,
) -> AddressTxMatch {
  AddressTxMatch {
    txid: utils::view_hex(&row.txid, HashEndian::View),
    version: i32::from(row.version),
    tx_type: row.tx_type,
    size: row.size as usize,
    value_out: row.value_out,
    fee: row.fee,
    last_seen_at: row.last_seen_at,
    islock,
  }
}

#[derive(Debug, Clone)]
struct PrevOutput {
  address: Option<String>,
  value: Option<i64>,
}

fn decode_tx(raw_tx: &[u8]) -> Result<DecodedTransaction, DaemonError> {
  bitcoin_consensus_encoding::decode_from_slice::<DecodedTransaction>(raw_tx)
    .map_err(|e| DaemonError::RestSource(format!("decode mempool tx: {e}")))
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

async fn load_prevouts(
  pool: &SqlPool,
  tx: &DecodedTransaction,
) -> Result<HashMap<(Vec<u8>, i32), PrevOutput>, DaemonError> {
  let prev_txids = tx
    .inputs
    .iter()
    .filter(|input| !input.prevout.hash.is_null())
    .map(|input| input.prevout.hash.as_bytes().to_vec())
    .collect::<Vec<_>>();
  let prev_vouts = tx
    .inputs
    .iter()
    .filter(|input| !input.prevout.hash.is_null())
    .map(|input| input.prevout.index as i32)
    .collect::<Vec<_>>();
  if prev_txids.is_empty() {
    return Ok(HashMap::new());
  }

  let mut sql_tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin mempool prevout lookup tx: {e}")))?;
  let rows =
    crate::index::spent::sql::lookup_prev_outputs(&mut sql_tx, &prev_txids, &prev_vouts).await?;
  sql_tx
    .commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit mempool prevout lookup tx: {e}")))?;

  let mut out = HashMap::with_capacity(rows.len());
  for row in rows {
    out.insert(
      (row.prev_txid, row.prev_vout),
      PrevOutput {
        address: row.prev_addr,
        value: Some(row.prev_value),
      },
    );
  }
  Ok(out)
}

#[cfg(test)]
mod tests {
  use crate::rest::openapi::TxLive;

  use super::{build_tx_value, normalize_tx};

  fn p2pkh_tx_bytes() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1i32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&[0x00; 32]);
    buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&42i64.to_le_bytes());
    buf.push(0x19);
    buf.extend_from_slice(&[
      0x76, 0xa9, 0x14, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
      0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x88, 0xac,
    ]);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
  }

  fn op_return_tx_bytes() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1i32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&[0x00; 32]);
    buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    buf.push(0x00);
    buf.extend_from_slice(&0xFFFF_FFFFu32.to_le_bytes());
    buf.push(0x01);
    buf.extend_from_slice(&42i64.to_le_bytes());
    buf.push(0x03);
    buf.extend_from_slice(&[0x6a, 0x01, 0x42]);
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf
  }

  fn provider_register_payload_bytes() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&[0x11; 32]); // collateral_outpoint.hash
    buf.extend_from_slice(&0u32.to_le_bytes()); // collateral_outpoint.index
    buf.extend_from_slice(&[0x22; 16]); // ipv6
    buf.extend_from_slice(&9999u16.to_be_bytes()); // port
    buf.extend_from_slice(&[0x33; 20]); // key_id_owner
    buf.extend_from_slice(&[0x44; 48]); // pub_key_operator
    buf.extend_from_slice(&[0x55; 20]); // key_id_voting
    buf.extend_from_slice(&100u16.to_le_bytes()); // operator_reward
    buf.push(0x00); // empty script_payout
    buf.extend_from_slice(&[0x66; 32]); // inputs_hash
    buf.push(0x03); // sig length
    buf.extend_from_slice(&[0x77; 3]); // sig
    buf
  }

  fn provider_register_tx_bytes(payload: &[u8]) -> Vec<u8> {
    assert!(
      payload.len() < 0xfd,
      "test payload must fit in one-byte CompactSize"
    );

    let mut buf = Vec::new();
    let n_version: i32 = 3 | (1 << 16);
    buf.extend_from_slice(&n_version.to_le_bytes());
    buf.push(0x00); // vin count
    buf.push(0x00); // vout count
    buf.extend_from_slice(&0u32.to_le_bytes()); // lock_time
    buf.push(payload.len() as u8); // extra_payload length
    buf.extend_from_slice(payload);
    buf
  }

  #[tokio::test]
  async fn build_tx_value_derives_output_addresses() {
    let pool = crate::sql::SqlPool::connect_lazy("postgres://localhost/unused")
      .expect("lazy postgres pool should build");
    let expected = dash_script::derive_address(
      &[
        0x76, 0xa9, 0x14, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11,
        0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x11, 0x88, 0xac,
      ],
      76,
      16,
    )
    .expect("p2pkh script should derive");
    let value = build_tx_value(
      &pool,
      &"11".repeat(32),
      &[0x11; 32],
      &p2pkh_tx_bytes(),
      76,
      16,
      false,
    )
    .await
    .expect("mempool tx should build");
    assert!(!value.tags.mined);
    assert_eq!(value.vout[0].address.as_deref(), Some(expected.as_str()));
  }

  #[tokio::test]
  async fn normalize_tx_derives_payload_and_address_rows() {
    let pool = crate::sql::SqlPool::connect_lazy("postgres://localhost/unused")
      .expect("lazy postgres pool should build");
    let normalized = normalize_tx(&pool, &[0x11; 32], &p2pkh_tx_bytes(), 76, 16, 10, 20)
      .await
      .expect("mempool tx should normalize");

    assert_eq!(normalized.detail.tx_type, "normal");
    assert_eq!(normalized.detail.first_seen_at, 10);
    assert_eq!(normalized.detail.last_seen_at, 20);
    assert_eq!(normalized.inputs.len(), 1);
    assert_eq!(normalized.outputs.len(), 1);
    assert_eq!(normalized.address_rows.len(), 1);
    assert_eq!(normalized.address_rows[0].last_seen_at, 20);
    assert!(normalized.detail.payload.is_none());
  }

  #[tokio::test]
  async fn build_tx_value_marks_normal_op_return_transactions_as_data() {
    let pool = crate::sql::SqlPool::connect_lazy("postgres://localhost/unused")
      .expect("lazy postgres pool should build");
    let value = build_tx_value(
      &pool,
      &"11".repeat(32),
      &[0x11; 32],
      &op_return_tx_bytes(),
      76,
      16,
      false,
    )
    .await
    .expect("mempool tx should build");
    assert_eq!(value.tx_type, "data");
    assert!(value.vout[0].address.is_none());
  }

  #[tokio::test]
  async fn normalize_tx_keeps_provider_register_payloads() {
    let pool = crate::sql::SqlPool::connect_lazy("postgres://localhost/unused")
      .expect("lazy postgres pool should build");
    let normalized = normalize_tx(
      &pool,
      &[0x11; 32],
      &provider_register_tx_bytes(&provider_register_payload_bytes()),
      76,
      16,
      10,
      20,
    )
    .await
    .expect("provider register mempool tx should normalize");

    assert_eq!(normalized.detail.tx_type, "provider_register");
    assert_eq!(normalized.detail.value_out, 0);
    assert!(normalized.inputs.is_empty());
    assert!(normalized.outputs.is_empty());
    assert!(normalized.address_rows.is_empty());
  }

  #[tokio::test]
  async fn build_tx_value_drops_invalid_special_payloads_instead_of_erroring() {
    let pool = crate::sql::SqlPool::connect_lazy("postgres://localhost/unused")
      .expect("lazy postgres pool should build");
    let value = build_tx_value(
      &pool,
      &"11".repeat(32),
      &[0x11; 32],
      &provider_register_tx_bytes(&[0xde, 0xad]),
      76,
      16,
      false,
    )
    .await
    .expect("invalid special payload should not break mempool tx detail");

    assert_eq!(value.tx_type, "provider_register");
    assert_eq!(value.value_out, 0);
    assert_eq!(value.payload, None);
  }

  #[test]
  fn persisted_tx_value_shapes_live_detail_tags_and_payload() {
    let value = super::persisted_tx_value(
      &"11".repeat(32),
      &crate::sql::live::mempool::MempoolTxDetailRow {
        txid: vec![0x11; 32],
        version: 3,
        tx_type: "normal".to_owned(),
        lock_time: 7,
        size: 123,
        value_out: 42,
        input_count: 1,
        output_count: 1,
        fee: Some(5),
        payload: Some(serde_json::json!({"foo": "bar"})),
        first_seen_at: 10,
        last_seen_at: 20,
      },
      &[crate::sql::live::mempool::MempoolTxInputRow {
        txid: vec![0x11; 32],
        vin_index: 0,
        sequence: 99,
        prev_txid: Some(vec![0x22; 32]),
        prev_vout: Some(1),
        coinbase: None,
        script_sig: vec![0xaa],
        prev_address: Some("Xprev".to_owned()),
        prev_value: Some(50),
      }],
      &[crate::sql::live::mempool::MempoolTxOutputRow {
        txid: vec![0x11; 32],
        vout_index: 0,
        value: 42,
        script_pubkey: vec![0xbb],
        address: Some("Xdest".to_owned()),
      }],
      true,
    );
    assert!(!value.tags.mined);
    assert!(!value.tags.coinbase);
    assert_eq!(value.live, TxLive::default());
    assert_eq!(value.payload, Some(serde_json::json!({"foo": "bar"})));
    assert_eq!(value.vin[0].address.as_deref(), Some("Xprev"));
    assert_eq!(value.vout[0].address.as_deref(), Some("Xdest"));
  }

  #[test]
  fn persisted_tx_value_marks_normal_op_return_transactions_as_data() {
    let value = super::persisted_tx_value(
      &"11".repeat(32),
      &crate::sql::live::mempool::MempoolTxDetailRow {
        txid: vec![0x11; 32],
        version: 3,
        tx_type: "normal".to_owned(),
        lock_time: 7,
        size: 123,
        value_out: 42,
        input_count: 1,
        output_count: 1,
        fee: Some(5),
        payload: None,
        first_seen_at: 10,
        last_seen_at: 20,
      },
      &[],
      &[crate::sql::live::mempool::MempoolTxOutputRow {
        txid: vec![0x11; 32],
        vout_index: 0,
        value: 42,
        script_pubkey: vec![0x6a, 0x01, 0x42],
        address: None,
      }],
      false,
    );
    assert_eq!(value.tx_type, "data");
  }

  #[test]
  fn address_match_from_persisted_row_keeps_fee_and_seen_time() {
    let row = crate::sql::live::mempool::MempoolTxDetailRow {
      txid: vec![0x44; 32],
      version: 2,
      tx_type: "special".to_owned(),
      lock_time: 0,
      size: 321,
      value_out: 99,
      input_count: 2,
      output_count: 3,
      fee: Some(7),
      payload: None,
      first_seen_at: 1,
      last_seen_at: 2,
    };
    let summary = super::address_match_from_persisted_row(row, false);
    assert_eq!(summary.txid, "44".repeat(32));
    assert_eq!(summary.version, 2);
    assert_eq!(summary.tx_type, "special");
    assert_eq!(summary.size, 321);
    assert_eq!(summary.value_out, 99);
    assert_eq!(summary.fee, Some(7));
    assert_eq!(summary.last_seen_at, 2);
    assert!(!summary.islock);
  }
}
