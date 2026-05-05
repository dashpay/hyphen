//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block-scoped spent/address write planning.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::sync::Mutex;
use std::time::Instant;

use crate::error::DaemonError;
use crate::index::batch::{BatchSizer, DEFAULT_MIN_BATCH_SIZE};
use crate::index::runner::{ApplyStagePlanSubstage, PlanConnectSubstage, Substage};

use super::{sql, BlockCtx};

const DEFAULT_SPENT_WRITE_CHUNK_SIZE: usize = 1_000;
const PREVOUT_APPLY_CHUNK_SIZE: usize = 512;
const TX_INPUT_UPDATE_CHUNK_SIZE: usize = 2_000;
const FEE_UPDATE_CHUNK_SIZE: usize = 2_000;
const SPENT_SUBSTEP_SLOW_MS: u64 = 1_000;

/// Per-address accumulator.
#[derive(Default, Clone)]
struct AddrAccum {
  received: i64,
  sent: i64,
  utxo_count: i32,
  txids: HashSet<([u8; 32], i32, i32)>,
}

#[derive(Clone)]
struct InputRef {
  txid: [u8; 32],
  vin: i32,
  prev_txid: Vec<u8>,
  prev_vout: i32,
  height: i32,
  block_pos: i32,
}

#[derive(Default)]
struct TxInputUpdates {
  txids: Vec<Vec<u8>>,
  vins: Vec<i32>,
  addrs: Vec<Option<String>>,
  values: Vec<i64>,
}

#[derive(Default)]
struct SpentSnapshotRows {
  prev_txids: Vec<Vec<u8>>,
  prev_vouts: Vec<i32>,
  prev_heights: Vec<i32>,
  prev_spent_txids: Vec<Option<Vec<u8>>>,
  prev_spent_vins: Vec<Option<i32>>,
  prev_spent_heights: Vec<Option<i32>>,
  spending_txids: Vec<Vec<u8>>,
  spending_vins: Vec<i32>,
  heights: Vec<i32>,
}

#[derive(Default)]
struct FeeUpdates {
  txids: Vec<Vec<u8>>,
  fees: Vec<i64>,
}

#[derive(Default)]
struct AddressHistoryRows {
  addrs: Vec<String>,
  txids: Vec<Vec<u8>>,
  block_heights: Vec<i32>,
  block_positions: Vec<i32>,
}

#[derive(Default)]
struct AddressStatsRows {
  addrs: Vec<String>,
  balances: Vec<i64>,
  total_received: Vec<i64>,
  tx_counts: Vec<i32>,
  utxo_counts: Vec<i32>,
  first_seen_heights: Vec<Option<i32>>,
  last_seen_heights: Vec<Option<i32>>,
}

#[derive(Default)]
struct SpentApplyMetrics {
  spent_prevout_lookup_ms: u64,
  spent_history_ms: u64,
  spent_mark_outputs_ms: u64,
  spent_update_inputs_ms: u64,
  spent_update_fees_ms: u64,
  spent_address_history_ms: u64,
  spent_address_stats_ms: u64,
  spent_address_balances_prepare_ms: u64,
  spent_address_balances_stage_ms: u64,
  spent_address_balances_apply_ms: u64,
}

pub(crate) struct SpentBatchPlan {
  history_height: u32,
  history_hash: [u8; 32],
  chunk_size: usize,
  input_refs: Vec<InputRef>,
  addr_deltas: HashMap<String, AddrAccum>,
  tx_value_out: Vec<TxValueOutRow>,
}

impl SpentBatchPlan {
  fn log_substep_elapsed(
    height: u32,
    substage: impl Substage,
    elapsed_ms: u64,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
    rows: usize,
  ) {
    if elapsed_ms >= SPENT_SUBSTEP_SLOW_MS {
      tracing::warn!(
        height,
        substep = substage.name(),
        elapsed_ms,
        chunk_index,
        chunk_total,
        rows,
        "spent substep is slow"
      );
    } else {
      tracing::debug!(
        height,
        substep = substage.name(),
        elapsed_ms,
        chunk_index,
        chunk_total,
        rows,
        "spent substep complete"
      );
    }
  }

  fn activity<S: Substage>(
    reporter: Option<&crate::index::runner::ForwardRunReporter>,
    substage: S,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
  ) {
    if let Some(reporter) = reporter {
      reporter.activity(substage, chunk_index, chunk_total);
    }
  }

  fn progress<S: Substage>(
    reporter: Option<&crate::index::runner::ForwardRunReporter>,
    substage: S,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
  ) {
    if let Some(reporter) = reporter {
      reporter.progress(substage, chunk_index, chunk_total);
    }
  }

  pub(crate) fn history_height(&self) -> u32 {
    self.history_height
  }

