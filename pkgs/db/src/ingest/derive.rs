//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The read-only fact-derivation layer. Given a block's raw bytes, the
//! [`PgDeriver`] decodes it, resolves every non-coinbase input against this
//! block's own outputs then the persisted UTXO set (one batched `= ANY`
//! query, never per-input probes), accumulates chainwork from the parent
//! block's stored value, and records the per-address deltas, per-tx fees,
//! spent outpoints, and cumulative metric inputs on the [`Scratch`]. It
//! performs **no writes** — every mutation and its undo live in the
//! [`IngestStage`s](super::pipeline::IngestStage) — but it runs *inside* the
//! block's write transaction so reorg re-derivation sees the branch state the
//! transaction has already built.

use std::collections::{HashMap, HashSet};

use async_trait::async_trait;

use super::pipeline::IngestDerive;
use super::scratch::{AddrDelta, HyDbDeriveError, MetricInputs, Scratch};
use crate::records::{HyDbDataSrc, PgTx, SpentOutput};
use crate::types::HyDbOutpoint;
use crate::util::accumulate_chainwork;

/// The Postgres-backed deriver: decode via the caller's data source, resolve
/// spends read-only, fill the scratch.
pub(super) struct PgDeriver;

#[async_trait]
impl IngestDerive for PgDeriver {
  async fn derive(
    &self,
    tx: &mut PgTx<'_>,
    data_src: &dyn HyDbDataSrc,
    scratch: &mut Scratch,
  ) -> Result<(), HyDbDeriveError> {
    let height = scratch.height;
    let raw_len = scratch.raw.len() as i64;
    let block = data_src
      .decode(&scratch.raw)
      .map_err(|e| HyDbDeriveError::Decode(e.to_string()))?;

    // Chainwork: parent's cumulative work (from the canonical row below this
    // block, which a reorg transaction has already put in place) plus this
    // block's proof from `bits`.
    let prev_chainwork = if height == 0 {
      None
    } else {
      sqlx::query_scalar!("SELECT chainwork FROM blocks WHERE height = $1", height - 1)
        .fetch_optional(&mut **tx)
        .await
        .map_err(|e| HyDbDeriveError::pg("derive: read parent chainwork", e))?
        .flatten()
    };
    let chainwork = accumulate_chainwork(prev_chainwork.as_deref(), block.header.bits);

    // This block's own outputs, keyed by packed outpoint, so an input can
    // resolve an output created earlier in the same block. The node has
    // already consensus-validated ordering, so building the full map up front
    // is safe.
    //
    // An outpoint is *not* globally unique — a txid can recur across blocks
    // before BIP30 enforcement, which `query::txs` pins a height to cope with —
    // but it is unique within a block, so this map cannot collide with itself,
    // and a lookup here shadowing the persisted set is the intended precedence:
    // an output created in this block is the one this block's inputs spend.
    let mut block_outputs: HashMap<[u8; 36], SpentOutput> = HashMap::new();
    for btx in &block.txs {
      for out in &btx.outputs {
        let outpoint = HyDbOutpoint {
          txid: btx.txid,
          vout: out.vout,
        }
        .packed();
        block_outputs.insert(
          outpoint,
          SpentOutput {
            address: out.address.as_ref().map(|a| a.0.clone()),
            value: out.value.0,
          },
        );
      }
    }

    // Batch-resolve every prevout not created in this block: one `= ANY`
    // round trip instead of one probe per input.
    let mut to_resolve: Vec<Vec<u8>> = Vec::new();
    for btx in &block.txs {
      for inp in &btx.inputs {
        if inp.coinbase {
          continue;
        }
        let Some(prev) = inp.prev_outpoint else {
          return Err(HyDbDeriveError::Malformed {
            what: format!(
              "non-coinbase input {}:{} has no prev_outpoint",
              hex::encode(btx.txid.0),
              inp.vin_index
            ),
          });
        };
        let packed = prev.packed();
        if !block_outputs.contains_key(&packed) {
          to_resolve.push(packed.to_vec());
        }
      }
    }
    let mut persisted: HashMap<Vec<u8>, SpentOutput> = HashMap::new();
    if !to_resolve.is_empty() {
      let rows = sqlx::query!(
        r#"SELECT outpoint, address, value FROM utxo_set WHERE outpoint = ANY($1)"#,
        &to_resolve
      )
      .fetch_all(&mut **tx)
      .await
      .map_err(|e| HyDbDeriveError::pg("derive: resolve prevouts", e))?;
      for r in rows {
        persisted.insert(
          r.outpoint,
          SpentOutput {
            address: r.address,
            value: r.value,
          },
        );
      }
    }

    // Per-input resolved prevouts (`[tx][vin]`, `None` for coinbase) and per-tx
    // fees (`None` for the coinbase), aligned to the block's tx/input order.
    let mut resolved: Vec<Vec<Option<SpentOutput>>> = Vec::with_capacity(block.txs.len());
    let mut fees: Vec<Option<i64>> = Vec::with_capacity(block.txs.len());
    let mut deltas: HashMap<String, AddrDelta> = HashMap::new();
    let mut spent_outpoints: Vec<HyDbOutpoint> = Vec::new();
    // Every outpoint this block spends, so a second spend of one is a refusal
    // rather than a double count.
    let mut spent_once: HashSet<[u8; 36]> = HashSet::new();

    // Cumulative metric inputs for this block.
    let mut spendable_created: i64 = 0;
    let mut unspendable_created: i64 = 0;
    let mut inputs_spent: i64 = 0;
    let mut coinbase_value: i64 = 0;
    let mut fee_total: i64 = 0;
    let mut unspendable_delta: i64 = 0;

    for btx in &block.txs {
      let txid = btx.txid.0.to_vec();

      // Outputs: register creation counts and fold address deltas.
      for out in &btx.outputs {
        match out.address.as_ref() {
          Some(addr) => {
            spendable_created += 1;
            let d = deltas.entry(addr.0.clone()).or_default();
            d.received += out.value.0;
            d.created_utxos += 1;
            d.txs.insert(txid.clone());
          }
          None => {
            unspendable_created += 1;
            unspendable_delta += out.value.0;
          }
        }
      }

      // Inputs: resolve each non-coinbase prevout; a coinbase carries none.
      let mut value_in: i64 = 0;
      let mut coinbase_tx = false;
      let mut row: Vec<Option<SpentOutput>> = Vec::with_capacity(btx.inputs.len());

      for inp in &btx.inputs {
        if inp.coinbase {
          coinbase_tx = true;
          row.push(None);
          continue;
        }
        // Presence of prev_outpoint was validated in the collection pass.
        let Some(prev) = inp.prev_outpoint else {
          return Err(HyDbDeriveError::Malformed {
            what: "input lost its prev_outpoint between passes".to_owned(),
          });
        };
        let packed = prev.packed();
        // One outpoint may be spent once per block. The resolution above is a
        // lookup, not a consumption, so a block spending the same outpoint from
        // two inputs would resolve both and count the value, the UTXO and the
        // fee contribution twice — leaving the address balance and the supply
        // wrong with nothing failing.
        //
        // Consensus forbids it and the node validates before this sees a block,
        // so this cannot fire against a healthy source. It is here because the
        // cost is one hash per input and the alternative is silent corruption
        // if that assumption ever stops holding.
        claim_spend(&mut spent_once, packed, &prev, height)?;
        let spent = match block_outputs.get(&packed) {
          Some(s) => s.clone(),
          None => persisted
            .get(&packed[..])
            .cloned()
            .ok_or(HyDbDeriveError::MissingPrevout {
              outpoint: prev,
              height,
            })?,
        };

        spent_outpoints.push(prev);
        value_in += spent.value;
        inputs_spent += 1;
        match &spent.address {
          Some(addr) => {
            let d = deltas.entry(addr.clone()).or_default();
            d.sent += spent.value;
            d.spent_utxos += 1;
            d.txs.insert(txid.clone());
          }
          None => unspendable_delta -= spent.value,
        }
        row.push(Some(spent));
      }

      resolved.push(row);
      if coinbase_tx {
        coinbase_value += btx.value_out.0;
        fees.push(None);
      } else {
        let fee = value_in - btx.value_out.0;
        fee_total += fee;
        fees.push(Some(fee));
      }
    }

    let metrics = MetricInputs {
      spendable_created,
      unspendable_created,
      inputs_spent,
      coinbase_value,
      fee_total,
      unspendable_delta,
      raw_len,
    };
    scratch.fill(
      block,
      resolved,
      fees,
      deltas,
      spent_outpoints,
      metrics,
      chainwork,
    );
    Ok(())
  }
}

