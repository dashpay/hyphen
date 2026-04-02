//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Benchmarks and seed helpers for write-plane SQL operations.

use divan::black_box;

use hyphend::bench::{self, BenchDataset, WorkloadGroup};
use hyphend::error::DaemonError;
use hyphend::index::Stage;

#[divan::bench(name = "blocks_upsert_one", ignore = !bench::should_run(WorkloadGroup::Write))]
fn blocks_upsert_one() {
  let hash = hyphend::sql::bench::synthetic_hash(b'W', 99);
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!("begin block upsert benchmark transaction: {error}"))
      })?;
      hyphend::sql::index::block::insert_block(
        &mut tx,
        1,
        hash.as_slice(),
        vec![0_u8; 32].as_slice(),
        hyphend::sql::bench::synthetic_hash(b'M', 99).as_slice(),
        1_700_000_099,
        486_604_799,
        99,
        3,
        1024,
        1,
        format!("{:064x}", 99).as_str(),
        &hyphend::index::block::util::CbTxFields::default(),
      )
      .await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback block upsert benchmark transaction: {error}"
        ))
      })?;
      Ok::<u64, DaemonError>(1)
    })
    .unwrap_or_else(|error| panic!("benchmark block upsert: {error}"));
  black_box(changed);
}

#[divan::bench(name = "mempool_upsert_one", ignore = !bench::should_run(WorkloadGroup::Write))]
fn mempool_upsert_one() {
  let txid = hyphend::sql::bench::synthetic_hash(b'w', 1);
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!(
          "begin mempool upsert benchmark transaction: {error}"
        ))
      })?;
      hyphend::sql::live::mempool::upsert_summary(
        &mut tx,
        &hyphend::sql::live::mempool::MempoolSummaryRow {
          tx_count: 1,
          total_bytes: 256,
          usage_bytes: Some(512),
          max_bytes: Some(100_000),
          updated_at: 1_700_000_500,
        },
      )
      .await?;
      hyphend::sql::live::mempool::upsert_recent_tx(
        &mut tx,
        &hyphend::sql::live::mempool::MempoolRecentTxRow {
          txid: txid.clone(),
          raw_tx: vec![0; 128],
          first_seen_at: 1_700_000_500,
          last_seen_at: 1_700_000_500,
        },
      )
      .await?;
      hyphend::sql::live::mempool::upsert_tx_detail(
        &mut tx,
        &hyphend::sql::live::mempool::MempoolTxDetailRow {
          txid: txid.clone(),
          version: 3,
          tx_type: "normal".to_owned(),
          lock_time: 0,
          size: 220,
          value_out: 1_000,
          input_count: 1,
          output_count: 1,
          fee: Some(10),
          payload: None,
          first_seen_at: 1_700_000_500,
          last_seen_at: 1_700_000_500,
        },
        &[],
        &[],
        &[],
      )
      .await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback mempool upsert benchmark transaction: {error}"
        ))
      })?;
      Ok::<u64, DaemonError>(1)
    })
    .unwrap_or_else(|error| panic!("benchmark mempool upsert: {error}"));
  black_box(changed);
}

#[divan::bench(name = "address_stats_delta_apply", ignore = !bench::should_run(WorkloadGroup::Write))]
fn address_stats_delta_apply() {
  let delta_rows = delta_rows(bench::runtime().dataset());
  let addresses: Vec<String> = (0..delta_rows)
    .map(|index| format!("XwriteDelta{index:05}"))
    .collect();
  let balances: Vec<i64> = vec![1_000; delta_rows as usize];
  let received: Vec<i64> = vec![2_000; delta_rows as usize];
  let tx_counts: Vec<i32> = vec![1; delta_rows as usize];
  let utxo_counts: Vec<i32> = vec![1; delta_rows as usize];
  let first_seen: Vec<Option<i32>> = vec![Some(1); delta_rows as usize];
  let last_seen: Vec<Option<i32>> = vec![Some(1); delta_rows as usize];
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!(
          "begin address delta benchmark transaction: {error}"
        ))
      })?;
      hyphend::sql::index::spent::apply_address_stats_chunk(
        &mut tx,
        &addresses,
        &balances,
        &received,
        &tx_counts,
        &utxo_counts,
        &first_seen,
        &last_seen,
      )
      .await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback address delta benchmark transaction: {error}"
        ))
      })?;
      Ok::<usize, DaemonError>(addresses.len())
    })
    .unwrap_or_else(|error| panic!("benchmark address delta apply: {error}"));
  black_box(changed);
}