  pub(crate) async fn build(
    block: &BlockCtx,
    p2pkh_version: u8,
    p2sh_version: u8,
    batch_sizer: &Mutex<BatchSizer>,
    reporter: Option<&crate::index::runner::ForwardRunReporter>,
  ) -> Result<Self, DaemonError> {
    // Planning stays structural. Any storage-backed hydration proportional to
    // block data belongs in apply, where work can run in bounded,
    // interruptible windows.
    let history_height = block.height();
    let chunk_size = current_chunk_size(batch_sizer);
    let progress_step = spent_plan_progress_step(chunk_size);
    Self::activity(
      reporter,
      PlanConnectSubstage::SpentCollectInputs,
      None,
      None,
    );
    let collect_started = Instant::now();
    let (mut input_refs, addr_deltas, tx_value_out) = collect_spent_inputs(
      block,
      p2pkh_version,
      p2sh_version,
      history_height,
      progress_step,
      reporter,
    )?;
    Self::log_substep_elapsed(
      history_height,
      PlanConnectSubstage::SpentCollectInputs,
      collect_started.elapsed().as_millis() as u64,
      None,
      None,
      input_refs.len(),
    );
    Self::progress(
      reporter,
      PlanConnectSubstage::SpentCollectInputs,
      None,
      None,
    );
    Self::activity(reporter, PlanConnectSubstage::SpentSortInputs, None, None);
    let sort_started = Instant::now();
    sort_input_refs_by_prevout(&mut input_refs);
    Self::log_substep_elapsed(
      history_height,
      PlanConnectSubstage::SpentSortInputs,
      sort_started.elapsed().as_millis() as u64,
      None,
      None,
      input_refs.len(),
    );
    Self::progress(reporter, PlanConnectSubstage::SpentSortInputs, None, None);

    Ok(Self {
      history_height,
      history_hash: block.block_hash(),
      chunk_size,
      input_refs,
      addr_deltas,
      tx_value_out,
    })
  }

