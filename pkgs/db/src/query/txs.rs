//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The transactions domain. Output spent-status comes from the append-only
//! `spends` table (joined on the packed outpoint), never a mutable flag; input
//! provenance (`prev_address`/`prev_value`) is stored inline on `tx_inputs` at
//! derive time, so reading a transaction needs no back-join to prior outputs.

use async_trait::async_trait;

use super::PgRead;
use crate::error::{HyDbError, HyDbResult};
use crate::types::{
  HyDbAddrType, HyDbAddress, HyDbCursor, HyDbHash, HyDbHeight, HyDbOutpoint, HyDbPage, HyDbTxType,
};
use crate::util::hex_prefix_pattern;

/// Full transaction detail.
#[derive(Debug, Clone)]
pub struct HyDbQrTxDetail {
  pub txid: HyDbHash,
  pub block_height: HyDbHeight,
  pub block_pos: i32,
  pub version: i32,
  pub tx_type: HyDbTxType,
  pub lock_time: i64,
  pub size: i32,
  pub value_out: i64,
  pub fee: Option<i64>,
  pub inputs: Vec<HyDbQrTxIn>,
  pub outputs: Vec<HyDbQrTxOut>,
  /// Decoded special-tx payload JSON, when present.
  pub payload: Option<serde_json::Value>,
}

/// A transaction in a list view.
#[derive(Debug, Clone)]
pub struct HyDbQrTxSummary {
  pub txid: HyDbHash,
  pub block_height: HyDbHeight,
  pub block_pos: i32,
  pub version: i32,
  pub tx_type: HyDbTxType,
  pub lock_time: i64,
  pub size: i32,
  pub value_out: i64,
  pub input_count: i32,
  pub output_count: i32,
  pub fee: Option<i64>,
}

/// A resolved transaction input: it *consumes* a UTXO. `prev_outpoint` names
/// the output being spent (absent for a coinbase input); `prev_address` and
/// `prev_value` are the inlined details of that consumed output.
#[derive(Debug, Clone)]
pub struct HyDbQrTxIn {
  pub vin_index: i32,
  pub prev_outpoint: Option<HyDbOutpoint>,
  pub coinbase: bool,
  pub prev_address: Option<HyDbAddress>,
  pub prev_value: Option<i64>,
  pub script_sig: Vec<u8>,
  pub sequence: i64,
}

/// A transaction output: it *creates* a UTXO. `spent_by` names the input that
/// later consumed it (as `txid:vin` packed into an outpoint), or is absent
/// while the output is still unspent.
#[derive(Debug, Clone)]
pub struct HyDbQrTxOut {
  pub vout_index: i32,
  pub value: i64,
  pub address: Option<HyDbAddress>,
  pub address_type: Option<HyDbAddrType>,
  pub script_pubkey: Vec<u8>,
  pub spent_by: Option<HyDbOutpoint>,
}

/// Query handle for transaction data.
#[async_trait]
pub trait HyDbTxIdx: Send + Sync {
  /// Full detail for a transaction, resolving inputs and output spent-status.
  async fn get(&self, txid: HyDbHash) -> HyDbResult<Option<HyDbQrTxDetail>>;

