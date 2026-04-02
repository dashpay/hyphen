//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Benchmarks and seed helpers for live SQL state.

use divan::black_box;

use hyphend::bench::{self, WorkloadGroup};

const PAGE_LIMIT: i64 = 100;

#[divan::bench(name = "mempool_summary_read", ignore = !bench::should_run(WorkloadGroup::Live))]
fn mempool_summary_read() {
  let row = bench::runtime()
    .run_async(hyphend::sql::live::mempool::summary(
      bench::runtime().pool(),
    ))
    .unwrap_or_else(|error| panic!("read mempool summary: {error}"));
  black_box(row);
}

#[divan::bench(name = "mempool_recent_later_page", ignore = !bench::should_run(WorkloadGroup::Live))]
fn mempool_recent_later_page() {
  let rows = bench::runtime()
    .run_async(hyphend::sql::live::mempool::recent_txs(
      bench::runtime().pool(),
      PAGE_LIMIT,
    ))
    .unwrap_or_else(|error| panic!("read mempool recent txs: {error}"));
  black_box(rows.len());
}

#[divan::bench(
  name = "mempool_detail_inputs_outputs",
  ignore = !bench::should_run(WorkloadGroup::Live)
)]
fn mempool_detail_inputs_outputs() {
  let txid = hyphend::sql::bench::synthetic_hash(b'L', 1);
  let detail = bench::runtime()
    .run_async(hyphend::sql::live::mempool::tx_detail(
      bench::runtime().pool(),
      txid.as_slice(),
    ))
    .unwrap_or_else(|error| panic!("read mempool tx detail: {error}"));
  let inputs = bench::runtime()
    .run_async(hyphend::sql::live::mempool::tx_inputs(
      bench::runtime().pool(),
      txid.as_slice(),
    ))
    .unwrap_or_else(|error| panic!("read mempool tx inputs: {error}"));
  let outputs = bench::runtime()
    .run_async(hyphend::sql::live::mempool::tx_outputs(
      bench::runtime().pool(),
      txid.as_slice(),
    ))
    .unwrap_or_else(|error| panic!("read mempool tx outputs: {error}"));
  black_box((detail, inputs.len(), outputs.len()));
}

#[divan::bench(name = "mempool_address_page", ignore = !bench::should_run(WorkloadGroup::Live))]
fn mempool_address_page() {
  let rows = bench::runtime()
    .run_async(hyphend::sql::live::mempool::address_txs_page(
      bench::runtime().pool(),
      "XliveAddr00000",
      PAGE_LIMIT,
      0,
    ))
    .unwrap_or_else(|error| panic!("read mempool address page: {error}"));
  black_box(rows.len());
}

#[divan::bench(name = "governance_object_list", ignore = !bench::should_run(WorkloadGroup::Live))]
fn governance_object_list() {
  let rows = bench::runtime()
    .run_async(hyphend::sql::live::gov::objects(
      bench::runtime().pool(),
      PAGE_LIMIT,
      None,
    ))
    .unwrap_or_else(|error| panic!("read governance objects: {error}"));
  black_box(rows.len());
}

#[divan::bench(name = "governance_votes_later_page", ignore = !bench::should_run(WorkloadGroup::Live))]
fn governance_votes_later_page() {
  let object_hash = hyphend::sql::bench::synthetic_hash(b'G', 0);
  let rows = bench::runtime()
    .run_async(hyphend::sql::live::gov::votes(
      bench::runtime().pool(),
      object_hash.as_slice(),
      PAGE_LIMIT,
      None,
    ))
    .unwrap_or_else(|error| panic!("read governance votes: {error}"));
  black_box(rows.len());
}

#[divan::bench(name = "network_snapshot_read", ignore = !bench::should_run(WorkloadGroup::Live))]
fn network_snapshot_read() {
  let row = bench::runtime()
    .run_async(hyphend::sql::live::network::snapshot(
      bench::runtime().pool(),
    ))
    .unwrap_or_else(|error| panic!("read network snapshot: {error}"));
  black_box(row);
}

#[divan::bench(name = "chainlock_snapshot_read", ignore = !bench::should_run(WorkloadGroup::Live))]
fn chainlock_snapshot_read() {
  let row = bench::runtime()
    .run_async(hyphend::sql::live::chainlock::snapshot(
      bench::runtime().pool(),
    ))
    .unwrap_or_else(|error| panic!("read chainlock snapshot: {error}"));
  black_box(row);
}

#[divan::bench(name = "islock_lookup", ignore = !bench::should_run(WorkloadGroup::Live))]
fn islock_lookup() {
  let txid = hyphend::sql::bench::synthetic_hash(b'I', 1);
  let row = bench::runtime()
    .run_async(hyphend::sql::live::islock::current_by_txid(
      bench::runtime().pool(),
      txid.as_slice(),
    ))
    .unwrap_or_else(|error| panic!("read islock row: {error}"));
  black_box(row);
}

#[divan::bench(
  name = "validator_protx_snapshot_scan",
  ignore = !bench::should_run(WorkloadGroup::Live)
)]
fn validator_protx_snapshot_scan() {
  let rows = bench::runtime()
    .run_async(hyphend::sql::live::protx::snapshot(bench::runtime().pool()))
    .unwrap_or_else(|error| panic!("read protx snapshot rows: {error}"));
  black_box(rows.len());
}