  pub(crate) async fn apply(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    prevout_db: &rocksdb::DB,
    address_history_db: Option<&rocksdb::DB>,
    persist_pg_address_history: bool,
    reporter: Option<&crate::index::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    let apply_started = Instant::now();
    let mut metrics = SpentApplyMetrics::default();
    let mut addr_deltas = self.addr_deltas.clone();
    let mut tx_input_totals = HashMap::new();
    if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
      return Err(DaemonError::Index("forward run interrupted".to_owned()));
    }
    sql::prepare_prevout_stage(tx).await?;
    let spent_chunk_total = self
      .input_refs
      .chunks(spent_lookup_window_size(self.chunk_size))
      .len()
      .max(1) as u32;
    let mut staged_prevout_offset = 0_i64;
    for (chunk_index, input_chunk) in self
      .input_refs
      .chunks(spent_lookup_window_size(self.chunk_size))
      .enumerate()
    {
      if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
        return Err(DaemonError::Index("forward run interrupted".to_owned()));
      }
      let chunk_number = (chunk_index + 1) as u32;
      let prev_txids: Vec<Vec<u8>> = input_chunk
        .iter()
        .map(|input| input.prev_txid.clone())
        .collect();
      let prev_vouts: Vec<i32> = input_chunk.iter().map(|input| input.prev_vout).collect();
      Self::activity(
        reporter,
        ApplyStagePlanSubstage::SpentPrevoutLookup,
        Some(chunk_number),
        Some(spent_chunk_total),
      );
      let started = Instant::now();
      let prev_lookup = crate::store::prevout::load_rows_interruptible(
        prevout_db,
        &prev_txids,
        &prev_vouts,
        spent_lookup_window_size(self.chunk_size),
        || reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested),
      )?;
      let elapsed_ms = started.elapsed().as_millis() as u64;
      metrics.spent_prevout_lookup_ms += elapsed_ms;
      Self::log_substep_elapsed(
        self.history_height,
        ApplyStagePlanSubstage::SpentPrevoutLookup,
        elapsed_ms,
        Some(chunk_number),
        Some(spent_chunk_total),
        prev_lookup.len(),
      );
      Self::progress(
        reporter,
        ApplyStagePlanSubstage::SpentPrevoutLookup,
        Some(chunk_number),
        Some(spent_chunk_total),
      );
      let (spent_snapshots, tx_input_updates) = hydrate_spent_chunk(
        input_chunk,
        prev_lookup,
        &mut addr_deltas,
        &mut tx_input_totals,
      );
      let chunk_rows = spent_snapshots.spending_txids.len();
      if chunk_rows == 0 {
        tokio::task::yield_now().await;
        continue;
      }
      Self::activity(
        reporter,
        ApplyStagePlanSubstage::SpentHistory,
        Some(chunk_number),
        Some(spent_chunk_total),
      );
      let started = Instant::now();
      sql::insert_spent_output_history(
        tx,
        self.history_height,
        &spent_snapshots.prev_txids,
        &spent_snapshots.prev_vouts,
        &spent_snapshots.prev_spent_txids,
        &spent_snapshots.prev_spent_vins,
        &spent_snapshots.prev_spent_heights,
        &spent_snapshots.spending_txids,
        &spent_snapshots.spending_vins,
      )
      .await?;
      let elapsed_ms = started.elapsed().as_millis() as u64;
      metrics.spent_history_ms += elapsed_ms;
      Self::log_substep_elapsed(
        self.history_height,
        ApplyStagePlanSubstage::SpentHistory,
        elapsed_ms,
        Some(chunk_number),
        Some(spent_chunk_total),
        chunk_rows,
      );
      Self::progress(
        reporter,
        ApplyStagePlanSubstage::SpentHistory,
        Some(chunk_number),
        Some(spent_chunk_total),
      );
      sql::stage_prevout_snapshot_rows(
        tx,
        &spent_snapshots.prev_txids,
        &spent_snapshots.prev_vouts,
        &spent_snapshots.prev_heights,
        &spent_snapshots.spending_txids,
        &spent_snapshots.spending_vins,
        &spent_snapshots.heights,
        &spent_snapshots.prev_spent_txids,
        &spent_snapshots.prev_spent_vins,
        &spent_snapshots.prev_spent_heights,
        &tx_input_updates.addrs,
        &tx_input_updates.values,
      )
      .await?;
      let staged_prevouts = spent_snapshots.prev_txids.len();
      let prevout_chunks = staged_prevouts.div_ceil(PREVOUT_APPLY_CHUNK_SIZE);
      for (index, start) in (0..staged_prevouts)
        .step_by(PREVOUT_APPLY_CHUNK_SIZE)
        .enumerate()
      {
        if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
          return Err(DaemonError::Index("forward run interrupted".to_owned()));
        }
        Self::activity(
          reporter,
          ApplyStagePlanSubstage::SpentMarkOutputs,
          Some((index + 1) as u32),
          Some(prevout_chunks as u32),
        );
        let end = (start + PREVOUT_APPLY_CHUNK_SIZE).min(staged_prevouts);
        let started = Instant::now();
        sql::mark_outputs_spent_chunk(
          tx,
          staged_prevout_offset + start as i64,
          staged_prevout_offset + end as i64,
        )
        .await?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        metrics.spent_mark_outputs_ms += elapsed_ms;
        Self::log_substep_elapsed(
          self.history_height,
          ApplyStagePlanSubstage::SpentMarkOutputs,
          elapsed_ms,
          Some((index + 1) as u32),
          Some(prevout_chunks as u32),
          end - start,
        );
        Self::progress(
          reporter,
          ApplyStagePlanSubstage::SpentMarkOutputs,
          Some((index + 1) as u32),
          Some(prevout_chunks as u32),
        );
      }
      staged_prevout_offset += staged_prevouts as i64;

      let tx_input_chunks = tx_input_updates
        .txids
        .len()
        .div_ceil(TX_INPUT_UPDATE_CHUNK_SIZE);
      for (index, (txids_c, (vins_c, (addrs_c, values_c)))) in tx_input_updates
        .txids
        .chunks(TX_INPUT_UPDATE_CHUNK_SIZE)
        .zip(
          tx_input_updates
            .vins
            .chunks(TX_INPUT_UPDATE_CHUNK_SIZE)
            .zip(
              tx_input_updates
                .addrs
                .chunks(TX_INPUT_UPDATE_CHUNK_SIZE)
                .zip(tx_input_updates.values.chunks(TX_INPUT_UPDATE_CHUNK_SIZE)),
            ),
        )
        .enumerate()
      {
        if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
          return Err(DaemonError::Index("forward run interrupted".to_owned()));
        }
        Self::activity(
          reporter,
          ApplyStagePlanSubstage::SpentUpdateInputs,
          Some((index + 1) as u32),
          Some(tx_input_chunks as u32),
        );
        let started = Instant::now();
        sql::batch_update_tx_inputs(tx, txids_c, vins_c, addrs_c, values_c).await?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        metrics.spent_update_inputs_ms += elapsed_ms;
        Self::log_substep_elapsed(
          self.history_height,
          ApplyStagePlanSubstage::SpentUpdateInputs,
          elapsed_ms,
          Some((index + 1) as u32),
          Some(tx_input_chunks as u32),
          txids_c.len(),
        );
        Self::progress(
          reporter,
          ApplyStagePlanSubstage::SpentUpdateInputs,
          Some((index + 1) as u32),
          Some(tx_input_chunks as u32),
        );
      }
      tokio::task::yield_now().await;
    }

    let fee_updates = build_fee_updates(&self.tx_value_out, &tx_input_totals);
    let address_history = build_address_history_rows(&addr_deltas);
    let address_stats = build_address_stats_rows(&addr_deltas);
    if let Some(db) = address_history_db {
      crate::store::address_history::insert_rows(
        db,
        &address_history.addrs,
        &address_history.txids,
        &address_history.block_heights,
        &address_history.block_positions,
        self.history_height,
        self.history_hash,
      )?;
    }
    let undo_addresses = address_stats
      .addrs
      .iter()
      .cloned()
      .collect::<BTreeSet<_>>()
      .into_iter()
      .collect::<Vec<_>>();
    if !undo_addresses.is_empty() {
      crate::sql::undo::capture_address_stats_undo(tx, self.history_height, &undo_addresses)
        .await?;
    }

    let fee_chunks = fee_updates.txids.len().div_ceil(FEE_UPDATE_CHUNK_SIZE);
    for (index, (txids_c, fees_c)) in fee_updates
      .txids
      .chunks(FEE_UPDATE_CHUNK_SIZE)
      .zip(fee_updates.fees.chunks(FEE_UPDATE_CHUNK_SIZE))
      .enumerate()
    {
      if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
        return Err(DaemonError::Index("forward run interrupted".to_owned()));
      }
      Self::activity(
        reporter,
        ApplyStagePlanSubstage::SpentUpdateFees,
        Some((index + 1) as u32),
        Some(fee_chunks as u32),
      );
      let started = Instant::now();
      sql::batch_update_fees(tx, self.history_height as i32, txids_c, fees_c).await?;
      let elapsed_ms = started.elapsed().as_millis() as u64;
      metrics.spent_update_fees_ms += elapsed_ms;
      Self::log_substep_elapsed(
        self.history_height,
        ApplyStagePlanSubstage::SpentUpdateFees,
        elapsed_ms,
        Some((index + 1) as u32),
        Some(fee_chunks as u32),
        txids_c.len(),
      );
      Self::progress(
        reporter,
        ApplyStagePlanSubstage::SpentUpdateFees,
        Some((index + 1) as u32),
        Some(fee_chunks as u32),
      );
    }

    if persist_pg_address_history && !address_history.addrs.is_empty() {
      let history_chunks = address_history.addrs.len().div_ceil(self.chunk_size);
      for (index, (addrs_c, (txids_c, (heights_c, positions_c)))) in address_history
        .addrs
        .chunks(self.chunk_size)
        .zip(
          address_history.txids.chunks(self.chunk_size).zip(
            address_history
              .block_heights
              .chunks(self.chunk_size)
              .zip(address_history.block_positions.chunks(self.chunk_size)),
          ),
        )
        .enumerate()
      {
        if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
          return Err(DaemonError::Index("forward run interrupted".to_owned()));
        }
        Self::activity(
          reporter,
          ApplyStagePlanSubstage::SpentAddressHistory,
          Some((index + 1) as u32),
          Some(history_chunks as u32),
        );
        let started = Instant::now();
        sql::insert_address_history(tx, addrs_c, txids_c, heights_c, positions_c).await?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        metrics.spent_address_history_ms += elapsed_ms;
        Self::log_substep_elapsed(
          self.history_height,
          ApplyStagePlanSubstage::SpentAddressHistory,
          elapsed_ms,
          Some((index + 1) as u32),
          Some(history_chunks as u32),
          addrs_c.len(),
        );
        Self::progress(
          reporter,
          ApplyStagePlanSubstage::SpentAddressHistory,
          Some((index + 1) as u32),
          Some(history_chunks as u32),
        );
      }
    }

    if !address_stats.addrs.is_empty() {
      let stat_chunks = address_stats.addrs.len().div_ceil(self.chunk_size);
      for (index, chunk) in chunk_address_stats(&address_stats, self.chunk_size)
        .into_iter()
        .enumerate()
      {
        if reporter.is_some_and(crate::index::runner::ForwardRunReporter::interrupt_requested) {
          return Err(DaemonError::Index("forward run interrupted".to_owned()));
        }
        Self::activity(
          reporter,
          ApplyStagePlanSubstage::SpentAddressStats,
          Some((index + 1) as u32),
          Some(stat_chunks as u32),
        );
        let started = Instant::now();
        sql::apply_address_stats_summary_chunk(tx, chunk.addrs, chunk.balances).await?;
        metrics.spent_address_balances_apply_ms += started.elapsed().as_millis() as u64;
        let started = Instant::now();
        sql::apply_address_stats_chunk(
          tx,
          chunk.addrs,
          chunk.balances,
          chunk.total_received,
          chunk.tx_counts,
          chunk.utxo_counts,
          chunk.first_seen_heights,
          chunk.last_seen_heights,
        )
        .await?;
        let elapsed_ms = started.elapsed().as_millis() as u64;
        metrics.spent_address_stats_ms += elapsed_ms;
        Self::log_substep_elapsed(
          self.history_height,
          ApplyStagePlanSubstage::SpentAddressStats,
          elapsed_ms,
          Some((index + 1) as u32),
          Some(stat_chunks as u32),
          chunk.addrs.len(),
        );
        Self::progress(
          reporter,
          ApplyStagePlanSubstage::SpentAddressStats,
          Some((index + 1) as u32),
          Some(stat_chunks as u32),
        );
      }
    }

    let total_ms = apply_started.elapsed().as_millis() as u64;
    if total_ms >= SPENT_SUBSTEP_SLOW_MS {
      tracing::info!(
        height = self.history_height,
        total_ms,
        spent_prevout_lookup_ms = metrics.spent_prevout_lookup_ms,
        spent_history_ms = metrics.spent_history_ms,
        spent_mark_outputs_ms = metrics.spent_mark_outputs_ms,
        spent_update_inputs_ms = metrics.spent_update_inputs_ms,
        spent_update_fees_ms = metrics.spent_update_fees_ms,
        spent_address_history_ms = metrics.spent_address_history_ms,
        spent_address_stats_ms = metrics.spent_address_stats_ms,
        spent_address_balances_prepare_ms = metrics.spent_address_balances_prepare_ms,
        spent_address_balances_stage_ms = metrics.spent_address_balances_stage_ms,
        spent_address_balances_apply_ms = metrics.spent_address_balances_apply_ms,
        touched_addresses = address_stats.addrs.len(),
        touched_history_rows = address_history.addrs.len(),
        "spent stage timing summary"
      );
    } else {
      tracing::debug!(
        height = self.history_height,
        total_ms,
        spent_prevout_lookup_ms = metrics.spent_prevout_lookup_ms,
        spent_history_ms = metrics.spent_history_ms,
        spent_mark_outputs_ms = metrics.spent_mark_outputs_ms,
        spent_update_inputs_ms = metrics.spent_update_inputs_ms,
        spent_update_fees_ms = metrics.spent_update_fees_ms,
        spent_address_history_ms = metrics.spent_address_history_ms,
        spent_address_stats_ms = metrics.spent_address_stats_ms,
        spent_address_balances_prepare_ms = metrics.spent_address_balances_prepare_ms,
        spent_address_balances_stage_ms = metrics.spent_address_balances_stage_ms,
        spent_address_balances_apply_ms = metrics.spent_address_balances_apply_ms,
        touched_addresses = address_stats.addrs.len(),
        touched_history_rows = address_history.addrs.len(),
        "spent stage timing summary"
      );
    }

    Ok(())
  }
}