  /// Newest-first page of transactions across the chain.
  async fn list(
    &self,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrTxSummary>>;

  /// Total number of canonical transactions (precomputed).
  async fn total(&self) -> HyDbResult<u64>;

  /// txids whose DISPLAY-order hex (what the UI renders) starts with
  /// `prefix`, most-recent first. Lowercased and `LIKE`-escaped internally.
  async fn search_prefix(&self, prefix: String, limit: u32) -> HyDbResult<Vec<HyDbHash>>;
}

#[derive(Debug)]
pub(crate) struct PgTxIdx {
  pub(crate) pg: PgRead,
}

/// Rebuild an outpoint from separate txid/index columns.
fn outpoint(txid: Option<Vec<u8>>, index: Option<i32>) -> HyDbResult<Option<HyDbOutpoint>> {
  match (txid, index) {
    (Some(t), Some(i)) => Ok(Some(HyDbOutpoint {
      txid: HyDbHash::from_slice(&t)?,
      vout: i as u32,
    })),
    _ => Ok(None),
  }
}

#[async_trait]
impl HyDbTxIdx for PgTxIdx {
  async fn get(&self, txid: HyDbHash) -> HyDbResult<Option<HyDbQrTxDetail>> {
    let key = &txid.0[..];

    // The transaction header. A txid can (pre-BIP30 enforcement) recur across
    // blocks; take the most recent for determinism, matching prior behavior.
    let Some(tx) = sqlx::query!(
      r#"SELECT block_height, block_pos, version, tx_type AS "tx_type: HyDbTxType",
                lock_time, size, value_out, fee
           FROM transactions WHERE txid = $1
          ORDER BY block_height DESC LIMIT 1"#,
      key
    )
    .fetch_optional(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("transaction header", e))?
    else {
      return Ok(None);
    };

    // Inputs/outputs/payload all carry the chosen header's height: it prunes
    // the partition fan-out to one partition, and it keeps a duplicated txid
    // from concatenating both occurrences' rows.
    let input_rows = sqlx::query!(
      r#"SELECT vin_index, prev_txid, prev_vout, prev_address, prev_value, script_sig, sequence
           FROM tx_inputs WHERE txid = $1 AND block_height = $2 ORDER BY vin_index"#,
      key,
      tx.block_height
    )
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("transaction inputs", e))?;

    let mut inputs = Vec::with_capacity(input_rows.len());
    for r in input_rows {
      let coinbase = r.prev_txid.is_none();
      inputs.push(HyDbQrTxIn {
        vin_index: r.vin_index,
        prev_outpoint: outpoint(r.prev_txid, r.prev_vout)?,
        coinbase,
        prev_address: r.prev_address.map(HyDbAddress),
        prev_value: r.prev_value,
        script_sig: r.script_sig,
        sequence: r.sequence,
      });
    }

    let output_rows = sqlx::query!(
      r#"SELECT o.vout_index, o.value, o.script_pubkey, o.address,
                o.address_type AS "address_type: HyDbAddrType",
                s.spending_txid AS "spending_txid?", s.spending_vin AS "spending_vin?"
           FROM tx_outputs o
           LEFT JOIN spends s ON s.outpoint = o.outpoint
          WHERE o.txid = $1 AND o.block_height = $2 ORDER BY o.vout_index"#,
      key,
      tx.block_height
    )
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("transaction outputs", e))?;

    let mut outputs = Vec::with_capacity(output_rows.len());
    for r in output_rows {
      outputs.push(HyDbQrTxOut {
        vout_index: r.vout_index,
        value: r.value,
        address: r.address.map(HyDbAddress),
        address_type: r.address_type,
        script_pubkey: r.script_pubkey,
        spent_by: outpoint(r.spending_txid, r.spending_vin)?,
      });
    }

    let payload = sqlx::query_scalar!(
      r#"SELECT payload FROM sptx_payloads WHERE txid = $1 AND block_height = $2
          ORDER BY block_height DESC LIMIT 1"#,
      key,
      tx.block_height
    )
    .fetch_optional(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("transaction payload", e))?;

