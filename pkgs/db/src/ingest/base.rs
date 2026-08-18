//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The base-facts stage: the block header, every transaction (fee inlined —
//! it is known before the write transaction opens, so the append-only
//! `transactions` table never takes a follow-up `UPDATE`), inputs and
//! outputs, the `spends` ledger, the `utxo_set` projection, and the
//! `address_txs` index. This is the widest stage — it owns all the raw chain
//! facts every other stage's accumulations derive from, and it writes them
//! **batched**: one `UNNEST` statement per table per block rather than one
//! round trip per row. Its disconnect rebuilds the spent UTXOs from those
//! same facts and drops every row the block wrote.

use std::collections::BTreeSet;

use async_trait::async_trait;

use super::pipeline::IngestStage;
use super::scratch::{HyDbStageError, Scratch};
use crate::records::{PgTx, SpentOutput};
use crate::types::{HyDbAddrType, HyDbOutpoint, HyDbTxType};

/// The base-facts + base-index stage.
pub(super) struct BaseStage;

#[async_trait]
impl IngestStage for BaseStage {
  const NAME: &'static str = "base";

  async fn connect(&self, tx: &mut PgTx<'_>, scratch: &Scratch) -> Result<(), HyDbStageError> {
    let height = scratch.height;
    let block = scratch.block(Self::NAME)?;
    let resolved = scratch.resolved(Self::NAME)?;
    let fees = scratch.fees(Self::NAME)?;

    block
      .header
      .insert(tx, height, scratch.chainwork())
      .await
      .map_err(HyDbStageError::wrap(Self::NAME))?;

    // ---- Column arrays, one pass over the block ---------------------------
    let inconsistent = |what: String| HyDbStageError::Inconsistent {
      stage: Self::NAME,
      what,
    };

    // transactions
    let n_txs = block.txs.len();
    let mut t_txid: Vec<Vec<u8>> = Vec::with_capacity(n_txs);
    let mut t_pos: Vec<i32> = Vec::with_capacity(n_txs);
    let mut t_version: Vec<i32> = Vec::with_capacity(n_txs);
    let mut t_type: Vec<HyDbTxType> = Vec::with_capacity(n_txs);
    let mut t_lock: Vec<i64> = Vec::with_capacity(n_txs);
    let mut t_size: Vec<i32> = Vec::with_capacity(n_txs);
    let mut t_value_out: Vec<i64> = Vec::with_capacity(n_txs);
    let mut t_in_count: Vec<i32> = Vec::with_capacity(n_txs);
    let mut t_out_count: Vec<i32> = Vec::with_capacity(n_txs);
    let mut t_fee: Vec<Option<i64>> = Vec::with_capacity(n_txs);

    // tx_outputs + created UTXOs (same row set)
    let mut o_txid: Vec<Vec<u8>> = Vec::new();
    let mut o_vout: Vec<i32> = Vec::new();
    let mut o_value: Vec<i64> = Vec::new();
    let mut o_script: Vec<Vec<u8>> = Vec::new();
    let mut o_addr: Vec<Option<String>> = Vec::new();
    let mut o_addr_type: Vec<HyDbAddrType> = Vec::new();
    let mut o_outpoint: Vec<Vec<u8>> = Vec::new();

    // tx_inputs
    let mut i_txid: Vec<Vec<u8>> = Vec::new();
    let mut i_vin: Vec<i32> = Vec::new();
    let mut i_prev_txid: Vec<Option<Vec<u8>>> = Vec::new();
    let mut i_prev_vout: Vec<Option<i32>> = Vec::new();
    let mut i_script: Vec<Vec<u8>> = Vec::new();
    let mut i_seq: Vec<i64> = Vec::new();
    let mut i_prev_addr: Vec<Option<String>> = Vec::new();
    let mut i_prev_value: Vec<Option<i64>> = Vec::new();

    // spends + consumed UTXOs
    let mut s_txid: Vec<Vec<u8>> = Vec::new();
    let mut s_vin: Vec<i32> = Vec::new();
    let mut s_outpoint: Vec<Vec<u8>> = Vec::new();

    // address_txs, deduped across the input and output sides
    let mut addr_txs: BTreeSet<(String, Vec<u8>, i32)> = BTreeSet::new();

    for (t, btx) in block.txs.iter().enumerate() {
      let txid = btx.txid.0.to_vec();
      let pos = btx.block_pos as i32;

      t_txid.push(txid.clone());
      t_pos.push(pos);
      t_version.push(btx.version);
      t_type.push(btx.tx_type);
      t_lock.push(i64::from(btx.lock_time));
      t_size.push(btx.size as i32);
      t_value_out.push(btx.value_out.0);
      t_in_count.push(btx.inputs.len() as i32);
      t_out_count.push(btx.outputs.len() as i32);
      t_fee.push(
        *fees
          .get(t)
          .ok_or_else(|| inconsistent(format!("no fee slot for tx index {t}")))?,
      );

      for out in &btx.outputs {
        let outpoint = HyDbOutpoint {
          txid: btx.txid,
          vout: out.vout,
        }
        .packed()
        .to_vec();
        o_txid.push(txid.clone());
        o_vout.push(out.vout as i32);
        o_value.push(out.value.0);
        o_script.push(out.script_pubkey.clone());
        o_addr.push(out.address.as_ref().map(|a| a.0.clone()));
        o_addr_type.push(out.address_type);
        o_outpoint.push(outpoint);
        if let Some(addr) = &out.address {
          addr_txs.insert((addr.0.clone(), txid.clone(), pos));
        }
      }

      let tx_resolved = resolved
        .get(t)
        .ok_or_else(|| inconsistent(format!("no resolved slot for tx index {t}")))?;
      for (v, inp) in btx.inputs.iter().enumerate() {
        let spent: Option<&SpentOutput> = tx_resolved
          .get(v)
          .ok_or_else(|| inconsistent(format!("no resolved slot for input {t}:{v}")))?
          .as_ref();
        let prev = inp.prev_outpoint;
        i_txid.push(txid.clone());
        i_vin.push(inp.vin_index as i32);
        i_prev_txid.push(prev.map(|p| p.txid.0.to_vec()));
        i_prev_vout.push(prev.map(|p| p.vout as i32));
        i_script.push(inp.script_sig.clone());
        i_seq.push(i64::from(inp.sequence));
        i_prev_addr.push(spent.and_then(|s| s.address.clone()));
        i_prev_value.push(spent.map(|s| s.value));

        if inp.coinbase {
          continue;
        }
        // The deriver already validated that every non-coinbase input carries
        // a prev_outpoint; a violation here is a corrupt scratch, surfaced as
        // a routable error rather than a panic.
        let Some(prev) = prev else {
          return Err(inconsistent(format!(
            "non-coinbase input {}:{} has no prev_outpoint",
            hex::encode(btx.txid.0),
            inp.vin_index
          )));
        };
        s_txid.push(txid.clone());
        s_vin.push(inp.vin_index as i32);
        s_outpoint.push(prev.packed().to_vec());
        if let Some(addr) = spent.and_then(|s| s.address.as_deref()) {
          addr_txs.insert((addr.to_owned(), txid.clone(), pos));
        }
      }
    }

    // ---- One statement per table ------------------------------------------
    sqlx::query!(
      r#"INSERT INTO transactions
           (txid, block_height, block_pos, version, tx_type, lock_time, size, value_out,
            input_count, output_count, fee)
         SELECT t.txid, $1, t.block_pos, t.version, t.tx_type, t.lock_time, t.size, t.value_out,
                t.input_count, t.output_count, t.fee
           FROM UNNEST($2::bytea[], $3::int4[], $4::int4[], $5::tx_type[], $6::int8[],
                       $7::int4[], $8::int8[], $9::int4[], $10::int4[], $11::int8[])
             AS t(txid, block_pos, version, tx_type, lock_time, size, value_out,
                  input_count, output_count, fee)"#,
      height,
      &t_txid,
      &t_pos,
      &t_version,
      &t_type as _,
      &t_lock,
      &t_size,
      &t_value_out,
      &t_in_count,
      &t_out_count,
      &t_fee as _,
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(Self::NAME, "base: insert transactions"))?;

    if !o_txid.is_empty() {
      sqlx::query!(
        r#"INSERT INTO tx_outputs
             (block_height, txid, vout_index, value, script_pubkey, address, address_type)
           SELECT $1, t.txid, t.vout_index, t.value, t.script_pubkey, t.address, t.address_type
             FROM UNNEST($2::bytea[], $3::int4[], $4::int8[], $5::bytea[], $6::text[],
                         $7::address_type[])
               AS t(txid, vout_index, value, script_pubkey, address, address_type)"#,
        height,
        &o_txid,
        &o_vout,
        &o_value,
        &o_script,
        &o_addr as _,
        &o_addr_type as _,
      )
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(Self::NAME, "base: insert outputs"))?;

      sqlx::query!(
        r#"INSERT INTO utxo_set (outpoint, address, value, block_height, script_pubkey)
           SELECT t.outpoint, t.address, t.value, $1, t.script_pubkey
             FROM UNNEST($2::bytea[], $3::text[], $4::int8[], $5::bytea[])
               AS t(outpoint, address, value, script_pubkey)"#,
        height,
        &o_outpoint,
        &o_addr as _,
        &o_value,
        &o_script,
      )
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(Self::NAME, "base: insert utxos"))?;
    }

    if !i_txid.is_empty() {
      sqlx::query!(
        r#"INSERT INTO tx_inputs
             (block_height, txid, vin_index, prev_txid, prev_vout, script_sig, sequence,
              prev_address, prev_value)
           SELECT $1, t.txid, t.vin_index, t.prev_txid, t.prev_vout, t.script_sig, t.sequence,
                  t.prev_address, t.prev_value
             FROM UNNEST($2::bytea[], $3::int4[], $4::bytea[], $5::int4[], $6::bytea[],
                         $7::int8[], $8::text[], $9::int8[])
               AS t(txid, vin_index, prev_txid, prev_vout, script_sig, sequence,
                    prev_address, prev_value)"#,
        height,
        &i_txid,
        &i_vin,
        &i_prev_txid as _,
        &i_prev_vout as _,
        &i_script,
        &i_seq,
        &i_prev_addr as _,
        &i_prev_value as _,
      )
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(Self::NAME, "base: insert inputs"))?;
    }

    if !s_outpoint.is_empty() {
      sqlx::query!(
        r#"INSERT INTO spends (spend_height, spending_txid, spending_vin, outpoint)
           SELECT $1, t.txid, t.vin, t.outpoint
             FROM UNNEST($2::bytea[], $3::int4[], $4::bytea[]) AS t(txid, vin, outpoint)"#,
        height,
        &s_txid,
        &s_vin,
        &s_outpoint,
      )
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(Self::NAME, "base: insert spends"))?;

      sqlx::query!("DELETE FROM utxo_set WHERE outpoint = ANY($1)", &s_outpoint)
        .execute(&mut **tx)
        .await
        .map_err(HyDbStageError::sql(Self::NAME, "base: remove spent utxos"))?;
    }

    if !addr_txs.is_empty() {
      let mut a_addr: Vec<String> = Vec::with_capacity(addr_txs.len());
      let mut a_txid: Vec<Vec<u8>> = Vec::with_capacity(addr_txs.len());
      let mut a_pos: Vec<i32> = Vec::with_capacity(addr_txs.len());
      for (addr, txid, pos) in addr_txs {
        a_addr.push(addr);
        a_txid.push(txid);
        a_pos.push(pos);
      }
      sqlx::query!(
        r#"INSERT INTO address_txs (address, block_height, txid, block_pos)
           SELECT t.address, $1, t.txid, t.block_pos
             FROM UNNEST($2::text[], $3::bytea[], $4::int4[]) AS t(address, txid, block_pos)
           ON CONFLICT DO NOTHING"#,
        height,
        &a_addr,
        &a_txid,
        &a_pos,
      )
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(Self::NAME, "base: insert address_txs"))?;
    }

    // Special-tx payloads are rare; per-row inserts are fine.
    for btx in &block.txs {
      btx
        .insert_payload(tx, height)
        .await
        .map_err(HyDbStageError::wrap(Self::NAME))?;
    }

    Ok(())
  }

  async fn disconnect(&self, tx: &mut PgTx<'_>, height: i32) -> Result<(), HyDbStageError> {
    // Rebuild the UTXOs this block spent from the base facts: every outpoint in
    // `spends` at this height still has its creating `tx_outputs` row. Restore
    // before removing this block's own outputs, so an output created and spent
    // within the same block nets back out to absent.
    sqlx::query!(
      r#"INSERT INTO utxo_set (outpoint, address, value, block_height, script_pubkey)
         SELECT o.outpoint, o.address, o.value, o.block_height, o.script_pubkey
           FROM spends s JOIN tx_outputs o ON o.outpoint = s.outpoint
          WHERE s.spend_height = $1
         ON CONFLICT (outpoint) DO NOTHING"#,
      height
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: restore spent utxos",
    ))?;

    sqlx::query!(
      r#"DELETE FROM utxo_set WHERE outpoint IN
           (SELECT outpoint FROM tx_outputs WHERE block_height = $1)"#,
      height
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: drop created utxos",
    ))?;

    for stmt in [
      "DELETE FROM spends WHERE spend_height = $1",
      "DELETE FROM address_txs WHERE block_height = $1",
      "DELETE FROM sptx_payloads WHERE block_height = $1",
      "DELETE FROM tx_inputs WHERE block_height = $1",
      "DELETE FROM tx_outputs WHERE block_height = $1",
      "DELETE FROM transactions WHERE block_height = $1",
      "DELETE FROM blocks WHERE height = $1",
    ] {
      sqlx::query(stmt)
        .bind(height)
        .execute(&mut **tx)
        .await
        .map_err(HyDbStageError::sql(
          Self::NAME,
          "disconnect: delete base rows",
        ))?;
    }
    Ok(())
  }
}