#[divan::bench(name = "transactions_batch_insert", ignore = !bench::should_run(WorkloadGroup::Write))]
fn transactions_batch_insert() {
  let rows = delta_rows(bench::runtime().dataset()) as usize;
  let txids: Vec<Vec<u8>> = (0..rows)
    .map(|index| hyphend::sql::bench::synthetic_hash(b'Q', index as i64))
    .collect();
  let heights: Vec<i32> = vec![101; rows];
  let positions: Vec<i32> = (0..rows as i32).collect();
  let versions: Vec<i32> = vec![3; rows];
  let types: Vec<String> = vec!["normal".to_owned(); rows];
  let lock_times: Vec<i32> = vec![0; rows];
  let sizes: Vec<i32> = vec![250; rows];
  let values_out: Vec<i64> = vec![1_000; rows];
  let input_counts: Vec<i32> = vec![1; rows];
  let output_counts: Vec<i32> = vec![2; rows];
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!(
          "begin transactions batch benchmark transaction: {error}"
        ))
      })?;
      hyphend::sql::index::tx::insert_transactions(
        &mut tx,
        101,
        &txids,
        &heights,
        &positions,
        &versions,
        &types,
        &lock_times,
        &sizes,
        &values_out,
        &input_counts,
        &output_counts,
      )
      .await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback transactions batch benchmark transaction: {error}"
        ))
      })?;
      Ok::<usize, DaemonError>(rows)
    })
    .unwrap_or_else(|error| panic!("benchmark transactions batch insert: {error}"));
  black_box(changed);
}

#[divan::bench(name = "tx_inputs_batch_insert", ignore = !bench::should_run(WorkloadGroup::Write))]
fn tx_inputs_batch_insert() {
  let rows = delta_rows(bench::runtime().dataset()) as usize;
  let txids: Vec<Vec<u8>> = (0..rows)
    .map(|index| hyphend::sql::bench::synthetic_hash(b'I', index as i64))
    .collect();
  let heights: Vec<i32> = vec![101; rows];
  let positions: Vec<i32> = (0..rows as i32).collect();
  let versions: Vec<i32> = vec![3; rows];
  let types: Vec<String> = vec!["normal".to_owned(); rows];
  let lock_times: Vec<i32> = vec![0; rows];
  let sizes: Vec<i32> = vec![250; rows];
  let values_out: Vec<i64> = vec![1_000; rows];
  let input_counts: Vec<i32> = vec![1; rows];
  let output_counts: Vec<i32> = vec![2; rows];
  let vins: Vec<i32> = vec![0; rows];
  let prev_txids: Vec<Option<Vec<u8>>> = (0..rows)
    .map(|index| Some(hyphend::sql::bench::synthetic_hash(b'P', index as i64)))
    .collect();
  let prev_vouts: Vec<Option<i32>> = vec![Some(0); rows];
  let scripts: Vec<Vec<u8>> = vec![vec![0x51]; rows];
  let sequences: Vec<i64> = vec![u32::MAX as i64; rows];
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!(
          "begin tx_inputs batch benchmark transaction: {error}"
        ))
      })?;
      hyphend::sql::index::tx::insert_transactions(
        &mut tx,
        101,
        &txids,
        &heights,
        &positions,
        &versions,
        &types,
        &lock_times,
        &sizes,
        &values_out,
        &input_counts,
        &output_counts,
      )
      .await?;
      hyphend::sql::index::tx::insert_tx_inputs(
        &mut tx,
        101,
        &txids,
        &vins,
        &prev_txids,
        &prev_vouts,
        &scripts,
        &sequences,
      )
      .await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback tx_inputs batch benchmark transaction: {error}"
        ))
      })?;
      Ok::<usize, DaemonError>(rows)
    })
    .unwrap_or_else(|error| panic!("benchmark tx_inputs batch insert: {error}"));
  black_box(changed);
}

#[divan::bench(name = "address_history_batch_insert", ignore = !bench::should_run(WorkloadGroup::Write))]
fn address_history_batch_insert() {
  let rows = delta_rows(bench::runtime().dataset()) as usize;
  let addrs: Vec<String> = (0..rows).map(|index| format!("Xhist{index:05}")).collect();
  let txids: Vec<Vec<u8>> = (0..rows)
    .map(|index| hyphend::sql::bench::synthetic_hash(b'H', index as i64))
    .collect();
  let heights: Vec<i32> = vec![101; rows];
  let positions: Vec<i32> = (0..rows as i32).collect();
  let versions: Vec<i32> = vec![3; rows];
  let types: Vec<String> = vec!["normal".to_owned(); rows];
  let lock_times: Vec<i32> = vec![0; rows];
  let sizes: Vec<i32> = vec![250; rows];
  let values_out: Vec<i64> = vec![1_000; rows];
  let input_counts: Vec<i32> = vec![1; rows];
  let output_counts: Vec<i32> = vec![2; rows];
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!(
          "begin address history batch benchmark transaction: {error}"
        ))
      })?;
      hyphend::sql::index::tx::insert_transactions(
        &mut tx,
        101,
        &txids,
        &heights,
        &positions,
        &versions,
        &types,
        &lock_times,
        &sizes,
        &values_out,
        &input_counts,
        &output_counts,
      )
      .await?;
      hyphend::sql::index::spent::insert_address_history(
        &mut tx, &addrs, &txids, &heights, &positions,
      )
      .await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback address history batch benchmark transaction: {error}"
        ))
      })?;
      Ok::<usize, DaemonError>(rows)
    })
    .unwrap_or_else(|error| panic!("benchmark address history batch insert: {error}"));
  black_box(changed);
}

