//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Timed benchmarks for address-domain indexed SQL reads.

use divan::black_box;

use hyphend::bench::{self, BenchDataset, WorkloadGroup};

const WHALE_ADDRESS: &str = "XbenchWhale1111111111111111111111111";
const PAGE_LIMIT: i64 = 100;

#[divan::bench(name = "address_summary_whale", ignore = !bench::should_run(WorkloadGroup::Spent))]
fn address_summary_whale() {
  let row = bench::runtime()
    .run_async(hyphend::sql::index::spent::address_summary(
      bench::runtime().pool(),
      WHALE_ADDRESS,
    ))
    .unwrap_or_else(|error| panic!("read whale address summary: {error}"));
  black_box(row);
}

#[divan::bench(
  name = "address_history_first_page_whale",
  ignore = !bench::should_run(WorkloadGroup::Spent)
)]
fn address_history_first_page_whale() {
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::spent::address_history_txs(
      bench::runtime().pool(),
      WHALE_ADDRESS,
      PAGE_LIMIT,
      None,
    ))
    .unwrap_or_else(|error| panic!("read whale address history first page: {error}"));
  black_box(rows.len());
}

#[divan::bench(
  name = "address_history_later_page_whale",
  ignore = !bench::should_run(WorkloadGroup::Spent)
)]
fn address_history_later_page_whale() {
  let cursor_height = history_rows(bench::runtime().dataset()) - 1_000;
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::spent::address_history_txs(
      bench::runtime().pool(),
      WHALE_ADDRESS,
      PAGE_LIMIT,
      Some((
        cursor_height,
        0,
        hyphend::sql::bench::synthetic_hash(b'T', i64::from(cursor_height)).as_slice(),
      )),
    ))
    .unwrap_or_else(|error| panic!("read whale address history later page: {error}"));
  black_box(rows.len());
}

#[divan::bench(name = "balances_first_page", ignore = !bench::should_run(WorkloadGroup::Spent))]
fn balances_first_page() {
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::spent::balances_page(
      bench::runtime().pool(),
      PAGE_LIMIT,
      None,
    ))
    .unwrap_or_else(|error| panic!("read balances first page: {error}"));
  black_box(rows.len());
}

#[divan::bench(
  name = "balances_summary_snapshot",
  ignore = !bench::should_run(WorkloadGroup::Spent)
)]
fn balances_summary_snapshot() {
  let summary = bench::runtime()
    .run_async(hyphend::sql::index::spent::balances_summary(
      bench::runtime().pool(),
    ))
    .unwrap_or_else(|error| panic!("read balances summary: {error}"));
  black_box(summary.included_supply);
}

#[divan::bench(name = "balances_later_page", ignore = !bench::should_run(WorkloadGroup::Spent))]
fn balances_later_page() {
  let index = 10_000.min(balance_rows(bench::runtime().dataset()) - 1);
  let balance = i64::from(balance_rows(bench::runtime().dataset()) - index) * 1_000;
  let address = format!("Xbench{index:05}");
  let rows = bench::runtime()
    .run_async(hyphend::sql::index::spent::balances_page(
      bench::runtime().pool(),
      PAGE_LIMIT,
      Some((balance, address.as_str())),
    ))
    .unwrap_or_else(|error| panic!("read balances later page: {error}"));
  black_box(rows.len());
}

fn history_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 4_000,
    BenchDataset::Default => 20_000,
    BenchDataset::Large => 60_000,
  }
}

fn balance_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 10_000,
    BenchDataset::Default => 50_000,
    BenchDataset::Large => 100_000,
  }
}
