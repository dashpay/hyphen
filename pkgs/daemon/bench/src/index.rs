//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Timed benchmarks for indexed block, transaction, validator, and metrics SQL.

use divan::black_box;

use hyphend::bench::{self, BenchDataset, WorkloadGroup};

const PAGE_LIMIT: i64 = 100;
const CHART_TARGET_POINTS: i32 = 144;

#[divan::bench(name = "blocks_list_later_page", ignore = !bench::should_run(WorkloadGroup::Index))]
fn blocks_list_later_page() {
  let before_height = block_rows(bench::runtime().dataset()) - 5_000;
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::block::list_blocks(
      bench::runtime().pool(),
      PAGE_LIMIT,
      Some(before_height),
    ))
    .unwrap_or_else(|error| panic!("list indexed blocks: {error}"));
  black_box(rows.len());
}

#[divan::bench(name = "block_detail_by_hash", ignore = !bench::should_run(WorkloadGroup::Index))]
fn block_detail_by_hash() {
  let hash = hyphend::sql::bench::synthetic_hash(
    b'B',
    i64::from(block_rows(bench::runtime().dataset()) - 1),
  );
  let row = bench::runtime()
    .run_async(hyphend::sql::index::block::block_by_hash(
      bench::runtime().pool(),
      hash.as_slice(),
    ))
    .unwrap_or_else(|error| panic!("read indexed block detail: {error}"));
  black_box(row);
}

#[divan::bench(
  name = "transactions_list_later_page",
  ignore = !bench::should_run(WorkloadGroup::Index)
)]
fn transactions_list_later_page() {
  let before_height = block_rows(bench::runtime().dataset()) - 10_000;
  let before_txid = hyphend::sql::bench::synthetic_hash(b'T', i64::from(before_height));
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::tx::list_transactions(
      bench::runtime().pool(),
      PAGE_LIMIT,
      Some((before_height, 0, before_txid.as_slice())),
    ))
    .unwrap_or_else(|error| panic!("list indexed transactions: {error}"));
  black_box(rows.len());
}

#[divan::bench(
  name = "transaction_inputs_prevout_fallback",
  ignore = !bench::should_run(WorkloadGroup::Index)
)]
fn transaction_inputs_prevout_fallback() {
  let txid = hyphend::sql::bench::synthetic_hash(
    b'T',
    i64::from(block_rows(bench::runtime().dataset()) - 1),
  );
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::tx::transaction_inputs(
      bench::runtime().pool(),
      txid.as_slice(),
      true,
    ))
    .unwrap_or_else(|error| panic!("read indexed transaction inputs: {error}"));
  black_box(rows.len());
}

#[divan::bench(
  name = "validators_filtered_later_page",
  ignore = !bench::should_run(WorkloadGroup::Index)
)]
fn validators_filtered_later_page() {
  let before_height = validator_rows(bench::runtime().dataset()) - 1_000;
  let before_hash = hyphend::sql::bench::synthetic_hash(b'V', i64::from(before_height));
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::validator::list_validators(
      bench::runtime().pool(),
      Some("registered"),
      Some("Regular"),
      PAGE_LIMIT,
      Some((before_height, before_hash.as_slice())),
    ))
    .unwrap_or_else(|error| panic!("list indexed validators: {error}"));
  black_box(rows.len());
}

#[divan::bench(
  name = "validator_history_detail",
  ignore = !bench::should_run(WorkloadGroup::Index)
)]
fn validator_history_detail() {
  let hash = hyphend::sql::bench::synthetic_hash(
    b'V',
    i64::from(validator_rows(bench::runtime().dataset()) - 100),
  );
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::validator::validator_history(
      bench::runtime().pool(),
      hash.as_slice(),
    ))
    .unwrap_or_else(|error| panic!("read validator history: {error}"));
  black_box(rows.len());
}

#[divan::bench(name = "metrics_range_midspan", ignore = !bench::should_run(WorkloadGroup::Index))]
fn metrics_range_midspan() {
  let total_rows = block_rows(bench::runtime().dataset());
  let start = total_rows / 4;
  let end = total_rows - start;
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::metrics::metrics_range(
      bench::runtime().pool(),
      start,
      end,
    ))
    .unwrap_or_else(|error| panic!("read metrics range: {error}"));
  black_box(rows.len());
}

#[divan::bench(
  name = "metrics_chart_bucketed_midspan",
  ignore = !bench::should_run(WorkloadGroup::Index)
)]
fn metrics_chart_bucketed_midspan() {
  let total_rows = block_rows(bench::runtime().dataset());
  let start = total_rows / 4;
  let end = total_rows - start;
  let blocks = end - start + 1;
  let bucket_blocks = ((blocks + CHART_TARGET_POINTS - 1) / CHART_TARGET_POINTS).max(1);
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::metrics::bench_metrics_bucketed_range(
      bench::runtime().pool(),
      start,
      end,
      bucket_blocks,
    ))
    .unwrap_or_else(|error| panic!("read metrics chart buckets: {error}"));
  black_box(rows.len());
}

fn block_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 12_000,
    BenchDataset::Default => 60_000,
    BenchDataset::Large => 120_000,
  }
}

fn validator_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 3_000,
    BenchDataset::Default => 12_000,
    BenchDataset::Large => 24_000,
  }
}
