//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Audit benchmark coverage across the public SQL surface.

use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy)]
enum CoverageKind {
  Bench(&'static str),
  CoveredBy(&'static str),
  Excluded(&'static str),
}

#[derive(Clone, Copy)]
struct CoverageEntry {
  route: &'static str,
  kind: CoverageKind,
}

macro_rules! bench {
  ($route:literal, $name:literal) => {
    CoverageEntry {
      route: $route,
      kind: CoverageKind::Bench($name),
    }
  };
}

macro_rules! covered_by {
  ($route:literal, $name:literal) => {
    CoverageEntry {
      route: $route,
      kind: CoverageKind::CoveredBy($name),
    }
  };
}

macro_rules! excluded {
  ($route:literal, $reason:literal) => {
    CoverageEntry {
      route: $route,
      kind: CoverageKind::Excluded($reason),
    }
  };
}

const SQL_ROUTE_SOURCES: &[(&str, &str)] = &[
  ("sql", include_str!("mod.rs")),
  ("segment", include_str!("segment.rs")),
  ("snapshot", include_str!("snapshot.rs")),
  ("sync", include_str!("sync.rs")),
  ("init::apply", include_str!("init/apply.rs")),
  ("init::reset", include_str!("init/reset.rs")),
  ("init::validate", include_str!("init/validate.rs")),
  ("verbs::reindex", include_str!("verbs/reindex.rs")),
  ("verbs::rewind", include_str!("verbs/rewind.rs")),
  ("index::block", include_str!("index/block.rs")),
  ("index::metrics", include_str!("index/metrics.rs")),
  ("index::script", include_str!("index/script.rs")),
  ("index::spent", include_str!("index/spent.rs")),
  ("index::tx", include_str!("index/tx.rs")),
  ("index::validator", include_str!("index/validator.rs")),
  ("live::chainlock", include_str!("live/chainlock.rs")),
  ("live::gov", include_str!("live/gov.rs")),
  ("live::islock", include_str!("live/islock.rs")),
  ("live::mempool", include_str!("live/mempool.rs")),
  ("live::network", include_str!("live/network.rs")),
  ("live::protx", include_str!("live/protx.rs")),
];

const COVERAGE: &[CoverageEntry] = &[
  excluded!("sql::create_pools", "connection bootstrap"),
  excluded!("sql::connect_test_pool", "test helper"),
  bench!(
    "segment::ensure_core_history_partitions",
    "partition_ensure_existing"
  ),
  excluded!("segment::history_partition_sqls", "pure SQL builder"),
  excluded!("segment::history_partition_start", "pure helper"),
  excluded!("segment::canonical_partition_sqls", "pure SQL builder"),
  excluded!("segment::canonical_partition_start", "pure helper"),
  excluded!(
    "segment::ensure_script_output_history_partition_sql",
    "pure SQL builder"
  ),
  excluded!(
    "segment::ensure_transactions_partition_sql",
    "pure SQL builder"
  ),
  excluded!(
    "segment::ensure_spent_output_history_partition_sql",
    "pure SQL builder"
  ),
  excluded!(
    "segment::ensure_tx_outputs_partition_sql",
    "pure SQL builder"
  ),
  excluded!(
    "segment::ensure_core_canonical_partitions",
    "partition bootstrap helper"
  ),
  excluded!("snapshot::startup_diag", "admin diagnostic snapshot"),
  excluded!("snapshot::database_size_diag", "admin diagnostic snapshot"),
  bench!("snapshot::table_size_diag", "table_size_diag_snapshot"),
  excluded!("snapshot::wal_size_diag", "admin diagnostic snapshot"),
  covered_by!("sync::indexed_height", "sync_state_update_all"),
  excluded!(
    "sync::indexed_stage_height",
    "typed helper around sync state"
  ),
  excluded!(
    "sync::durable_stage_height",
    "typed helper around sync state"
  ),
  bench!("sync::update_indexed_stage_height", "sync_state_update_all"),
  covered_by!(
    "sync::update_indexed_stage_height_tx",
    "sync_state_update_all"
  ),
  excluded!("sync::indexed_stage_height_tx", "transaction helper"),
  excluded!("sync::block_hash_at_height_tx", "transaction helper"),
  excluded!("sync::set_local_async_commit", "transaction session helper"),
  excluded!("sync::sync_state_rows", "diagnostic helper"),
  excluded!("sync::table_row_count", "test and diagnostic helper"),
  excluded!("init::apply::bootstrap", "schema bootstrap"),
  excluded!("init::reset::reset", "destructive maintenance verb"),
  excluded!(
    "init::reset::targeted_reset",
    "destructive maintenance verb"
  ),
  excluded!("init::validate::validate", "schema validation helper"),
  excluded!(
    "verbs::reindex::force_reindex_stage",
    "maintenance orchestration"
  ),
  excluded!(
    "verbs::reindex::force_reindex_stage_shared",
    "maintenance orchestration"
  ),
  excluded!(
    "verbs::rewind::rewind_target_after_disconnect",
    "pure helper"
  ),
  excluded!(
    "verbs::rewind::rewind_to_height",
    "maintenance orchestration"
  ),
  excluded!(
    "verbs::rewind::rewind_to_height_shared",
    "maintenance orchestration"
  ),
  excluded!("verbs::rewind::disconnect_tip", "maintenance orchestration"),
  excluded!(
    "verbs::rewind::disconnect_tip_shared",
    "maintenance orchestration"
  ),
  covered_by!("index::block::prev_chainwork", "blocks_upsert_one"),
  bench!("index::block::list_blocks", "blocks_list_later_page"),
  covered_by!("index::block::block_by_height", "block_detail_by_hash"),
  bench!("index::block::block_by_hash", "block_detail_by_hash"),
  excluded!(
    "index::block::search_blocks_by_prefix",
    "search helper has no dedicated benchmark yet"
  ),
  covered_by!("index::block::orphan_block_by_hash", "block_detail_by_hash"),
  covered_by!("index::block::block_txs", "block_detail_by_hash"),
  covered_by!("index::block::block_exists", "blocks_upsert_one"),
  covered_by!(
    "index::block::block_height_for_hash",
    "block_detail_by_hash"
  ),
  covered_by!(
    "index::block::block_hash_for_height",
    "block_detail_by_hash"
  ),
  bench!("index::block::insert_block", "blocks_upsert_one"),
  excluded!(
    "index::block::insert_orphan_block_for_tests",
    "test-only helper for rest shaping coverage"
  ),
  bench!(
    "index::block::delete_block",
    "block_archive_delete_rollback"
  ),
  covered_by!(
    "index::metrics::query_prev_metrics",
    "metrics_range_midspan"
  ),
  covered_by!(
    "index::metrics::insert_block_metrics",
    "metrics_range_midspan"
  ),
  covered_by!("index::metrics::latest_metrics", "metrics_range_midspan"),
  bench!("index::metrics::metrics_range", "metrics_range_midspan"),
  bench!(
    "index::metrics::metrics_bucketed_range",
    "metrics_chart_bucketed_midspan"
  ),
  excluded!(
    "index::metrics::bench_metrics_bucketed_range",
    "bench-only wrapper around the production chart SQL path"
  ),
  covered_by!(
    "index::metrics::delete_block_metrics",
    "metrics_range_midspan"
  ),
  covered_by!(
    "index::script::ensure_output_history_partition",
    "partition_ensure_existing"
  ),
  excluded!(
    "index::script::restore_output_addresses",
    "disconnect helper"
  ),
  covered_by!(
    "index::spent::ensure_history_partitions",
    "partition_ensure_existing"
  ),
  bench!("index::spent::address_summary", "address_summary_whale"),
  covered_by!("index::spent::address_exists", "address_summary_whale"),
  covered_by!(
    "index::spent::block_time_by_height",
    "address_history_later_page_whale"
  ),
  bench!(
    "index::spent::address_history_txs",
    "address_history_later_page_whale"
  ),
  excluded!(
    "index::spent::hydrate_address_history_page",
    "store-backed hydration helper has no dedicated benchmark yet"
  ),
  covered_by!("index::spent::address_utxos", "address_summary_whale"),
  bench!("index::spent::balances_page", "balances_later_page"),
  bench!(
    "index::spent::balances_summary",
    "balances_summary_snapshot"
  ),
  covered_by!(
    "index::spent::lookup_prev_outputs",
    "tx_inputs_batch_insert"
  ),
  covered_by!(
    "index::spent::prepare_prevout_stage",
    "tx_inputs_batch_insert"
  ),
  covered_by!(
    "index::spent::stage_prevout_snapshot_rows",
    "tx_inputs_batch_insert"
  ),
  covered_by!(
    "index::spent::staged_prevout_count",
    "tx_inputs_batch_insert"
  ),
  covered_by!(
    "index::spent::insert_spent_output_history",
    "address_history_batch_insert"
  ),
  covered_by!(
    "index::spent::mark_outputs_spent_chunk",
    "address_history_batch_insert"
  ),
  covered_by!(
    "index::spent::batch_update_tx_inputs",
    "tx_inputs_batch_insert"
  ),
  covered_by!(
    "index::spent::batch_update_fees",
    "transactions_batch_insert"
  ),
  bench!(
    "index::spent::insert_address_history",
    "address_history_batch_insert"
  ),
  bench!(
    "index::spent::stage_address_balances_delta",
    "address_balances_stage_chunk"
  ),
  bench!(
    "index::spent::apply_address_stats_chunk",
    "address_stats_delta_apply"
  ),
  excluded!(
    "index::spent::apply_address_balances_chunk",
    "write helper has no dedicated benchmark yet"
  ),
  excluded!(
    "index::spent::apply_address_balances_summary_chunk",
    "summary maintenance helper has no dedicated benchmark yet"
  ),
  excluded!(
    "index::spent::apply_address_stats_summary_chunk",
    "summary maintenance helper has no dedicated benchmark yet"
  ),
  covered_by!(
    "index::spent::prepare_address_balances_staging",
    "address_balances_stage_chunk"
  ),
  covered_by!(
    "index::spent::staged_address_balances_count",
    "address_balances_apply_chunk"
  ),
  bench!(
    "index::spent::apply_staged_address_balances_chunk",
    "address_balances_apply_chunk"
  ),
  excluded!(
    "index::spent::apply_staged_address_balances_summary_chunk",
    "summary maintenance helper has no dedicated benchmark yet"
  ),
  excluded!(
    "index::spent::refresh_address_balances_summary",
    "maintenance refresh helper"
  ),
  excluded!(
    "index::spent::refresh_address_balances_summary_pool",
    "maintenance refresh helper"
  ),
  excluded!(
    "index::spent::search_addresses_by_prefix",
    "search helper has no dedicated benchmark yet"
  ),
  excluded!(
    "index::spent::seed_address_history_tx_for_rest_tests",
    "test helper"
  ),
  excluded!("index::spent::restore_spent_at_height", "disconnect helper"),
  excluded!(
    "index::spent::reverse_address_balances_at_height",
    "disconnect helper"
  ),
  excluded!(
    "index::spent::reverse_address_history_at_height",
    "disconnect helper"
  ),
  excluded!(
    "index::spent::reverse_address_stats_at_height",
    "disconnect helper"
  ),
  covered_by!("index::tx::transaction", "transactions_list_later_page"),
  bench!(
    "index::tx::transaction_inputs",
    "transaction_inputs_prevout_fallback"
  ),
  covered_by!("index::tx::transaction_outputs", "tx_outputs_apply_chunk"),
  covered_by!(
    "index::tx::transaction_payload",
    "transactions_list_later_page"
  ),
  covered_by!(
    "index::tx::transaction_location",
    "transactions_list_later_page"
  ),
  covered_by!(
    "index::tx::insert_transaction_row",
    "transactions_batch_insert"
  ),
  covered_by!(
    "index::tx::insert_transaction_input_rows",
    "tx_inputs_batch_insert"
  ),
  covered_by!(
    "index::tx::insert_transaction_output_rows",
    "tx_outputs_apply_chunk"
  ),
  bench!(
    "index::tx::list_transactions",
    "transactions_list_later_page"
  ),
  excluded!(
    "index::tx::delete_transactions_exact_summary",
    "delete-path helper has no dedicated benchmark yet"
  ),
  excluded!(
    "index::tx::ensure_canonical_partitions",
    "partition bootstrap helper"
  ),
  bench!(
    "index::tx::insert_transactions",
    "transactions_batch_insert"
  ),
  bench!("index::tx::insert_tx_inputs", "tx_inputs_batch_insert"),
  covered_by!(
    "index::tx::prepare_tx_outputs_staging",
    "tx_outputs_stage_chunk"
  ),
  bench!("index::tx::stage_tx_outputs", "tx_outputs_stage_chunk"),
  covered_by!(
    "index::tx::staged_tx_outputs_count",
    "tx_outputs_apply_chunk"
  ),
  bench!(
    "index::tx::apply_staged_tx_outputs_chunk",
    "tx_outputs_apply_chunk"
  ),
  covered_by!(
    "index::tx::insert_special_tx_payloads",
    "transactions_batch_insert"
  ),
  covered_by!(
    "index::tx::delete_tx_outputs",
    "block_archive_delete_rollback"
  ),
  covered_by!(
    "index::tx::delete_tx_inputs",
    "block_archive_delete_rollback"
  ),
  covered_by!(
    "index::tx::delete_special_tx_payloads",
    "block_archive_delete_rollback"
  ),
  covered_by!(
    "index::tx::delete_transactions",
    "block_archive_delete_rollback"
  ),
  excluded!(
    "index::tx::refresh_transactions_summary",
    "summary maintenance helper"
  ),
  excluded!(
    "index::tx::search_transactions_by_prefix",
    "search helper has no dedicated benchmark yet"
  ),
  excluded!("index::tx::transactions_summary", "summary read helper"),
  bench!(
    "index::validator::list_validators",
    "validators_filtered_later_page"
  ),
  covered_by!("index::validator::validator", "validator_history_detail"),
  bench!(
    "index::validator::validator_history",
    "validator_history_detail"
  ),
  covered_by!(
    "index::validator::insert_validator",
    "validators_filtered_later_page"
  ),
  covered_by!(
    "index::validator::update_service",
    "validators_filtered_later_page"
  ),
  covered_by!(
    "index::validator::update_registrar",
    "validators_filtered_later_page"
  ),
  covered_by!("index::validator::revoke", "validators_filtered_later_page"),
  covered_by!(
    "index::validator::restore_history",
    "validator_history_detail"
  ),
  excluded!(
    "index::validator::block_times_by_height",
    "hydration helper has no dedicated benchmark yet"
  ),
  excluded!(
    "index::validator::refresh_validator_counts",
    "summary maintenance helper"
  ),
  excluded!(
    "index::validator::refresh_validator_counts_tx",
    "summary maintenance helper"
  ),
  excluded!(
    "index::validator::search_validators_by_prefix",
    "search helper has no dedicated benchmark yet"
  ),
  excluded!("index::validator::validator_count", "count helper"),
  covered_by!(
    "live::chainlock::upsert_snapshot",
    "chainlock_snapshot_read"
  ),
  bench!("live::chainlock::snapshot", "chainlock_snapshot_read"),
  covered_by!("live::gov::replace_objects", "governance_object_list"),
  covered_by!("live::gov::upsert_object", "governance_object_list"),
  covered_by!("live::gov::replace_votes", "governance_votes_later_page"),
  covered_by!(
    "live::gov::replace_votes_for_object",
    "governance_votes_later_page"
  ),
  bench!("live::gov::objects", "governance_object_list"),
  covered_by!("live::gov::all_objects", "governance_object_list"),
  covered_by!("live::gov::object", "governance_object_list"),
  bench!("live::gov::votes", "governance_votes_later_page"),
  excluded!("live::gov::governance_summary", "summary read helper"),
  excluded!(
    "live::gov::search_objects_by_query",
    "search helper has no dedicated benchmark yet"
  ),
  covered_by!("live::islock::upsert_current", "islock_lookup"),
  covered_by!("live::islock::delete_current", "islock_lookup"),
  bench!("live::islock::current_by_txid", "islock_lookup"),
  bench!("live::mempool::upsert_summary", "mempool_upsert_one"),
  bench!("live::mempool::upsert_recent_tx", "mempool_upsert_one"),
  covered_by!("live::mempool::delete_recent_tx", "mempool_upsert_one"),
  bench!("live::mempool::upsert_tx_detail", "mempool_upsert_one"),
  covered_by!("live::mempool::delete_tx_detail", "mempool_upsert_one"),
  covered_by!("live::mempool::clear_recent_snapshot", "mempool_upsert_one"),
  covered_by!(
    "live::mempool::prune_recent_txs",
    "mempool_recent_later_page"
  ),
  bench!("live::mempool::summary", "mempool_summary_read"),
  bench!("live::mempool::recent_txs", "mempool_recent_later_page"),
  covered_by!(
    "live::mempool::recent_tx_by_txid",
    "mempool_recent_later_page"
  ),
  bench!("live::mempool::tx_detail", "mempool_detail_inputs_outputs"),
  covered_by!(
    "live::mempool::recent_tx_details",
    "mempool_detail_inputs_outputs"
  ),
  bench!("live::mempool::tx_inputs", "mempool_detail_inputs_outputs"),
  bench!("live::mempool::tx_outputs", "mempool_detail_inputs_outputs"),
  bench!("live::mempool::address_txs_page", "mempool_address_page"),
  covered_by!("live::mempool::address_tx_count", "mempool_address_page"),
  covered_by!("live::network::upsert_snapshot", "network_snapshot_read"),
  bench!("live::network::snapshot", "network_snapshot_read"),
  covered_by!(
    "live::protx::replace_snapshot",
    "validator_protx_snapshot_scan"
  ),
  bench!("live::protx::snapshot", "validator_protx_snapshot_scan"),
  excluded!("live::protx::summary", "summary read helper"),
  excluded!("sql::cancel_daemon_backends", "admin helper"),
  excluded!(
    "verbs::rewind::reconcile_postgres_with_local_rocks",
    "destructive maintenance verb"
  ),
];

fn discover_routes() -> BTreeSet<String> {
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

#[test]
fn every_public_sql_route_has_coverage() {
  let discovered = discover_routes();
  let covered: BTreeSet<&str> = COVERAGE.iter().map(|entry| entry.route).collect();
  let missing: Vec<_> = discovered
    .iter()
    .filter(|route| !covered.contains(route.as_str()))
    .cloned()
    .collect();
  assert!(
    missing.is_empty(),
    "missing SQL benchmark coverage entries: {missing:?}"
  );
}

#[test]
fn coverage_entries_match_real_sql_routes() {
  let discovered = discover_routes();
  let stale: Vec<_> = COVERAGE
    .iter()
    .filter(|entry| !discovered.contains(entry.route))
    .map(|entry| entry.route)
    .collect();
  assert!(
    stale.is_empty(),
    "stale SQL benchmark coverage entries: {stale:?}"
  );
}

#[test]
fn coverage_entries_are_unique() {
  let mut seen = BTreeMap::new();
  for entry in COVERAGE {
    let prior = seen.insert(entry.route, entry.kind);
    assert!(
      prior.is_none(),
      "duplicate SQL coverage entry: {}",
      entry.route
    );
  }
}

#[test]
fn direct_benches_stay_meaningful() {
  for entry in COVERAGE {
    match entry.kind {
      CoverageKind::Bench(name) | CoverageKind::CoveredBy(name) => {
        assert!(!name.is_empty(), "coverage mapping must name a benchmark")
      }
      CoverageKind::Excluded(reason) => {
        assert!(
          !reason.is_empty(),
          "excluded coverage entries must explain why"
        )
      }
    }
  }
}
