//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL-owned benchmark validation metadata and read-only live analysis helpers.

#![allow(dead_code)]

use sqlx::PgPool;

use crate::error::DaemonError;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ValidationMethod {
  ExactExplainAnalyze,
  PlannerOnlyExplain,
  DaemonSlowLog,
  BenchmarkOnly,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Representativeness {
  Representative,
  LiveShapeMismatch,
  ProdScaleVariant,
  UnsafeToReplayLive,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidationEntry {
  pub bench: &'static str,
  pub owners: &'static [&'static str],
  pub method: ValidationMethod,
  pub representativeness: Representativeness,
  pub note: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptimizationBaseline {
  pub bench: &'static str,
  pub problem: &'static str,
  pub acceptance: &'static str,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OptimizationGuard {
  pub bench: &'static str,
  pub guard: &'static str,
}

pub const VALIDATION_MATRIX: &[ValidationEntry] = &[
  ValidationEntry {
    bench: "ingest_admit_batch",
    owners: &["ingest::admit::admit_batch"],
    method: ValidationMethod::BenchmarkOnly,
    representativeness: Representativeness::Representative,
    note: "Real ingest admission decision over the benchmark context.",
  },
  ValidationEntry {
    bench: "ingest_fetch_batch",
    owners: &["ingest::fetch::fetch_batch"],
    method: ValidationMethod::BenchmarkOnly,
    representativeness: Representativeness::Representative,
    note: "Real ingest fetch step over the benchmark context.",
  },
  ValidationEntry {
    bench: "ingest_commit_batch",
    owners: &["ingest::commit::commit_batch"],
    method: ValidationMethod::BenchmarkOnly,
    representativeness: Representativeness::Representative,
    note: "Real ingest RocksDB commit step over the benchmark context.",
  },
  ValidationEntry {
    bench: "ingest_turn_admit_fetch_commit",
    owners: &[
      "ingest::admit::admit_batch",
      "ingest::fetch::fetch_batch",
      "ingest::commit::commit_batch",
    ],
    method: ValidationMethod::BenchmarkOnly,
    representativeness: Representativeness::Representative,
    note: "Real ingest turn covering admission, fetch, and commit in sequence.",
  },
  ValidationEntry {
    bench: "blocks_list_later_page",
    owners: &["index::block::list_blocks"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "List-page index scan route.",
  },
  ValidationEntry {
    bench: "block_detail_by_hash",
    owners: &["index::block::block_by_hash"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Point block lookup by hash.",
  },
  ValidationEntry {
    bench: "transactions_list_later_page",
    owners: &["index::tx::list_transactions"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Cursor transaction list route.",
  },
  ValidationEntry {
    bench: "transaction_inputs_prevout_fallback",
    owners: &["index::tx::transaction_inputs"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Input read with prevout fallback join.",
  },
  ValidationEntry {
    bench: "validators_filtered_later_page",
    owners: &["index::validator::list_validators"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::LiveShapeMismatch,
    note: "Live validator status/type distribution may differ from synthetic bench filters.",
  },
  ValidationEntry {
    bench: "validator_history_detail",
    owners: &["index::validator::validator_history"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Validator history detail read.",
  },
  ValidationEntry {
    bench: "metrics_range_midspan",
    owners: &["index::metrics::metrics_range"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::ProdScaleVariant,
    note: "Benchmark span is smaller than observed production scans.",
  },
  ValidationEntry {
    bench: "metrics_chart_bucketed_midspan",
    owners: &["index::metrics::metrics_bucketed_range"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Production chart path reads one bucket-end metrics row per output point.",
  },
  ValidationEntry {
    bench: "chainlock_snapshot_read",
    owners: &["live::chainlock::snapshot"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Singleton live snapshot read.",
  },
  ValidationEntry {
    bench: "governance_object_list",
    owners: &["live::gov::objects"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Governance object page read.",
  },
  ValidationEntry {
    bench: "governance_votes_later_page",
    owners: &["live::gov::votes"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Governance votes page read.",
  },
  ValidationEntry {
    bench: "islock_lookup",
    owners: &["live::islock::current_by_txid"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Point live islock lookup.",
  },
  ValidationEntry {
    bench: "mempool_address_page",
    owners: &["live::mempool::address_txs_page"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::LiveShapeMismatch,
    note: "Current live mempool size may be materially smaller than the synthetic corpus.",
  },
  ValidationEntry {
    bench: "mempool_detail_inputs_outputs",
    owners: &[
      "live::mempool::tx_detail",
      "live::mempool::tx_inputs",
      "live::mempool::tx_outputs",
    ],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::LiveShapeMismatch,
    note: "Detail route is sensitive to current mempool churn and sample tx choice.",
  },
  ValidationEntry {
    bench: "mempool_recent_later_page",
    owners: &["live::mempool::recent_txs"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Cursor live mempool page.",
  },
  ValidationEntry {
    bench: "mempool_summary_read",
    owners: &["live::mempool::summary"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Singleton mempool summary read.",
  },
  ValidationEntry {
    bench: "network_snapshot_read",
    owners: &["live::network::snapshot"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Singleton network snapshot read.",
  },
  ValidationEntry {
    bench: "validator_protx_snapshot_scan",
    owners: &["live::protx::snapshot"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Validator protx snapshot scan.",
  },
  ValidationEntry {
    bench: "address_history_first_page_whale",
    owners: &["index::spent::address_history_txs"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Whale address first page read.",
  },
  ValidationEntry {
    bench: "address_history_later_page_whale",
    owners: &["index::spent::address_history_txs"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Whale address later page read.",
  },
  ValidationEntry {
    bench: "address_summary_whale",
    owners: &["index::spent::address_summary"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Point address summary read.",
  },
  ValidationEntry {
    bench: "balances_first_page",
    owners: &["index::spent::balances_page"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "First balances page ordered by balance rank.",
  },
  ValidationEntry {
    bench: "balances_summary_snapshot",
    owners: &["index::spent::balances_summary"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Balances included-supply summary snapshot aggregate.",
  },
  ValidationEntry {
    bench: "balances_later_page",
    owners: &["index::spent::balances_page"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Later balances page ordered by balance rank.",
  },
  ValidationEntry {
    bench: "address_balances_apply_chunk",
    owners: &["index::spent::apply_staged_address_balances_chunk"],
    method: ValidationMethod::DaemonSlowLog,
    representativeness: Representativeness::UnsafeToReplayLive,
    note: "Live validation uses daemon slow-query correlation and planner stats.",
  },
  ValidationEntry {
    bench: "address_balances_stage_chunk",
    owners: &["index::spent::stage_address_balances_delta"],
    method: ValidationMethod::PlannerOnlyExplain,
    representativeness: Representativeness::Representative,
    note: "Temp-table staging path is validated by local planner shape only.",
  },
  ValidationEntry {
    bench: "address_history_batch_insert",
    owners: &["index::spent::insert_address_history"],
    method: ValidationMethod::DaemonSlowLog,
    representativeness: Representativeness::UnsafeToReplayLive,
    note: "FK-backed write route validated through live slow logs.",
  },
  ValidationEntry {
    bench: "address_stats_delta_apply",
    owners: &["index::spent::apply_address_stats_chunk"],
    method: ValidationMethod::DaemonSlowLog,
    representativeness: Representativeness::UnsafeToReplayLive,
    note: "Large address summary upsert validated through live slow logs.",
  },
  ValidationEntry {
    bench: "block_archive_delete_rollback",
    owners: &["index::block::delete_block"],
    method: ValidationMethod::BenchmarkOnly,
    representativeness: Representativeness::UnsafeToReplayLive,
    note: "Destructive rollback path is benchmark-only by policy.",
  },
  ValidationEntry {
    bench: "blocks_upsert_one",
    owners: &["index::block::insert_block"],
    method: ValidationMethod::PlannerOnlyExplain,
    representativeness: Representativeness::Representative,
    note: "Write route is not a current live hotspot.",
  },
  ValidationEntry {
    bench: "mempool_upsert_one",
    owners: &[
      "live::mempool::upsert_summary",
      "live::mempool::upsert_recent_tx",
      "live::mempool::upsert_tx_detail",
    ],
    method: ValidationMethod::PlannerOnlyExplain,
    representativeness: Representativeness::Representative,
    note: "Live mempool write path is not a current hotspot.",
  },
  ValidationEntry {
    bench: "partition_ensure_existing",
    owners: &["segment::ensure_core_history_partitions"],
    method: ValidationMethod::BenchmarkOnly,
    representativeness: Representativeness::UnsafeToReplayLive,
    note: "DDL-style route is benchmark-only by policy.",
  },
  ValidationEntry {
    bench: "sync_state_update_all",
    owners: &["sync::update_indexed_stage_height"],
    method: ValidationMethod::PlannerOnlyExplain,
    representativeness: Representativeness::Representative,
    note: "Small sync-state maintenance write.",
  },
  ValidationEntry {
    bench: "table_size_diag_snapshot",
    owners: &["snapshot::table_size_diag"],
    method: ValidationMethod::ExactExplainAnalyze,
    representativeness: Representativeness::Representative,
    note: "Admin diagnostic query; expensive by nature.",
  },
  ValidationEntry {
    bench: "transactions_batch_insert",
    owners: &["index::tx::insert_transactions"],
    method: ValidationMethod::DaemonSlowLog,
    representativeness: Representativeness::UnsafeToReplayLive,
    note: "Live validation uses daemon slow-query correlation and planner stats.",
  },
  ValidationEntry {
    bench: "tx_inputs_batch_insert",
    owners: &["index::tx::insert_tx_inputs"],
    method: ValidationMethod::DaemonSlowLog,
    representativeness: Representativeness::UnsafeToReplayLive,
    note: "FK-backed tx input write route validated through live slow logs.",
  },
  ValidationEntry {
    bench: "tx_outputs_apply_chunk",
    owners: &["index::tx::apply_staged_tx_outputs_chunk"],
    method: ValidationMethod::DaemonSlowLog,
    representativeness: Representativeness::UnsafeToReplayLive,
    note: "Final tx output apply path validated through live slow logs.",
  },
  ValidationEntry {
    bench: "tx_outputs_stage_chunk",
    owners: &["index::tx::stage_tx_outputs"],
    method: ValidationMethod::PlannerOnlyExplain,
    representativeness: Representativeness::Representative,
    note: "Temp-table staging path is planner-validated only.",
  },
];

pub const OPTIMIZATION_BASELINES: &[OptimizationBaseline] = &[
  OptimizationBaseline {
    bench: "balances_first_page",
    problem: "ordered balances pagination still shows double-digit live latency",
    acceptance: "exact explain and bench median improve without changing page semantics",
  },
  OptimizationBaseline {
    bench: "balances_later_page",
    problem: "later-page balances cursor path is slower than first-page path",
    acceptance: "exact explain and bench median improve without changing cursor semantics",
  },
  OptimizationBaseline {
    bench: "address_history_first_page_whale",
    problem: "whale address history pays repeated transaction row lookups",
    acceptance: "live explain and bench median improve without changing row shape",
  },
  OptimizationBaseline {
    bench: "address_history_later_page_whale",
    problem: "later whale address history page has the same hydration cost",
    acceptance: "live explain and bench median improve without changing cursor semantics",
  },
  OptimizationBaseline {
    bench: "address_history_batch_insert",
    problem: "address_txs write path is a live slow-query hotspot",
    acceptance: "bench median and live slow-log timings improve",
  },
  OptimizationBaseline {
    bench: "tx_inputs_batch_insert",
    problem: "tx_inputs batch insert is a live slow-query hotspot",
    acceptance: "bench median and live slow-log timings improve",
  },
  OptimizationBaseline {
    bench: "tx_outputs_apply_chunk",
    problem: "tx_outputs final apply is the heaviest write-route benchmark",
    acceptance: "bench median and live slow-log timings improve",
  },
  OptimizationBaseline {
    bench: "address_stats_delta_apply",
    problem: "address_stats upsert remains a live slow-query hotspot",
    acceptance: "bench median and live slow-log timings improve",
  },
  OptimizationBaseline {
    bench: "address_balances_apply_chunk",
    problem: "address_balances staged apply remains a live slow-query hotspot",
    acceptance: "bench median and live slow-log timings improve",
  },
  OptimizationBaseline {
    bench: "table_size_diag_snapshot",
    problem: "admin table-size snapshot is much slower live than in the bench corpus",
    acceptance: "diagnostic cost is reduced or explicitly bounded",
  },
  OptimizationBaseline {
    bench: "metrics_range_midspan",
    problem: "benchmark span is smaller than the production-shaped metrics scan",
    acceptance: "benchmark posture is updated to reflect prod-shaped scans",
  },
  OptimizationBaseline {
    bench: "validators_filtered_later_page",
    problem: "benchmark filter posture can drift away from current live validator mix",
    acceptance: "benchmark posture matches a live-representative status/type pair",
  },
];

pub const OPTIMIZATION_GUARDS: &[OptimizationGuard] = &[
  OptimizationGuard {
    bench: "balances_first_page",
    guard: "spent::tests::balances_query_keeps_tuple_cursor_predicate",
  },
  OptimizationGuard {
    bench: "balances_later_page",
    guard: "spent::tests::balances_query_keeps_tuple_cursor_predicate",
  },
  OptimizationGuard {
    bench: "address_history_first_page_whale",
    guard: "spent::tests::address_history_query_keeps_materialized_page_boundary",
  },
  OptimizationGuard {
    bench: "address_history_later_page_whale",
    guard: "spent::tests::address_history_query_keeps_materialized_page_boundary",
  },
  OptimizationGuard {
    bench: "address_history_batch_insert",
    guard: "spent::tests::address_history_insert_orders_conflict_keys",
  },
  OptimizationGuard {
    bench: "tx_inputs_batch_insert",
    guard: "tx::tests::tx_inputs_insert_orders_conflict_keys",
  },
  OptimizationGuard {
    bench: "tx_outputs_apply_chunk",
    guard: "tx::tests::tx_outputs_apply_stays_unsorted",
  },
  OptimizationGuard {
    bench: "address_stats_delta_apply",
    guard: "spent::tests::address_stats_apply_keeps_duplicate_aggregation",
  },
  OptimizationGuard {
    bench: "address_balances_apply_chunk",
    guard:
      "spent::tests::apply_staged_address_balances_chunk_upserts_mixed_rows_when_pg_is_configured",
  },
  OptimizationGuard {
    bench: "table_size_diag_snapshot",
    guard: "snapshot::tests::table_size_diag_bounds_candidate_set",
  },
  OptimizationGuard {
    bench: "metrics_range_midspan",
    guard: "bench::index::metrics_range_midspan benchmark posture",
  },
  OptimizationGuard {
    bench: "validators_filtered_later_page",
    guard: "bench::index::validators_filtered_later_page benchmark posture",
  },
];

pub async fn explain_analyze_text(
  pool: &PgPool,
  select_sql: &str,
) -> Result<Vec<String>, DaemonError> {
  explain_text(pool, "EXPLAIN (ANALYZE, BUFFERS)", select_sql).await
}

pub async fn explain_plan_text(
  pool: &PgPool,
  select_sql: &str,
) -> Result<Vec<String>, DaemonError> {
  explain_text(pool, "EXPLAIN", select_sql).await
}

pub async fn table_size_snapshot(pool: &PgPool) -> Vec<(String, u64)> {
  crate::sql::snapshot::table_size_diag(pool).await
}

async fn explain_text(
  pool: &PgPool,
  explain_prefix: &str,
  select_sql: &str,
) -> Result<Vec<String>, DaemonError> {
  ensure_read_only_statement(select_sql)?;
  let statement = format!("{explain_prefix} {select_sql}");
  sqlx::query_scalar::<_, String>(&statement)
    .fetch_all(pool)
    .await
    .map_err(|error| DaemonError::Pg(format!("run benchmark validation explain: {error}")))
}

fn ensure_read_only_statement(select_sql: &str) -> Result<(), DaemonError> {
  let trimmed = select_sql.trim();
  if trimmed.is_empty() {
    return Err(DaemonError::Config(
      "benchmark validation explain requires a non-empty SQL statement".to_owned(),
    ));
  }
  if trimmed.contains(';') {
    return Err(DaemonError::Config(
      "benchmark validation explain accepts exactly one read-only statement".to_owned(),
    ));
  }
  let upper = trimmed.to_ascii_uppercase();
  if !(upper.starts_with("SELECT") || upper.starts_with("WITH")) {
    return Err(DaemonError::Config(
      "benchmark validation explain requires a SELECT or WITH statement".to_owned(),
    ));
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use std::collections::BTreeSet;

  use super::{OPTIMIZATION_BASELINES, OPTIMIZATION_GUARDS, VALIDATION_MATRIX};

  const BENCH_SOURCES: &[&str] = &[
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/bench/src/ingest.rs")),
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/bench/src/index.rs")),
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/bench/src/live.rs")),
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/bench/src/spent.rs")),
    include_str!(concat!(env!("CARGO_MANIFEST_DIR"), "/bench/src/write.rs")),
  ];

  const SQL_ROUTE_SOURCES: &[(&str, &str)] = &[
    ("ingest::admit", include_str!("../../ingest/admit.rs")),
    ("ingest::commit", include_str!("../../ingest/commit.rs")),
    ("ingest::fetch", include_str!("../../ingest/fetch.rs")),
    ("snapshot", include_str!("../snapshot.rs")),
    ("sync", include_str!("../sync.rs")),
    ("segment", include_str!("../segment.rs")),
    ("index::block", include_str!("../index/block.rs")),
    ("index::metrics", include_str!("../index/metrics.rs")),
    ("index::spent", include_str!("../index/spent.rs")),
    ("index::tx", include_str!("../index/tx.rs")),
    ("index::validator", include_str!("../index/validator.rs")),
    ("live::chainlock", include_str!("../live/chainlock.rs")),
    ("live::gov", include_str!("../live/gov.rs")),
    ("live::islock", include_str!("../live/islock.rs")),
    ("live::mempool", include_str!("../live/mempool.rs")),
    ("live::network", include_str!("../live/network.rs")),
    ("live::protx", include_str!("../live/protx.rs")),
  ];

  #[test]
  fn every_timed_sql_benchmark_has_validation_record() {
    let benches = discover_benchmark_names();
    let covered: BTreeSet<&str> = VALIDATION_MATRIX.iter().map(|entry| entry.bench).collect();
    let missing: Vec<_> = benches
      .iter()
      .filter(|bench| !covered.contains(bench.as_str()))
      .cloned()
      .collect();
    assert!(
      missing.is_empty(),
      "missing SQL benchmark validation records: {missing:?}"
    );
  }

  #[test]
  fn validation_records_match_real_benchmarks() {
    let benches = discover_benchmark_names();
    let stale: Vec<_> = VALIDATION_MATRIX
      .iter()
      .map(|entry| entry.bench)
      .filter(|bench| !benches.contains(*bench))
      .collect();
    assert!(
      stale.is_empty(),
      "stale SQL benchmark validation records: {stale:?}"
    );
  }

  #[test]
  fn validation_records_reference_real_sql_routes() {
    let routes = discover_sql_routes();
    let stale: Vec<_> = VALIDATION_MATRIX
      .iter()
      .flat_map(|entry| entry.owners.iter().copied())
      .filter(|route| !routes.contains(*route))
      .collect::<Vec<_>>();
    assert!(
      stale.is_empty(),
      "validation records reference unknown SQL routes: {stale:?}"
    );
  }

  #[test]
  fn optimization_baselines_reference_real_benchmarks() {
    let benches = discover_benchmark_names();
    let stale: Vec<_> = OPTIMIZATION_BASELINES
      .iter()
      .map(|entry| entry.bench)
      .filter(|bench| !benches.contains(*bench))
      .collect();
    assert!(
      stale.is_empty(),
      "optimization baselines reference unknown benchmarks: {stale:?}"
    );
  }

  #[test]
  fn optimization_guards_reference_real_benchmarks() {
    let benches = discover_benchmark_names();
    let stale: Vec<_> = OPTIMIZATION_GUARDS
      .iter()
      .map(|entry| entry.bench)
      .filter(|bench| !benches.contains(*bench))
      .collect();
    assert!(
      stale.is_empty(),
      "optimization guards reference unknown benchmarks: {stale:?}"
    );
  }

  #[test]
  fn every_optimization_baseline_has_regression_guard() {
    let guarded: BTreeSet<&str> = OPTIMIZATION_GUARDS
      .iter()
      .map(|entry| entry.bench)
      .collect();
    let missing: Vec<_> = OPTIMIZATION_BASELINES
      .iter()
      .map(|entry| entry.bench)
      .filter(|bench| !guarded.contains(bench))
      .collect();
    assert!(
      missing.is_empty(),
      "optimization baselines missing regression guards: {missing:?}"
    );
  }

  #[test]
  fn validation_notes_and_owners_are_non_empty() {
    for entry in VALIDATION_MATRIX {
      assert!(
        !entry.note.is_empty(),
        "validation records must document their posture"
      );
      assert!(
        !entry.owners.is_empty(),
        "validation records must map to at least one SQL owner"
      );
    }
  }

  fn discover_benchmark_names() -> BTreeSet<String> {
    let mut names = BTreeSet::new();
    for source in BENCH_SOURCES {
      for line in source.lines() {
        let trimmed = line.trim();
        let Some(rest) = trimmed.split_once("name = \"") else {
          continue;
        };
        let Some(name) = rest.1.split('"').next() else {
          continue;
        };
        names.insert(name.to_owned());
      }
    }
    names
  }

  fn discover_sql_routes() -> BTreeSet<String> {
    let mut routes = BTreeSet::new();
    for (module, source) in SQL_ROUTE_SOURCES {
      for line in source.lines() {
        let trimmed = line.trim_start();
        let prefix = if let Some(rest) = trimmed.strip_prefix("pub async fn ") {
          Some(rest)
        } else if let Some(rest) = trimmed.strip_prefix("pub fn ") {
          Some(rest)
        } else if let Some(rest) = trimmed.strip_prefix("pub(crate) async fn ") {
          Some(rest)
        } else if let Some(rest) = trimmed.strip_prefix("pub(crate) fn ") {
          Some(rest)
        } else {
          None
        };
        let Some(rest) = prefix else {
          continue;
        };
        let Some(name) = rest.split('(').next() else {
          continue;
        };
        routes.insert(format!("{module}::{name}"));
      }
    }
    routes
  }
}