    Ok(Some(HyDbQrTxDetail {
      txid,
      block_height: HyDbHeight::try_from_i32(tx.block_height)?,
      block_pos: tx.block_pos,
      version: tx.version,
      tx_type: tx.tx_type,
      lock_time: tx.lock_time,
      size: tx.size,
      value_out: tx.value_out,
      fee: tx.fee,
      inputs,
      outputs,
      payload,
    }))
  }

  async fn list(
    &self,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrTxSummary>> {
    let limit = limit.clamp(1, 500) as i64;
    let fetch = limit + 1;
    let before = match before {
      None => None,
      Some(HyDbCursor(s)) => {
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        let [h, p, txid_hex] = parts.as_slice() else {
          return Err(HyDbError::Decode(format!("invalid tx cursor: {s}")));
        };
        let h: i32 = h
          .parse()
          .map_err(|_| HyDbError::Decode(format!("invalid tx cursor height: {s}")))?;
        let p: i32 = p
          .parse()
          .map_err(|_| HyDbError::Decode(format!("invalid tx cursor pos: {s}")))?;
        Some((h, p, hex::decode(txid_hex)?))
      }
    };

    type Row = (
      Vec<u8>,
      i32,
      i32,
      i32,
      HyDbTxType,
      i64,
      i32,
      i64,
      i32,
      i32,
      Option<i64>,
    );
    let rows: Vec<Row> = match &before {
      Some((h, p, txid)) => sqlx::query!(
        r#"SELECT txid, block_height, block_pos, version, tx_type AS "tx_type: HyDbTxType",
                  lock_time, size, value_out, input_count, output_count, fee
             FROM transactions
            WHERE (block_height, block_pos, txid) < ($1, $2, $3)
            ORDER BY block_height DESC, block_pos DESC, txid DESC LIMIT $4"#,
        h,
        p,
        &txid[..],
        fetch
      )
      .fetch_all(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("list transactions", e))?
      .into_iter()
      .map(|r| {
        (
          r.txid,
          r.block_height,
          r.block_pos,
          r.version,
          r.tx_type,
          r.lock_time,
          r.size,
          r.value_out,
          r.input_count,
          r.output_count,
          r.fee,
        )
      })
      .collect(),
      None => sqlx::query!(
        r#"SELECT txid, block_height, block_pos, version, tx_type AS "tx_type: HyDbTxType",
                  lock_time, size, value_out, input_count, output_count, fee
             FROM transactions
            ORDER BY block_height DESC, block_pos DESC, txid DESC LIMIT $1"#,
        fetch
      )
      .fetch_all(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("list transactions", e))?
      .into_iter()
      .map(|r| {
        (
          r.txid,
          r.block_height,
          r.block_pos,
          r.version,
          r.tx_type,
          r.lock_time,
          r.size,
          r.value_out,
          r.input_count,
          r.output_count,
          r.fee,
        )
      })
      .collect(),
    };

    let has_more = rows.len() as i64 > limit;
    let mut items = Vec::with_capacity(rows.len().min(limit as usize));
    for r in rows.iter().take(limit as usize) {
      items.push(HyDbQrTxSummary {
        txid: HyDbHash::from_slice(&r.0)?,
        block_height: HyDbHeight::try_from_i32(r.1)?,
        block_pos: r.2,
        version: r.3,
        tx_type: r.4,
        lock_time: r.5,
        size: r.6,
        value_out: r.7,
        input_count: r.8,
        output_count: r.9,
        fee: r.10,
      });
    }
    let next = items.last().filter(|_| has_more).map(|t| {
      HyDbCursor(format!(
        "{}:{}:{}",
        t.block_height.0,
        t.block_pos,
        hex::encode(t.txid.as_bytes())
      ))
    });
    Ok(HyDbPage::new(items, next))
  }

  async fn total(&self) -> HyDbResult<u64> {
    let total =
      sqlx::query_scalar!("SELECT total FROM transactions_summary WHERE singleton = TRUE")
        .fetch_one(&self.pg.query)
        .await
        .map_err(|e| HyDbError::pg("transactions summary", e))?;
    Ok(total.max(0) as u64)
  }

  async fn search_prefix(&self, prefix: String, limit: u32) -> HyDbResult<Vec<HyDbHash>> {
    let limit = limit.clamp(1, 100) as i64;
    // DISPLAY-order hex match (see the search contract in the schema), backed
    // by the display_hex expression index.
    let pattern = hex_prefix_pattern(&prefix);
    let rows = sqlx::query_scalar!(
      r#"SELECT txid AS "txid!" FROM transactions
          WHERE display_hex(txid) LIKE $1 ESCAPE '\'
          ORDER BY block_height DESC, block_pos DESC, txid ASC LIMIT $2"#,
      pattern,
      limit
    )
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("search transactions by prefix", e))?;
    rows.iter().map(|txid| HyDbHash::from_slice(txid)).collect()
  }
}
