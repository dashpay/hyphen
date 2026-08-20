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

/// Hard bound on how many inputs, and how many outputs, a single
/// [`HyDbTxIdx::get`] materializes.
///
/// A transaction's input and output counts are attacker-chosen up to the block
/// size limit: mainnet holds coinbases with tens of thousands of outputs, and a
/// summary render of one of those used to cost a row per output. `get` is the
/// endpoint behind "show me this transaction", so it now returns the first
/// `HYDB_TX_IO_CAP` of each side and nothing more.
///
/// A row per output is not the whole cost, and the measurement is worse than
/// the row count suggests. `spends` is partitioned by *spend* height while the
/// join key is the outpoint, which carries no spend height — so the planner
/// cannot prune, and every single output probes all nine partitions. On the
/// 860,000-block seed, the widest transaction (2,467 outputs) cost 22,203 index
/// searches and 81,026 buffer accesses, 8.16 s cold. Capped at 1,000 the same
/// query is 9,000 searches and 32,854 buffers: ~33 buffer accesses per output,
/// linear in the cap, which is exactly what a bound is for.
///
/// Extrapolated, that is the argument for having one at all: an unbounded
/// 40,000-output transaction would touch ~1.3M buffers — around 10 GB of buffer
/// traffic through an 8 GB `shared_buffers`, evicting the working set of every
/// other query on the server to render one page. The cap holds it to ~33,000.
/// (The nine-partition fan-out itself is a separate defect and a much larger
/// win if fixed; it is not addressed here.)
///
/// The bound is discoverable without a second query and without new fields on
/// [`HyDbQrTxDetail`]: a side whose vector came back with exactly
/// `HYDB_TX_IO_CAP` entries is *possibly* truncated, and the rest of it is read
/// with [`HyDbTxIdx::inputs`] / [`HyDbTxIdx::outputs`] from index
/// `HYDB_TX_IO_CAP` onwards. It is deliberately far above any normal
/// transaction (a 1000-output transaction is already ~35 KB of one block), so
/// the response shape of ordinary traffic is unchanged.
pub const HYDB_TX_IO_CAP: u32 = 1000;

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

/// A transaction hit from a prefix search: the txid plus enough positional
/// detail (`block_pos`) for a caller to derive `coinbase` (`block_pos == 0`)
/// without a second round trip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyDbQrTxRef {
  pub txid: HyDbHash,
  pub block_pos: i32,
}

/// Query handle for transaction data.
#[async_trait]
pub trait HyDbTxIdx: Send + Sync {
  /// Full detail for a transaction, resolving inputs and output spent-status.
  ///
  /// Bounded: at most [`HYDB_TX_IO_CAP`] inputs and [`HYDB_TX_IO_CAP`] outputs,
  /// lowest index first. See that constant for how a caller detects the bound
  /// and reads past it.
  async fn get(&self, txid: HyDbHash) -> HyDbResult<Option<HyDbQrTxDetail>>;

  /// A window of a transaction's inputs, `limit` of them starting at
  /// `from_index`, ascending by `vin_index`.
  ///
  /// `vin_index` is dense from zero, so `from_index` is both the offset and the
  /// key: the scan starts at the right place in the index instead of counting
  /// rows up to it. `next_cursor` is the decimal index the following page
  /// starts at, and is `None` on the last page. An unknown transaction yields
  /// an empty terminal page, matching `get`'s `None`.
  ///
  /// `limit` is clamped to `1..=`[`HYDB_TX_IO_CAP`].
  ///
  /// # Errors
  ///
  /// Propagates query failure.
  async fn inputs(
    &self,
    txid: HyDbHash,
    from_index: u32,
    limit: u32,
  ) -> HyDbResult<HyDbPage<HyDbQrTxIn>> {
    Ok(io_window_from_detail(
      self.get(txid).await?.map(|d| d.inputs).unwrap_or_default(),
      from_index,
      limit,
    ))
  }

  /// A window of a transaction's outputs, `limit` of them starting at
  /// `from_index`, ascending by `vout_index`. Spent-status is resolved for the
  /// window only. Cursor and clamping follow [`HyDbTxIdx::inputs`].
  ///
  /// # Errors
  ///
  /// Propagates query failure.
  async fn outputs(
    &self,
    txid: HyDbHash,
    from_index: u32,
    limit: u32,
  ) -> HyDbResult<HyDbPage<HyDbQrTxOut>> {
    Ok(io_window_from_detail(
      self.get(txid).await?.map(|d| d.outputs).unwrap_or_default(),
      from_index,
      limit,
    ))
  }