type TxValueOutRow = ([u8; 32], i64);
type TxInputTotals = HashMap<[u8; 32], i64>;
type CollectedSpentInputs = (
  Vec<InputRef>,
  HashMap<String, AddrAccum>,
  Vec<TxValueOutRow>,
);

type PrevoutKey = (Vec<u8>, i32);

fn collect_spent_inputs(
  block: &BlockCtx,
  p2pkh_version: u8,
  p2sh_version: u8,
  history_height: u32,
  progress_step: usize,
  reporter: Option<&crate::index::runner::ForwardRunReporter>,
) -> Result<CollectedSpentInputs, DaemonError> {
  let mut input_refs = Vec::new();
  let mut addr_deltas: HashMap<String, AddrAccum> = HashMap::new();
  let mut tx_value_out = Vec::new();
  let block_height = block.height() as i32;
  let tx_total = block.block().transactions.len() as u32;
  let chunk_total = tx_total.div_ceil(progress_step as u32);

  for (chunk_number, tx_chunk) in block.block().transactions.chunks(progress_step).enumerate() {
    let chunk_index = (chunk_number + 1) as u32;
    SpentBatchPlan::activity(
      reporter,
      PlanConnectSubstage::SpentCollectInputs,
      Some(chunk_index),
      Some(chunk_total),
    );
    let started = Instant::now();
    for (offset, transaction) in tx_chunk.iter().enumerate() {
      let pos = chunk_number * progress_step + offset;
      let txid = *block.txid(pos).ok_or_else(|| {
        DaemonError::Index(format!("tx {pos} missing txid at {}", block.height()))
      })?;
      let block_pos = pos as i32;

      for output in &transaction.outputs {
        if let Some(addr) =
          dash_script::derive_address(output.script_pubkey.as_bytes(), p2pkh_version, p2sh_version)
        {
          let acc = addr_deltas.entry(addr).or_default();
          acc.received += output.value.to_sat() as i64;
          acc.utxo_count += 1;
          acc.txids.insert((txid, block_height, block_pos));
        }
      }

      for (vin, input) in transaction.inputs.iter().enumerate() {
        if input.prevout.hash.is_null() {
          continue;
        }
        input_refs.push(InputRef {
          txid,
          vin: vin as i32,
          prev_txid: input.prevout.hash.as_bytes().to_vec(),
          prev_vout: input.prevout.index as i32,
          height: block_height,
          block_pos,
        });
      }

      if pos > 0 {
        tx_value_out.push((
          txid,
          transaction.outputs.iter().map(|output| output.value.to_sat() as i64).sum(),
        ));
      }
    }
    SpentBatchPlan::log_substep_elapsed(
      history_height,
      PlanConnectSubstage::SpentCollectInputs,
      started.elapsed().as_millis() as u64,
      Some(chunk_index),
      Some(chunk_total),
      tx_chunk.len(),
    );
    SpentBatchPlan::progress(
      reporter,
      PlanConnectSubstage::SpentCollectInputs,
      Some(chunk_index),
      Some(chunk_total),
    );
  }

  Ok((input_refs, addr_deltas, tx_value_out))
}