/// Record that this block spends `packed`, refusing a second claim on it.
///
/// Extracted from the derive walk so the refusal can be tested. It guards the
/// system's least forgiving invariant — two inputs consuming one outpoint in
/// one block would count the value, the UTXO and the fee twice, leaving the
/// balance and the supply wrong with nothing failing — and inside the walk it
/// was unreachable from a test, because the integration fixture cannot build a
/// block with two spends of the same outpoint. Deleting the check therefore
/// left the whole suite green.
///
/// Consensus forbids the input and the node validates before this sees a
/// block, so it cannot fire against a healthy source. It exists because the
/// cost is a hash per input and the alternative is silent corruption if that
/// assumption ever stops holding.
///
/// # Errors
///
/// [`HyDbDeriveError::Malformed`] naming the outpoint and the block.
fn claim_spend(
  seen: &mut HashSet<[u8; 36]>,
  packed: [u8; 36],
  prev: &HyDbOutpoint,
  height: i32,
) -> Result<(), HyDbDeriveError> {
  if seen.insert(packed) {
    return Ok(());
  }
  Err(HyDbDeriveError::Malformed {
    what: format!("outpoint {prev:?} is spent twice in block {height}"),
  })
}

#[cfg(test)]
mod tests {
  use super::*;

  fn outpoint(txid_byte: u8, vout: u32) -> HyDbOutpoint {
    HyDbOutpoint {
      txid: crate::types::HyDbHash([txid_byte; 32]),
      vout,
    }
  }

