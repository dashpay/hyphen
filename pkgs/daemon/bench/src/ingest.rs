//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Timed ingest benchmarks over the real daemon ingest seams.

use divan::black_box;

use hyphend::bench::{self, WorkloadGroup};

#[divan::bench(name = "ingest_admit_batch", ignore = !bench::should_run(WorkloadGroup::Ingest))]
fn ingest_admit_batch() {
  let admitted = hyphend::bench::ingest_admit_batch_bench()
    .unwrap_or_else(|error| panic!("benchmark ingest admit batch: {error}"));
  black_box(admitted);
}

#[divan::bench(name = "ingest_fetch_batch", ignore = !bench::should_run(WorkloadGroup::Ingest))]
fn ingest_fetch_batch() {
  let fetched = hyphend::bench::ingest_fetch_batch_bench()
    .unwrap_or_else(|error| panic!("benchmark ingest fetch batch: {error}"));
  black_box(fetched);
}

#[divan::bench(name = "ingest_commit_batch", ignore = !bench::should_run(WorkloadGroup::Ingest))]
fn ingest_commit_batch() {
  let committed = hyphend::bench::ingest_commit_batch_bench()
    .unwrap_or_else(|error| panic!("benchmark ingest commit batch: {error}"));
  black_box(committed);
}

#[divan::bench(
  name = "ingest_turn_admit_fetch_commit",
  ignore = !bench::should_run(WorkloadGroup::Ingest)
)]
fn ingest_turn_admit_fetch_commit() {
  let committed = hyphend::bench::ingest_turn_admit_fetch_commit_bench()
    .unwrap_or_else(|error| panic!("benchmark ingest admit/fetch/commit turn: {error}"));
  black_box(committed);
}