fn sort_input_refs_by_prevout(input_refs: &mut [InputRef]) {
  input_refs.sort_unstable_by(|left, right| {
    left
      .prev_txid
      .cmp(&right.prev_txid)
      .then(left.prev_vout.cmp(&right.prev_vout))
      .then(left.height.cmp(&right.height))
      .then(left.block_pos.cmp(&right.block_pos))
      .then(left.vin.cmp(&right.vin))
  });
}

fn hydrate_spent_chunk(
  input_refs: &[InputRef],
  prev_lookup: Vec<crate::store::prevout::PrevoutRow>,
  addr_deltas: &mut HashMap<String, AddrAccum>,
  tx_input_totals: &mut TxInputTotals,
) -> (SpentSnapshotRows, TxInputUpdates) {
  let mut spent_snapshots = SpentSnapshotRows::default();
  let mut tx_input_updates = TxInputUpdates::default();
  let prev_by_key: HashMap<PrevoutKey, crate::store::prevout::PrevoutRow> = prev_lookup
    .into_iter()
    .map(|row| ((row.prev_txid.clone(), row.prev_vout), row))
    .collect();

  for input in input_refs {
    let key = (input.prev_txid.clone(), input.prev_vout);
    let Some(prev) = prev_by_key.get(&key) else {
      continue;
    };
    let spending_txid = copy_txid(&input.txid);
    spent_snapshots.prev_txids.push(prev.prev_txid.clone());
    spent_snapshots.prev_vouts.push(prev.prev_vout);
    spent_snapshots.prev_heights.push(prev.prev_height);
    spent_snapshots
      .prev_spent_txids
      .push(prev.prev_spent_txid.clone());
    spent_snapshots.prev_spent_vins.push(prev.prev_spent_vin);
    spent_snapshots
      .prev_spent_heights
      .push(prev.prev_spent_height);
    spent_snapshots.spending_txids.push(input.txid.to_vec());
    spent_snapshots.spending_vins.push(input.vin);
    spent_snapshots.heights.push(input.height);

    tx_input_updates.txids.push(input.txid.to_vec());
    tx_input_updates.vins.push(input.vin);
    tx_input_updates.addrs.push(prev.prev_addr.clone());
    tx_input_updates.values.push(prev.prev_value);

    *tx_input_totals.entry(spending_txid).or_insert(0) += prev.prev_value;

    if let Some(addr) = &prev.prev_addr {
      let acc = addr_deltas.entry(addr.clone()).or_default();
      acc.sent += prev.prev_value;
      acc.utxo_count -= 1;
      acc
        .txids
        .insert((spending_txid, input.height, input.block_pos));
    }
  }

  (spent_snapshots, tx_input_updates)
}