  /// The same outpoint twice in one block is refused; different outpoints are
  /// not. The `vout` cases matter because the packed key carries it in the
  /// trailing four bytes, so a guard keyed on the txid alone would reject two
  /// legitimate spends of different outputs of the same transaction — which is
  /// ordinary, and would halt the chain.
  #[test]
  fn one_outpoint_may_be_spent_once_per_block() {
    let mut seen = HashSet::new();

    let a = outpoint(0xAA, 0);
    assert!(claim_spend(&mut seen, a.packed(), &a, 100).is_ok());

    // Same outpoint again: refused, and the message names both the outpoint
    // and the height so the offending block is identifiable from the log.
    let err = claim_spend(&mut seen, a.packed(), &a, 100)
      .expect_err("a second claim on one outpoint must be refused");
    let HyDbDeriveError::Malformed { what } = err else {
      panic!("a double spend must be reported as malformed, got {err:?}");
    };
    assert!(what.contains("spent twice"), "{what}");
    assert!(what.contains("100"), "{what}");

    // A different vout of the same transaction is a different outpoint.
    let a1 = outpoint(0xAA, 1);
    assert!(claim_spend(&mut seen, a1.packed(), &a1, 100).is_ok());

    // And a different transaction entirely.
    let b = outpoint(0xBB, 0);
    assert!(claim_spend(&mut seen, b.packed(), &b, 100).is_ok());
  }
}