  /// Newest-first page of transactions across the chain.
  async fn list(
    &self,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrTxSummary>>;

  /// Total number of canonical transactions (precomputed).
  async fn total(&self) -> HyDbResult<u64>;

  /// Txids (with `block_pos`) whose DISPLAY-order hex (what the UI renders)
  /// starts with `prefix`, most-recent first. Lowercased and `LIKE`-escaped
  /// internally.
  async fn search_prefix(&self, prefix: String, limit: u32) -> HyDbResult<Vec<HyDbQrTxRef>>;
}

/// Clamp a caller's page size into `1..=`[`HYDB_TX_IO_CAP`], as an `i64` for
/// `LIMIT`. One extra row is fetched to decide whether a next page exists.
fn io_limit(limit: u32) -> i64 {
  i64::from(limit.clamp(1, HYDB_TX_IO_CAP))
}

/// Split an over-fetched window (`limit + 1` rows) into a page plus the cursor
/// naming where the next one starts.
fn io_page<T>(mut rows: Vec<T>, from_index: u32, limit: i64) -> HyDbPage<T> {
  let limit = limit as usize;
  if rows.len() > limit {
    rows.truncate(limit);
    let next = u64::from(from_index) + limit as u64;
    return HyDbPage::new(rows, Some(HyDbCursor(next.to_string())));
  }
  HyDbPage::last(rows)
}

/// The window a caller asked for, taken from an already-materialized side of a
/// transaction. Backs the default trait impls of `inputs`/`outputs`, where the
/// whole vector is already in hand and the database cannot be re-queried.
fn io_window_from_detail<T: Clone>(all: Vec<T>, from_index: u32, limit: u32) -> HyDbPage<T> {
  let limit = io_limit(limit);
  let start = from_index as usize;
  let taken: Vec<T> = all
    .into_iter()
    .skip(start)
    .take(limit as usize + 1)
    .collect();
  io_page(taken, from_index, limit)
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

impl PgTxIdx {
  /// The height of the occurrence of `txid` this handle serves.
  ///
  /// A txid can (pre-BIP30 enforcement) recur across blocks; the most recent
  /// occurrence is the one every part of a read resolves against. Pinning the
  /// height also prunes the partition fan-out to one partition and keeps a
  /// duplicated txid from concatenating both occurrences' rows.
  async fn height_of(&self, key: &[u8]) -> HyDbResult<Option<i32>> {
    sqlx::query_scalar!(
      "SELECT block_height FROM transactions WHERE txid = $1 ORDER BY block_height DESC LIMIT 1",
      key
    )
    .fetch_optional(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("transaction height", e))
  }

  /// `limit` inputs of `(txid, height)` from `from_index` upward.
  async fn fetch_inputs(
    &self,
    key: &[u8],
    height: i32,
    from_index: u32,
    limit: i64,
  ) -> HyDbResult<Vec<HyDbQrTxIn>> {
    let from = i64::from(from_index);
    let rows = sqlx::query!(
      r#"SELECT vin_index, prev_txid, prev_vout, prev_address, prev_value, script_sig, sequence
           FROM tx_inputs
          WHERE txid = $1 AND block_height = $2 AND vin_index >= $3
          ORDER BY vin_index LIMIT $4"#,
      key,
      height,
      i32::try_from(from).unwrap_or(i32::MAX),
      limit
    )
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("transaction inputs", e))?;

    let mut inputs = Vec::with_capacity(rows.len());
    for r in rows {
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
    Ok(inputs)
  }

  /// `limit` outputs of `(txid, height)` from `from_index` upward, each with
  /// its spent-status resolved through the append-only `spends` table.
  async fn fetch_outputs(
    &self,
    key: &[u8],
    height: i32,
    from_index: u32,
    limit: i64,
  ) -> HyDbResult<Vec<HyDbQrTxOut>> {
    let from = i64::from(from_index);
    // The window is taken on `tx_outputs` before the `spends` join, so the
    // join runs against at most `limit` rows rather than the whole output set.
    //
    // The spend is picked laterally and bounded below by this occurrence's
    // height. An outpoint is unique only within an occurrence: a txid that
    // recurs — which this module pins a height precisely because it can —
    // produces the same 36-byte outpoint twice, so a plain equi-join on the
    // outpoint alone matched the *other* occurrence's spend row. Serving the
    // newest occurrence, that reported an unspent output as spent, and where
    // both occurrences were spent it returned the output twice, because the
    // join multiplied the row it was meant to annotate.
    //
    // Written against the runtime API so it needs no offline-cache entry.
    type OutRow = (
      i32,
      i64,
      Vec<u8>,
      Option<String>,
      Option<HyDbAddrType>,
      Option<Vec<u8>>,
      Option<i32>,
    );
    let rows: Vec<OutRow> = sqlx::query_as(
      "SELECT o.vout_index, o.value, o.script_pubkey, o.address, o.address_type, \
              s.spending_txid, s.spending_vin \
         FROM (SELECT vout_index, value, script_pubkey, address, address_type, outpoint \
                 FROM tx_outputs \
                WHERE txid = $1 AND block_height = $2 AND vout_index >= $3 \
                ORDER BY vout_index LIMIT $4) o \
         LEFT JOIN LATERAL ( \
              SELECT sp.spending_txid, sp.spending_vin \
                FROM spends sp \
               WHERE sp.outpoint = o.outpoint AND sp.spend_height >= $2 \
               ORDER BY sp.spend_height, sp.spending_txid, sp.spending_vin \
               LIMIT 1) s ON TRUE \
        ORDER BY o.vout_index",
    )
    .bind(key)
    .bind(height)
    .bind(i32::try_from(from).unwrap_or(i32::MAX))
    .bind(limit)
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("transaction outputs", e))?;

    let mut outputs = Vec::with_capacity(rows.len());
    for (vout_index, value, script_pubkey, address, address_type, spending_txid, spending_vin) in
      rows
    {
      outputs.push(HyDbQrTxOut {
        vout_index,
        value,
        address: address.map(HyDbAddress),
        address_type,
        script_pubkey,
        spent_by: outpoint(spending_txid, spending_vin)?,
      });
    }
    Ok(outputs)
  }
}

#[async_trait]
impl HyDbTxIdx for PgTxIdx {
  async fn get(&self, txid: HyDbHash) -> HyDbResult<Option<HyDbQrTxDetail>> {
    let key = &txid.0[..];

    // The transaction header, of the most recent occurrence (see `height_of`).
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

    // Inputs, outputs and payload all carry the chosen header's height, and
    // both io sides are bounded by HYDB_TX_IO_CAP: a 40k-output transaction
    // renders from 1000 rows, not 40k.
    let cap = i64::from(HYDB_TX_IO_CAP);
    let inputs = self.fetch_inputs(key, tx.block_height, 0, cap).await?;
    let outputs = self.fetch_outputs(key, tx.block_height, 0, cap).await?;

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

  async fn inputs(
    &self,
    txid: HyDbHash,
    from_index: u32,
    limit: u32,
  ) -> HyDbResult<HyDbPage<HyDbQrTxIn>> {
    let key = &txid.0[..];
    let Some(height) = self.height_of(key).await? else {
      return Ok(HyDbPage::last(Vec::new()));
    };
    let limit = io_limit(limit);
    let rows = self
      .fetch_inputs(key, height, from_index, limit + 1)
      .await?;
    Ok(io_page(rows, from_index, limit))
  }

  async fn outputs(
    &self,
    txid: HyDbHash,
    from_index: u32,
    limit: u32,
  ) -> HyDbResult<HyDbPage<HyDbQrTxOut>> {
    let key = &txid.0[..];
    let Some(height) = self.height_of(key).await? else {
      return Ok(HyDbPage::last(Vec::new()));
    };
    let limit = io_limit(limit);
    let rows = self
      .fetch_outputs(key, height, from_index, limit + 1)
      .await?;
    Ok(io_page(rows, from_index, limit))
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
        t.block_height.get(),
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

  async fn search_prefix(&self, prefix: String, limit: u32) -> HyDbResult<Vec<HyDbQrTxRef>> {
    let limit = limit.clamp(1, 100) as i64;
    // DISPLAY-order hex match (see the search contract in the schema), backed
    // by the display_hex expression index.
    //
    // No ORDER BY, deliberately: `LIMIT n` lets the index scan stop after n
    // matches, while ordering by recency forces every match to be enumerated
    // and heap-fetched first. On the retained mainnet database a 4-character
    // prefix costs 39-48 ms ordered against 0.4 ms unordered, and txids are
    // random, so "most recent among prefix matches" is arbitrary UX anyway.
    //
    // `block_pos` is selected alongside `txid` so a caller can derive
    // `coinbase` (`block_pos == 0`) itself, the same way the exact-match path
    // does — a prefix hit is not missing that detail, it just used to not ask.
    let pattern = hex_prefix_pattern(&prefix);
    let rows = sqlx::query!(
      r#"SELECT txid AS "txid!", block_pos AS "block_pos!" FROM transactions
          WHERE display_hex(txid) LIKE $1 ESCAPE '\' LIMIT $2"#,
      pattern,
      limit
    )
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("search transactions by prefix", e))?;
    rows
      .iter()
      .map(|r| {
        Ok(HyDbQrTxRef {
          txid: HyDbHash::from_slice(&r.txid)?,
          block_pos: r.block_pos,
        })
      })
      .collect()
  }
}