#[divan::bench(name = "address_balances_stage_chunk", ignore = !bench::should_run(WorkloadGroup::Write))]
fn address_balances_stage_chunk() {
  let rows = delta_rows(bench::runtime().dataset()) as usize;
  let addrs: Vec<String> = (0..rows)
    .map(|index| format!("XbalStage{index:05}"))
    .collect();
  let balances: Vec<i64> = vec![1_000; rows];
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!(
          "begin address balances stage benchmark transaction: {error}"
        ))
      })?;
      hyphend::sql::index::spent::prepare_address_balances_staging(&mut tx).await?;
      hyphend::sql::index::spent::stage_address_balances_delta(&mut tx, &addrs, &balances).await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback address balances stage benchmark transaction: {error}"
        ))
      })?;
      Ok::<usize, DaemonError>(rows)
    })
    .unwrap_or_else(|error| panic!("benchmark address balances stage chunk: {error}"));
  black_box(changed);
}

#[divan::bench(name = "address_balances_apply_chunk", ignore = !bench::should_run(WorkloadGroup::Write))]
fn address_balances_apply_chunk() {
  let rows = delta_rows(bench::runtime().dataset()) as usize;
  let existing = rows / 2;
  let addrs: Vec<String> = (0..rows)
    .map(|index| {
      if index < existing {
        format!("XbalSeed{index:05}")
      } else {
        format!("XbalApply{index:05}")
      }
    })
    .collect();
  let balances: Vec<i64> = vec![1_000; rows];
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!(
          "begin address balances apply benchmark transaction: {error}"
        ))
      })?;
      hyphend::sql::index::spent::prepare_address_balances_staging(&mut tx).await?;
      hyphend::sql::index::spent::stage_address_balances_delta(&mut tx, &addrs, &balances).await?;
      let staged = hyphend::sql::index::spent::staged_address_balances_count(&mut tx).await?;
      hyphend::sql::index::spent::apply_staged_address_balances_chunk(&mut tx, 0, staged).await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback address balances apply benchmark transaction: {error}"
        ))
      })?;
      Ok::<usize, DaemonError>(rows)
    })
    .unwrap_or_else(|error| panic!("benchmark address balances apply chunk: {error}"));
  black_box(changed);
}

#[divan::bench(name = "tx_outputs_stage_chunk", ignore = !bench::should_run(WorkloadGroup::Write))]
fn tx_outputs_stage_chunk() {
  let rows = delta_rows(bench::runtime().dataset()) as usize;
  let txids: Vec<Vec<u8>> = (0..rows)
    .map(|index| hyphend::sql::bench::synthetic_hash(b'O', index as i64))
    .collect();
  let vouts: Vec<i32> = vec![0; rows];
  let heights: Vec<i32> = vec![101; rows];
  let values: Vec<i64> = vec![1_000; rows];
  let scripts: Vec<Vec<u8>> = vec![vec![0x76, 0xa9, 0x14, 0x88, 0xac]; rows];
  let addrs: Vec<Option<String>> = (0..rows)
    .map(|index| Some(format!("Xout{index:05}")))
    .collect();
  let addr_types: Vec<Option<String>> = vec![Some("p2pkh".to_owned()); rows];
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!(
          "begin tx_outputs stage benchmark transaction: {error}"
        ))
      })?;
      hyphend::sql::index::tx::prepare_tx_outputs_staging(&mut tx).await?;
      hyphend::sql::index::tx::stage_tx_outputs(
        &mut tx,
        &txids,
        &vouts,
        &heights,
        &values,
        &scripts,
        &addrs,
        &addr_types,
      )
      .await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback tx_outputs stage benchmark transaction: {error}"
        ))
      })?;
      Ok::<usize, DaemonError>(rows)
    })
    .unwrap_or_else(|error| panic!("benchmark tx_outputs stage chunk: {error}"));
  black_box(changed);
}

