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

/// Decode and enrich one mempool transaction into an in-RAM [`MempoolTx`].
/// Input provenance (prev address/value) is resolved from the storage engine's
/// committed outputs.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when the transaction cannot be decoded and
/// `DaemonError::Storage` when a prevout lookup fails.
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
  let value_out = total_duffs(tx.outputs.iter().map(|output| output.value.to_sat()))?;
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
    .map(|(index, output)| {
      Ok(MempoolTxOutput {
        vout_index: index as i32,
        value: duffs(output.value.to_sat())?,
        script_pubkey: output.script_pubkey.as_bytes().to_vec(),
        address: dash_script::derive_address(
          output.script_pubkey.as_bytes(),
          p2pkh_version,
          p2sh_version,
        ),
      })
    })
    .collect::<Result<Vec<_>, DaemonError>>()?;

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
  let value_out = total_duffs(tx.outputs.iter().map(|output| output.value.to_sat()))?;
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
    .map(|(index, output)| {
      Ok(TxOutput {
        n: index as i32,
        value: duffs(output.value.to_sat())?,
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
    })
    .collect::<Result<Vec<TxOutput>, DaemonError>>()?;

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

/// A satoshi count as the signed width the API renders amounts at.
///
/// Checked rather than cast. The release profile turns off overflow checks, so
/// `as i64` on a malformed value above `i64::MAX` yields a negative amount that
/// is then summed into a fee and served as fact.
///
/// # Errors
///
/// `RestSource` when the value cannot be represented, which is the same class
/// as the decode failure it sits beside — the node sent something unusable.
fn duffs(sats: u64) -> Result<i64, DaemonError> {
  i64::try_from(sats).map_err(|_| DaemonError::RestSource {
    message: format!("transaction claims {sats} duffs, beyond the representable range"),
    kind: RestErrorKind::Other,
  })
}

/// Sum satoshi counts, refusing an overflow rather than wrapping.
///
/// # Errors
///
/// `RestSource` when a value or the running total cannot be represented.
fn total_duffs(sats: impl IntoIterator<Item = u64>) -> Result<i64, DaemonError> {
  let mut total: i64 = 0;
  for value in sats {
    total = total
      .checked_add(duffs(value)?)
      .ok_or_else(|| DaemonError::RestSource {
        message: "transaction output total is beyond the representable range".to_owned(),
        kind: RestErrorKind::Other,
      })?;
  }
  Ok(total)
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

/// Ceiling on how many inputs one mempool transaction may have resolved.
///
/// Prevout resolution runs on the **shared query pool**, the same pool serving
/// the public API, and the resolver has no batched read to work with (see the
/// module note on `load_prevouts`). A consolidation transaction with thousands
/// of inputs would therefore spend that pool on enrichment at exactly the
/// moment the transaction is interesting to everyone else. Past the cap the
/// transaction is still tracked and served — it simply carries no input
/// addresses, values, or fee, which is a visible gap rather than a stalled API.
const MAX_RESOLVED_PREVOUT_INPUTS: usize = 1_000;

/// Resolve the consumed outputs of a mempool transaction from committed chain
/// data in the storage engine.
///
/// Two costs are bounded here:
///
/// * **Repeats.** Inputs are grouped by their previous transaction, so a
/// transaction spending twenty outputs of one parent costs one lookup, not
/// twenty. `HyDbTxIdx::get` already returns every output of that parent.
/// * **Volume.** Beyond [`MAX_RESOLVED_PREVOUT_INPUTS`] inputs nothing is
/// resolved at all.
///
/// What is *not* fixed here: each distinct parent still costs its own
/// round trip, because `hyphen-db` exposes no batched outpoint read on its
/// query side. The batched `outpoint = ANY($1)` form the ingest deriver uses
/// (79 ms for 10k) is `pub(crate)` and write-pool bound; making it available
/// to this path needs a public `resolve_outpoints` on the storage crate.
/// The lookups one mempool transaction's inputs require, after grouping.
#[derive(Debug, Default, PartialEq, Eq)]
struct PrevoutPlan {
  /// One entry per distinct parent transaction, with the vouts wanted from it
  /// sorted and deduplicated.
  parents: Vec<(Vec<u8>, Vec<i32>)>,
  /// Inputs that actually spend something (coinbase inputs excluded).
  spending_inputs: usize,
  /// Whether [`MAX_RESOLVED_PREVOUT_INPUTS`] was exceeded, in which case
  /// nothing is resolved.
  over_cap: bool,
}

/// Group `(prev_txid, vout)` pairs into per-parent lookups and apply the cap.
///
/// Separated from the I/O so the two properties that matter — one lookup per
/// distinct parent, and nothing at all past the cap — are testable without a
/// database.
fn plan_prevout_lookups<'a, I>(outpoints: I) -> PrevoutPlan
where
  I: IntoIterator<Item = (&'a [u8], u32)>,
{
  let mut grouped: HashMap<Vec<u8>, Vec<i32>> = HashMap::new();
  let mut spending_inputs = 0usize;
  for (prev_txid, vout) in outpoints {
    // A coinbase input names no previous output; nothing to resolve.
    if prev_txid.iter().all(|byte| *byte == 0) {
      continue;
    }
    spending_inputs += 1;
    grouped
      .entry(prev_txid.to_vec())
      .or_default()
      .push(vout as i32);
  }
  let over_cap = spending_inputs > MAX_RESOLVED_PREVOUT_INPUTS;
  let mut parents: Vec<(Vec<u8>, Vec<i32>)> = if over_cap {
    Vec::new()
  } else {
    grouped
      .into_iter()
      .map(|(txid, mut vouts)| {
        vouts.sort_unstable();
        vouts.dedup();
        (txid, vouts)
      })
      .collect()
  };
  // Stable order so the lookups are reproducible run to run.
  parents.sort_unstable();
  PrevoutPlan {
    parents,
    spending_inputs,
    over_cap,
  }
}

async fn load_prevouts(
  db: &HyphenDb,
  tx: &DecodedTransaction,
) -> Result<HashMap<(Vec<u8>, i32), PrevOutput>, DaemonError> {
  let mut out = HashMap::new();
  let plan = plan_prevout_lookups(
    tx.inputs
      .iter()
      .map(|input| (&input.prevout.hash.as_bytes()[..], input.prevout.index)),
  );

  if plan.over_cap {
    tracing::warn!(
      inputs = plan.spending_inputs,
      cap = MAX_RESOLVED_PREVOUT_INPUTS,
      "mempool transaction exceeds the prevout-resolution cap; serving it without input provenance or fee"
    );
    return Ok(out);
  }

  for (prev_txid, vouts) in plan.parents {
    let hash = hyphen_db::HyDbHash::from_slice(&prev_txid)
      .map_err(|e| DaemonError::storage("mempool prevout hash", e))?;
    let Some(&want_max) = vouts.last() else {
      continue;
    };
    // Windowed reads rather than `txs().get()`. `get` materialises only the
    // first `HYDB_TX_IO_CAP` outputs of the parent, so an input spending a
    // higher index of a wide parent — a coinbase paying tens of thousands of
    // addresses, which mainnet has — resolved to nothing, and the transaction
    // was served with no provenance for that input and no fee at all, silently.
    //
    // Walking forward from the lowest wanted index costs one round trip for
    // every parent narrow enough to matter, which is all of them in practice.
    let mut from = u32::try_from(vouts.first().copied().unwrap_or(0)).unwrap_or(0);
    loop {
      let page = db
        .txs()
        .outputs(hash, from, hyphen_db::query::HYDB_TX_IO_CAP)
        .await
        .map_err(|e| DaemonError::storage("mempool prevout lookup", e))?;
      if page.items.is_empty() {
        break;
      }
      let mut highest_seen = from;
      for output in &page.items {
        highest_seen = u32::try_from(output.vout_index).unwrap_or(highest_seen);
        if vouts.binary_search(&output.vout_index).is_err() {
          continue;
        }
        out.insert(
          (prev_txid.clone(), output.vout_index),
          PrevOutput {
            address: output.address.as_ref().map(|address| address.0.clone()),
            value: Some(output.value),
          },
        );
      }
      let past_the_last_wanted = i64::from(highest_seen) >= i64::from(want_max);
      if past_the_last_wanted || page.next_cursor.is_none() {
        break;
      }
      from = highest_seen.saturating_add(1);
    }
  }
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::{plan_prevout_lookups, MAX_RESOLVED_PREVOUT_INPUTS};

  fn txid(byte: u8) -> [u8; 32] {
    [byte; 32]
  }

  /// C3: inputs sharing a parent must cost one lookup, not one each. The
  /// resolver ran on the shared query pool, one `txs().get()` per input.
  #[test]
  fn inputs_sharing_a_parent_collapse_into_one_lookup() {
    let a = txid(1);
    let b = txid(2);
    let plan = plan_prevout_lookups(vec![
      (a.as_slice(), 0),
      (a.as_slice(), 7),
      (a.as_slice(), 0), // a duplicated outpoint is still one wanted vout
      (b.as_slice(), 3),
    ]);
    assert_eq!(plan.spending_inputs, 4);
    assert!(!plan.over_cap);
    assert_eq!(plan.parents.len(), 2, "one lookup per distinct parent");
    assert_eq!(plan.parents[0], (a.to_vec(), vec![0, 7]));
    assert_eq!(plan.parents[1], (b.to_vec(), vec![3]));
    // Sorted, because the resolver binary-searches this.
    for (_, vouts) in &plan.parents {
      assert!(vouts.windows(2).all(|w| w[0] < w[1]));
    }
  }

  /// A coinbase input names no previous output and must not become a lookup.
  #[test]
  fn a_coinbase_input_is_not_a_lookup() {
    let null = [0u8; 32];
    let plan = plan_prevout_lookups(vec![(null.as_slice(), u32::MAX)]);
    assert_eq!(plan.spending_inputs, 0);
    assert!(plan.parents.is_empty());
    assert!(!plan.over_cap);
  }

  /// Past the cap the whale transaction costs the shared query pool nothing.
  #[test]
  fn a_whale_transaction_resolves_nothing() {
    let parents: Vec<[u8; 32]> = (0..=MAX_RESOLVED_PREVOUT_INPUTS)
      .map(|i| {
        let mut hash = [0u8; 32];
        hash[0] = 1;
        hash[1..5].copy_from_slice(&(i as u32).to_le_bytes());
        hash
      })
      .collect();
    let plan = plan_prevout_lookups(parents.iter().map(|hash| (hash.as_slice(), 0)));
    assert_eq!(plan.spending_inputs, MAX_RESOLVED_PREVOUT_INPUTS + 1);
    assert!(plan.over_cap);
    assert!(
      plan.parents.is_empty(),
      "over the cap nothing may be looked up"
    );

    // One under the cap still resolves in full.
    let plan = plan_prevout_lookups(
      parents
        .iter()
        .take(MAX_RESOLVED_PREVOUT_INPUTS)
        .map(|hash| (hash.as_slice(), 0)),
    );
    assert!(!plan.over_cap);
    assert_eq!(plan.parents.len(), MAX_RESOLVED_PREVOUT_INPUTS);
  }
}
