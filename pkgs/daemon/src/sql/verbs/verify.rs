//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Read-only health-check for PostgreSQL index stages.

use std::sync::Arc;

use tracing::{info, warn};

use crate::db;
use crate::error::DaemonError;
use crate::index::Stage;
use crate::sql;

/// Per-stage health report entry.
#[derive(Debug, Clone)]
pub struct StageHealth {
  pub stage: &'static str,
  pub claimed_height: i32,
  pub durable_height: Option<i32>,
  pub healthy: bool,
}

/// Overall index health report.
#[derive(Debug, Clone)]
pub struct IndexHealthReport {
  pub rocks_tip: Option<u32>,
  pub pg_block_height: i32,
  pub rocks_covers_pg: bool,
  pub undo_tip: i32,
  pub canonical_tip: i32,
  pub undo_coverage_ok: bool,
  pub stages: Vec<StageHealth>,
}

impl IndexHealthReport {
  pub fn is_healthy(&self) -> bool {
    self.rocks_covers_pg && self.undo_coverage_ok && self.stages.iter().all(|s| s.healthy)
  }
}

const ALL_STAGES: &[Stage] = &[
  Stage::Block,
  Stage::Transaction,
  Stage::Script,
  Stage::Spent,
  Stage::Metrics,
  Stage::Validator,
];

/// Run a read-only health check against one or all PostgreSQL index stages.
///
/// Returns `Ok(report)` with the diagnostic results.  The caller decides
/// whether to treat a failed report as an error exit code.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when a required query fails.
pub async fn verify_index(
  pool: &sqlx::PgPool,
  db: &Arc<rocksdb::DB>,
  stage_filter: Option<&str>,
) -> Result<IndexHealthReport, DaemonError> {
  let rocks_tip = db::get_tip_height(db);
  let pg_block_height =
    sql::query::indexed_stage_height(pool, Stage::Block).await?;

  let rocks_covers_pg = match rocks_tip {
    Some(tip) => pg_block_height < 0 || (pg_block_height as u32) <= tip,
    None => pg_block_height < 0,
  };

  // Undo coverage.
  let (canonical_tip, undo_tip, undo_coverage_ok) =
    match sql::progress::chain_progress(pool).await {
      Ok(progress) => {
        let ok = if progress.canonical_tip_height < 0 {
          true
        } else {
          let expected = i64::from(progress.canonical_tip_height) + 1;
          let covered = sql::progress::count_block_undo_rows_through(
            pool,
            progress.canonical_tip_height,
          )
          .await?;
          progress.undo_tip_height >= progress.canonical_tip_height && covered == expected
        };
        (progress.canonical_tip_height, progress.undo_tip_height, ok)
      }
      Err(error) => {
        warn!(%error, "chain_progress table missing or unreadable");
        (pg_block_height, -1, false)
      }
    };

  // Per-stage checks.
  let stages_to_check: Vec<Stage> = match stage_filter {
    Some(name) => {
      let stage = Stage::from_name(name).ok_or_else(|| {
        DaemonError::Config(format!("unknown index stage: {name}"))
      })?;
      vec![stage]
    }
    None => ALL_STAGES.to_vec(),
  };

  let mut stages = Vec::new();
  for stage in stages_to_check {
    let claimed = match sql::query::indexed_stage_height(pool, stage).await {
      Ok(h) => h,
      Err(error) => {
        warn!(
          stage = stage.name(),
          %error,
          "stage sync_state row missing or unreadable"
        );
        stages.push(StageHealth {
          stage: stage.name(),
          claimed_height: -1,
          durable_height: None,
          healthy: false,
        });
        continue;
      }
    };

    let durable = match stage.durable_height_query() {
      Some(_) => Some(sql::query::durable_stage_height(pool, stage).await?),
      None => {
        info!(
          stage = stage.name(),
          claimed = claimed,
          "stage has no durable height query; skipping cross-check"
        );
        None
      }
    };

    let healthy = match durable {
      Some(d) => claimed <= d,
      None => true,
    };

    stages.push(StageHealth {
      stage: stage.name(),
      claimed_height: claimed,
      durable_height: durable,
      healthy,
    });
  }

  let report = IndexHealthReport {
    rocks_tip,
    pg_block_height,
    rocks_covers_pg,
    undo_tip,
    canonical_tip,
    undo_coverage_ok,
    stages,
  };

  // Log the report.
  if report.is_healthy() {
    info!("index health: OK");
  } else {
    warn!("index health: ISSUES DETECTED");
  }
  info!(
    rocks_tip = ?report.rocks_tip,
    pg_block_height = report.pg_block_height,
    rocks_covers_pg = report.rocks_covers_pg,
    "RocksDB / PostgreSQL coverage"
  );
  info!(
    canonical_tip = report.canonical_tip,
    undo_tip = report.undo_tip,
    undo_coverage_ok = report.undo_coverage_ok,
    "undo coverage"
  );
  for s in &report.stages {
    if s.healthy {
      info!(
        stage = s.stage,
        claimed = s.claimed_height,
        durable = ?s.durable_height,
        "stage OK"
      );
    } else {
      warn!(
        stage = s.stage,
        claimed = s.claimed_height,
        durable = ?s.durable_height,
        "stage MISMATCH: claimed height exceeds durable data"
      );
    }
  }

  Ok(report)
}