#[divan::bench(name = "tx_outputs_apply_chunk", ignore = !bench::should_run(WorkloadGroup::Write))]
fn tx_outputs_apply_chunk() {
  let rows = delta_rows(bench::runtime().dataset()) as usize;
  let txids: Vec<Vec<u8>> = (0..rows)
    .map(|index| hyphend::sql::bench::synthetic_hash(b'Y', index as i64))
    .collect();
  let vouts: Vec<i32> = vec![0; rows];
  let heights: Vec<i32> = vec![101; rows];
  let positions: Vec<i32> = (0..rows as i32).collect();
  let versions: Vec<i32> = vec![3; rows];
  let types: Vec<String> = vec!["normal".to_owned(); rows];
  let lock_times: Vec<i32> = vec![0; rows];
  let sizes: Vec<i32> = vec![250; rows];
  let values: Vec<i64> = vec![1_000; rows];
  let input_counts: Vec<i32> = vec![1; rows];
  let output_counts: Vec<i32> = vec![1; rows];
  let scripts: Vec<Vec<u8>> = vec![vec![0x76, 0xa9, 0x14, 0x88, 0xac]; rows];
  let addrs: Vec<Option<String>> = (0..rows)
    .map(|index| Some(format!("Xapply{index:05}")))
    .collect();
  let addr_types: Vec<Option<String>> = vec![Some("p2pkh".to_owned()); rows];
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!(
          "begin tx_outputs apply benchmark transaction: {error}"
        ))
      })?;
      hyphend::sql::index::tx::insert_transactions(
        &mut tx,
        101,
        &txids,
        &heights,
        &positions,
        &versions,
        &types,
        &lock_times,
        &sizes,
        &values,
        &input_counts,
        &output_counts,
      )
      .await?;
      hyphend::sql::index::tx::prepare_tx_outputs_staging(&mut tx).await?;
      hyphend::sql::index::tx::stage_tx_outputs(
        &mut tx,
        &txids,
        &vouts,
        &heights,
        &values,
        &scripts,
        &addrs,
        &addr_types,
      )
      .await?;
      let staged = hyphend::sql::index::tx::staged_tx_outputs_count(&mut tx).await?;
      hyphend::sql::index::tx::apply_staged_tx_outputs_chunk(&mut tx, 101, 0, staged).await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback tx_outputs apply benchmark transaction: {error}"
        ))
      })?;
      Ok::<usize, DaemonError>(rows)
    })
    .unwrap_or_else(|error| panic!("benchmark tx_outputs apply chunk: {error}"));
  black_box(changed);
}

#[divan::bench(name = "sync_state_update_all", ignore = !bench::should_run(WorkloadGroup::Write))]
fn sync_state_update_all() {
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      hyphend::sql::sync::update_indexed_stage_height(pool, Stage::Block, 101).await?;
      hyphend::sql::sync::update_indexed_stage_height(pool, Stage::Transaction, 101).await?;
      hyphend::sql::sync::update_indexed_stage_height(pool, Stage::Script, 101).await?;
      hyphend::sql::sync::update_indexed_stage_height(pool, Stage::Spent, 101).await?;
      Ok::<u64, DaemonError>(4)
    })
    .unwrap_or_else(|error| panic!("benchmark sync_state updates: {error}"));
  black_box(changed);
}

#[divan::bench(
  name = "block_archive_delete_rollback",
  ignore = !bench::should_run(WorkloadGroup::Write)
)]
fn block_archive_delete_rollback() {
  let changed = bench::runtime()
    .run_async(async {
      let pool = bench::runtime().pool();
      let mut tx = pool.begin().await.map_err(|error| {
        DaemonError::Pg(format!("begin block delete benchmark transaction: {error}"))
      })?;
      hyphend::sql::index::block::delete_block(&mut tx, 1).await?;
      tx.rollback().await.map_err(|error| {
        DaemonError::Pg(format!(
          "rollback block delete benchmark transaction: {error}"
        ))
      })?;
      Ok::<u64, DaemonError>(1)
    })
    .unwrap_or_else(|error| panic!("benchmark block delete rollback: {error}"));
  black_box(changed);
}

#[divan::bench(name = "partition_ensure_existing", ignore = !bench::should_run(WorkloadGroup::Write))]
fn partition_ensure_existing() {
  let changed = bench::runtime()
    .run_async(async {
      hyphend::sql::segment::ensure_core_history_partitions(bench::runtime().pool()).await?;
      Ok::<u64, DaemonError>(1)
    })
    .unwrap_or_else(|error| panic!("benchmark partition ensure: {error}"));
  black_box(changed);
}

#[divan::bench(name = "table_size_diag_snapshot", ignore = !bench::should_run(WorkloadGroup::Write))]
fn table_size_diag_snapshot() {
  let rows = bench::runtime().run_async(hyphend::sql::snapshot::table_size_diag(
    bench::runtime().pool(),
  ));
  black_box(rows.len());
}

fn delta_rows(dataset: BenchDataset) -> i32 {
  match dataset {
    BenchDataset::Small => 500,
    BenchDataset::Default => 2_000,
    BenchDataset::Large => 10_000,
  }
}