fn build_fee_updates(
  tx_value_out: &[TxValueOutRow],
  tx_input_totals: &TxInputTotals,
) -> FeeUpdates {
  let mut fee_updates = FeeUpdates::default();

  for (txid, value_out) in tx_value_out {
    let total_in = tx_input_totals.get(txid).copied().unwrap_or(0);
    let fee = total_in - value_out;
    if fee >= 0 {
      fee_updates.txids.push(txid.to_vec());
      fee_updates.fees.push(fee);
    }
  }

  fee_updates
}

/// Build one deduplicated address-history row set for block-scoped writes.
fn build_address_history_rows(addr_deltas: &HashMap<String, AddrAccum>) -> AddressHistoryRows {
  let mut rows = AddressHistoryRows::default();
  let mut ordered = Vec::new();

  for (addr, acc) in sorted_addr_deltas(addr_deltas) {
    for (txid, block_height, block_pos) in &acc.txids {
      ordered.push((addr.clone(), *block_height, txid.to_vec(), *block_pos));
    }
  }
  ordered.sort_unstable_by(|left, right| {
    left
      .0
      .cmp(&right.0)
      .then(left.1.cmp(&right.1))
      .then(left.2.cmp(&right.2))
  });

  for (addr, block_height, txid, block_pos) in ordered {
    rows.addrs.push(addr);
    rows.txids.push(txid);
    rows.block_heights.push(block_height);
    rows.block_positions.push(block_pos);
  }

  rows
}

/// Build one block-scoped address summary delta row set.
fn build_address_stats_rows(addr_deltas: &HashMap<String, AddrAccum>) -> AddressStatsRows {
  let mut rows = AddressStatsRows::default();

  for (addr, acc) in sorted_addr_deltas(addr_deltas) {
    let mut first_seen = None;
    let mut last_seen = None;
    for (_, block_height, _) in &acc.txids {
      first_seen =
        Some(first_seen.map_or(*block_height, |current: i32| current.min(*block_height)));
      last_seen = Some(last_seen.map_or(*block_height, |current: i32| current.max(*block_height)));
    }

    rows.addrs.push(addr.clone());
    rows.balances.push(acc.received - acc.sent);
    rows.total_received.push(acc.received);
    rows.tx_counts.push(acc.txids.len() as i32);
    rows.utxo_counts.push(acc.utxo_count);
    rows.first_seen_heights.push(first_seen);
    rows.last_seen_heights.push(last_seen);
  }

  rows
}

fn sorted_addr_deltas<'a>(
  addr_deltas: &'a HashMap<String, AddrAccum>,
) -> Vec<(&'a String, &'a AddrAccum)> {
  let mut entries = addr_deltas.iter().collect::<Vec<_>>();
  entries.sort_unstable_by(|(left, _), (right, _)| left.cmp(right));
  entries
}

struct AddressStatsChunk<'a> {
  addrs: &'a [String],
  balances: &'a [i64],
  total_received: &'a [i64],
  tx_counts: &'a [i32],
  utxo_counts: &'a [i32],
  first_seen_heights: &'a [Option<i32>],
  last_seen_heights: &'a [Option<i32>],
}

fn chunk_address_stats(
  rows: &AddressStatsRows,
  chunk_size: usize,
) -> impl Iterator<Item = AddressStatsChunk<'_>> {
  let total = rows.addrs.len();
  (0..total).step_by(chunk_size).map(move |start| {
    let end = (start + chunk_size).min(total);
    AddressStatsChunk {
      addrs: &rows.addrs[start..end],
      balances: &rows.balances[start..end],
      total_received: &rows.total_received[start..end],
      tx_counts: &rows.tx_counts[start..end],
      utxo_counts: &rows.utxo_counts[start..end],
      first_seen_heights: &rows.first_seen_heights[start..end],
      last_seen_heights: &rows.last_seen_heights[start..end],
    }
  })
}

fn copy_txid(txid: &[u8]) -> [u8; 32] {
  let mut out = [0u8; 32];
  out.copy_from_slice(txid);
  out
}

fn current_chunk_size(batch_sizer: &Mutex<BatchSizer>) -> usize {
  batch_sizer
    .lock()
    .map(|sizer| sizer.current.max(1) as usize)
    .unwrap_or(DEFAULT_SPENT_WRITE_CHUNK_SIZE)
}

fn spent_plan_progress_step(chunk_size: usize) -> usize {
  chunk_size.max(1).min(PREVOUT_APPLY_CHUNK_SIZE)
}

fn spent_lookup_window_size(chunk_size: usize) -> usize {
  // Hydration windows must stay small enough that pause/fence can preempt the
  // worker between completed windows instead of waiting behind one opaque read.
  spent_plan_progress_step(chunk_size).min(DEFAULT_MIN_BATCH_SIZE as usize)
}

#[cfg(test)]
mod tests {
  use super::{
    build_address_history_rows, build_address_stats_rows, hydrate_spent_chunk,
    spent_lookup_window_size, spent_plan_progress_step, AddrAccum, InputRef,
  };
  use std::collections::{HashMap, HashSet};

  #[test]
  fn address_history_rows_sort_by_conflict_key() {
    let mut left_txids = HashSet::new();
    left_txids.insert(([3u8; 32], 100, 1));
    left_txids.insert(([1u8; 32], 100, 0));
    let mut right_txids = HashSet::new();
    right_txids.insert(([2u8; 32], 99, 0));

    let mut addr_deltas = HashMap::new();
    addr_deltas.insert(
      "XtestB".to_owned(),
      AddrAccum {
        received: 70,
        sent: 20,
        utxo_count: 1,
        txids: right_txids,
      },
    );
    addr_deltas.insert(
      "XtestA".to_owned(),
      AddrAccum {
        received: 50,
        sent: 10,
        utxo_count: 1,
        txids: left_txids,
      },
    );

    let rows = build_address_history_rows(&addr_deltas);
    assert_eq!(rows.addrs, ["XtestA", "XtestA", "XtestB"]);
    assert_eq!(rows.block_heights, [100, 100, 99]);
    assert_eq!(rows.txids, [vec![1u8; 32], vec![3u8; 32], vec![2u8; 32]]);
    assert_eq!(rows.block_positions, [0, 1, 0]);
  }

  #[test]
  fn address_history_rows_dedup_one_tx_touch_per_address() {
    let mut txids = HashSet::new();
    txids.insert(([1u8; 32], 100, 2));
    txids.insert(([1u8; 32], 100, 2));
    txids.insert(([2u8; 32], 105, 1));

    let mut addr_deltas = HashMap::new();
    addr_deltas.insert(
      "Xtest".to_owned(),
      AddrAccum {
        received: 70,
        sent: 20,
        utxo_count: 1,
        txids,
      },
    );

    let rows = build_address_history_rows(&addr_deltas);

    assert_eq!(rows.addrs.len(), 2);
    assert_eq!(rows.txids.len(), 2);
    assert_eq!(rows.block_heights.len(), 2);
    assert_eq!(rows.block_positions.len(), 2);
  }

  #[test]
  fn address_stats_rows_capture_received_count_seen_heights_and_sort_by_address() {
    let mut left_txids = HashSet::new();
    left_txids.insert(([3u8; 32], 90, 0));
    let mut right_txids = HashSet::new();
    right_txids.insert(([1u8; 32], 100, 2));
    right_txids.insert(([2u8; 32], 105, 1));

    let mut addr_deltas = HashMap::new();
    addr_deltas.insert(
      "XtestB".to_owned(),
      AddrAccum {
        received: 70,
        sent: 20,
        utxo_count: 2,
        txids: right_txids,
      },
    );
    addr_deltas.insert(
      "XtestA".to_owned(),
      AddrAccum {
        received: 10,
        sent: 1,
        utxo_count: 1,
        txids: left_txids,
      },
    );

    let rows = build_address_stats_rows(&addr_deltas);

    assert_eq!(rows.addrs, vec!["XtestA".to_owned(), "XtestB".to_owned()]);
    assert_eq!(rows.balances, vec![9, 50]);
    assert_eq!(rows.total_received, vec![10, 70]);
    assert_eq!(rows.tx_counts, vec![1, 2]);
    assert_eq!(rows.utxo_counts, vec![1, 2]);
    assert_eq!(rows.first_seen_heights, vec![Some(90), Some(100)]);
    assert_eq!(rows.last_seen_heights, vec![Some(90), Some(105)]);
  }

  #[test]
  fn spent_plan_progress_step_caps_large_batches() {
    assert_eq!(spent_plan_progress_step(0), 1);
    assert_eq!(spent_plan_progress_step(32), 32);
    assert_eq!(spent_plan_progress_step(512), 512);
    assert_eq!(spent_plan_progress_step(50_000), 512);
  }

  #[test]
  fn spent_lookup_window_size_uses_existing_min_batch_floor() {
    assert_eq!(spent_lookup_window_size(0), 1);
    assert_eq!(spent_lookup_window_size(32), 32);
    assert_eq!(spent_lookup_window_size(100), 100);
    assert_eq!(spent_lookup_window_size(512), 100);
    assert_eq!(spent_lookup_window_size(50_000), 100);
  }

  #[test]
  fn spent_prevout_lookup_hydrates_one_chunk() -> Result<(), Box<dyn std::error::Error>> {
    let mut addr_deltas = HashMap::new();
    let mut tx_input_totals = HashMap::new();
    let input_refs = vec![
      InputRef {
        txid: [0xAB; 32],
        vin: 0,
        prev_txid: vec![1; 32],
        prev_vout: 0,
        height: 2,
        block_pos: 0,
      },
      InputRef {
        txid: [0xAB; 32],
        vin: 1,
        prev_txid: vec![2; 32],
        prev_vout: 1,
        height: 2,
        block_pos: 0,
      },
    ];
    let prev_lookup = vec![
      crate::store::prevout::PrevoutRow {
        prev_txid: vec![1; 32],
        prev_vout: 0,
        prev_height: 1,
        prev_addr: Some("Xa".to_owned()),
        prev_value: 10,
        prev_spent_txid: None,
        prev_spent_vin: None,
        prev_spent_height: None,
      },
      crate::store::prevout::PrevoutRow {
        prev_txid: vec![2; 32],
        prev_vout: 1,
        prev_height: 1,
        prev_addr: Some("Xb".to_owned()),
        prev_value: 20,
        prev_spent_txid: None,
        prev_spent_vin: None,
        prev_spent_height: None,
      },
    ];

    let (spent_snapshots, tx_input_updates) = hydrate_spent_chunk(
      &input_refs,
      prev_lookup,
      &mut addr_deltas,
      &mut tx_input_totals,
    );

    assert_eq!(spent_snapshots.prev_txids.len(), 2);
    assert_eq!(tx_input_updates.txids.len(), 2);
    assert_eq!(tx_input_totals.get(&[0xAB; 32]), Some(&30));
    assert_eq!(addr_deltas["Xa"].sent, 10);
    assert_eq!(addr_deltas["Xb"].sent, 20);
    Ok(())
  }
}
