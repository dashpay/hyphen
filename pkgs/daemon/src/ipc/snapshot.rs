//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Heartbeat snapshot builder for IPC broadcast.

use std::path::Path;
use std::time::{Duration, Instant};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::api_metrics::ApiServerMetrics;
use crate::db;
use crate::index::batch::BatchSizerRegistry;
use crate::index::BlockIndexerWorker;
use crate::index::Stage;
use crate::ingest::IngestRuntime;
use crate::init::Options;
use crate::policy::api::API_DATA_SOURCE_TIMEOUT_SECS;
use crate::readiness::ReadinessCache;
use crate::sources::{RestSource, RpcSource, ZmqSource};
use crate::store::manager::SecondaryStoreSet;
use hyphend::wire::msg::{
  ConfigSnapshot, CorePipelineDiag, CorePipelineHealth, DaemonSnapshot, IndexSchedulerLane,
  PostgresDiag, RestSourceState, RpcDiag, SecondaryStoreDiag, StageProgress, ZmqDiag,
};

const PG_DB_SIZE_REFRESH_SECS: u64 = 30;
const PG_TABLE_SIZE_REFRESH_SECS: u64 = 30;
const PG_WAL_SIZE_REFRESH_SECS: u64 = 10;
const FORWARD_STALE_SECS: u64 = 30;
const SCHEDULER_STALE_SECS: u64 = 15;

/// Read-only shared state for heartbeat snapshot building.
pub struct HeartbeatCtx<'a> {
  pub options: &'a Options,
  pub write_pool: &'a crate::sql::SqlPool,
  pub query_pool: &'a crate::sql::SqlPool,
  pub admin_pool: &'a crate::sql::SqlPool,
  pub db: &'a rocksdb::DB,
  pub db_path: &'a Path,
  pub secondary_stores: &'a SecondaryStoreSet,
  pub source: &'a dyn RestSource,
  pub rpc: Option<&'a dyn RpcSource>,
  pub zmq: Option<&'a dyn ZmqSource>,
  pub ingestion_runtime: &'a IngestRuntime,
  pub block_worker: &'a BlockIndexerWorker,
  pub batch_registry: &'a BatchSizerRegistry,
  pub config_snap: &'a ConfigSnapshot,
  pub pg_diag_base: &'a PostgresDiag,
  pub api_metrics: &'a ApiServerMetrics,
  pub readiness_cache: &'a ReadinessCache,
  pub start_time: Instant,
  pub pausing_rx: &'a tokio::sync::watch::Receiver<std::collections::BTreeSet<String>>,
  pub suspend_rx: &'a tokio::sync::watch::Receiver<std::collections::BTreeSet<String>>,
}

impl std::fmt::Debug for HeartbeatCtx<'_> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("HeartbeatCtx")
      .field("options", &self.options)
      .field("write_pool", &"<PgPool>")
      .field("query_pool", &"<PgPool>")
      .field("admin_pool", &"<PgPool>")
      .field("db", &"<RocksDb>")
      .field("db_path", &self.db_path)
      .field("secondary_stores", &"<SecondaryStoreSet>")
      .field("source", &"<RestSource>")
      .field("rpc", &self.rpc.as_ref().map(|_| "<RpcSource>"))
      .field("zmq", &self.zmq.as_ref().map(|_| "<ZmqSource>"))
      .field("ingestion_runtime", &self.ingestion_runtime.snapshot())
      .field("block_worker", &"<BlockIndexerWorker>")
      .field("batch_registry", &self.batch_registry)
      .field("config_snap", &self.config_snap)
      .field("pg_diag_base", &self.pg_diag_base)
      .field("api_metrics", &self.api_metrics)
      .field("readiness_cache", &self.readiness_cache)
      .field("start_time", &self.start_time)
      .field("pausing_rx", &"<watch::Receiver>")
      .field("suspend_rx", &"<watch::Receiver>")
      .finish()
  }
}

/// Mutable state that persists across heartbeat ticks.
#[derive(Debug, Default)]
pub struct HeartbeatState {
  pub pg_backoff: u64,
  pub pg_skip: u64,
  pub pg_sizes: crate::sql::snapshot::PgSizeDiag,
  pub pg_db_refreshed_at: Option<Instant>,
  pub pg_table_refreshed_at: Option<Instant>,
  pub pg_wal_refreshed_at: Option<Instant>,
}

impl HeartbeatState {
  pub fn new() -> Self {
    Self {
      pg_backoff: 0,
      pg_skip: 0,
      pg_sizes: crate::sql::snapshot::PgSizeDiag::default(),
      pg_db_refreshed_at: None,
      pg_table_refreshed_at: None,
      pg_wal_refreshed_at: None,
    }
  }
}

/// Query Postgres version and config once at startup.
pub async fn pg_startup_diag(
  pool: &crate::sql::SqlPool,
  write_pool_size: usize,
  query_pool_size: usize,
  admin_pool_size: usize,
) -> PostgresDiag {
  crate::sql::snapshot::startup_diag(pool, write_pool_size, query_pool_size, admin_pool_size).await
}

/// Redact credentials from a Postgres connection URL.
pub fn redact_pg_url(url: &str) -> String {
  if let Some(at_pos) = url.find('@') {
    if let Some(proto_end) = url.find("://") {
      let proto = &url[..proto_end + 3];
      let rest = &url[at_pos..];
      return format!("{proto}***:***{rest}");
    }
  }
  url.to_owned()
}

/// Build one heartbeat snapshot.
///
/// This folds together global daemon state, per-stage sync rows, and live
/// datasource diagnostics into the wire-format snapshot sent to the console.
pub async fn build_snapshot(
  ctx: &HeartbeatCtx<'_>,
  schema_errors: &[String],
  state: &mut HeartbeatState,
) -> DaemonSnapshot {
  let ping_ok = tokio::time::timeout(
    std::time::Duration::from_secs(API_DATA_SOURCE_TIMEOUT_SECS),
    ctx.source.ping(),
  )
  .await
  .is_ok();
  let ingest = ctx.ingestion_runtime.snapshot();
  let ingesting = ingest.blocks_ingested_total > 0;
  let data_source = if ping_ok || ingesting {
    RestSourceState::Connected
  } else {
    RestSourceState::Disconnected
  };

  let network_tip = tokio::time::timeout(
    std::time::Duration::from_secs(API_DATA_SOURCE_TIMEOUT_SECS),
    ctx.source.block_count(),
  )
  .await
  .ok()
  .and_then(|r| r.ok());

  let stage_enabled = |stage: Stage| -> bool {
    stage.enabled_with_flags(crate::index::IndexStageFlags {
      address: ctx.options.indices.address,
      metrics: ctx.options.indices.metrics,
      validator: ctx.options.indices.validator,
    })
  };

  // Query stage progress from Postgres
  let mut stages: Vec<StageProgress> = if state.pg_skip > 0 {
    state.pg_skip -= 1;
    vec![]
  } else {
    match crate::sql::query::sync_state_rows(ctx.admin_pool).await {
      Ok(rows) => {
        state.pg_backoff = 0;
        ctx.readiness_cache.replace_from_rows(&rows).await;
        let batch_snaps = ctx.batch_registry.snapshot();
        rows
          .iter()
          .map(|r| {
            let batch = batch_snaps
              .iter()
              .find(|snapshot| snapshot.stage == r.stage);
            match Stage::from_name(&r.stage) {
              Some(Stage::Block) => crate::index::block::ipc::progress_row(r, network_tip, batch),
              Some(Stage::Transaction) => {
                crate::index::tx::ipc::progress_row(r, network_tip, batch)
              }
              Some(Stage::Script) => crate::index::script::ipc::progress_row(
                r,
                network_tip,
                stage_enabled(Stage::Script),
                batch,
              ),
              Some(Stage::Spent) => crate::index::spent::ipc::progress_row(
                r,
                network_tip,
                stage_enabled(Stage::Spent),
                batch,
              ),
              Some(Stage::Metrics) => crate::index::metrics::ipc::progress_row(
                r,
                network_tip,
                stage_enabled(Stage::Metrics),
                batch,
              ),
              Some(Stage::Validator) => crate::index::validator::ipc::progress_row(
                r,
                network_tip,
                stage_enabled(Stage::Validator),
                batch,
              ),
              None => StageProgress {
                stage: r.stage.clone(),
                stage_type: "Other".to_owned(),
                display_name: "Unknown".to_owned(),
                indexed_height: r.indexed_height,
                tip_height: network_tip.unwrap_or(0) as i32,
                updated_at: r.updated_at,
                gate: None,
                enabled: false,
                error: None,
                batch_size: None,
                last_batch_ms: None,
                batch_history: vec![],
              },
            }
          })
          .collect()
      }
      Err(e) => {
        state.pg_backoff = if state.pg_backoff == 0 {
          1
        } else {
          (state.pg_backoff * 2).min(60)
        };
        state.pg_skip = state.pg_backoff / 5;
        tracing::warn!(
          pg_backoff = state.pg_backoff,
          "heartbeat: failed to query sync_state: {e}"
        );
        vec![]
      }
    }
  };

  // Resolve tip_height for gated stages
  {
    let heights: std::collections::HashMap<String, i32> = stages
      .iter()
      .map(|s| (s.stage.clone(), s.indexed_height))
      .collect();
    for s in &mut stages {
      if let Some(ref gate) = s.gate {
        if let Some(&gate_h) = heights.get(gate.as_str()) {
          if gate_h >= 0 {
            s.tip_height = gate_h;
          }
        }
      }
    }
  }

  // Add disabled indices not in sync_state
  let all_stages = [
    Stage::Metrics,
    Stage::Validator,
    Stage::Script,
    Stage::Spent,
  ];
  for stage in all_stages {
    if !stages.iter().any(|s| s.stage == stage.name()) {
      stages.push(match stage {
        Stage::Metrics => crate::index::metrics::ipc::disabled_progress(stage_enabled(stage)),
        Stage::Validator => crate::index::validator::ipc::disabled_progress(stage_enabled(stage)),
        Stage::Script => crate::index::script::ipc::disabled_progress(stage_enabled(stage)),
        Stage::Spent => crate::index::spent::ipc::disabled_progress(stage_enabled(stage)),
        _ => {
          let (stage_type, display_name, _) = stage.meta();
          StageProgress {
            stage: stage.name().to_owned(),
            stage_type: stage_type.to_owned(),
            display_name: display_name.to_owned(),
            indexed_height: -1,
            tip_height: 0,
            updated_at: 0,
            gate: stage
              .gate_stage()
              .map(|gate_stage| gate_stage.name().to_owned()),
            enabled: stage_enabled(stage),
            error: None,
            batch_size: None,
            last_batch_ms: None,
            batch_history: vec![],
          }
        }
      });
    }
  }

  // Attach cached schema errors
  if !schema_errors.is_empty() {
    for s in &mut stages {
      if s.indexed_height < 0 {
        continue;
      }
      let owned = Stage::from_name(&s.stage).map_or(&[][..], Stage::tables);
      let problems: Vec<&str> = schema_errors
        .iter()
        .filter(|p| owned.iter().any(|t| p.starts_with(t)))
        .map(|p| p.as_str())
        .collect();
      if !problems.is_empty() {
        s.error = Some(format!("Schema Mismatch: {}", problems.join(", ")));
      }
    }
  }

  let ingestion_tip = db::get_tip_height(ctx.db);
  let ingestion_tip_hash = db::get_tip_hash(ctx.db).map(|mut h| {
    h.reverse();
    hex::encode(h)
  });
  let chain_progress_diag = crate::sql::progress::chain_progress(ctx.admin_pool)
    .await
    .map(|progress| hyphend::wire::msg::ChainProgressDiag {
      canonical_tip_height: progress.canonical_tip_height,
      projection_tip_height: progress.projection_tip_height,
      undo_tip_height: progress.undo_tip_height,
      recovery_state: progress.recovery_state,
      recovery_height: progress.recovery_height,
    })
    .unwrap_or_default();
  let now_epoch = SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs();
  let ingestion_diag = crate::ingest::build_ingestion_diag(
    &ingest,
    ctx
      .config_snap
      .performance
      .max_ingestion_lead_blocks
      .unwrap_or(crate::ingest::DEFAULT_MAX_INGEST_LEAD_BLOCKS),
    now_epoch,
  );
  let index_scheduler_diag = ctx.block_worker.scheduler_diag();
  let core_pipeline_diag =
    derive_core_pipeline_diag(&ingestion_diag, &index_scheduler_diag, now_epoch);

  let rocksdb_diag = db::rocksdb_diag(ctx.db, ctx.db_path);
  let secondary_store_diags: Vec<SecondaryStoreDiag> = ctx
    .secondary_stores
    .diagnostics()
    .unwrap_or_default()
    .into_iter()
    .map(|diag| SecondaryStoreDiag {
      name: diag.name,
      required: diag.required,
      tip_height: diag.tip_height,
      recovery_state: diag.recovery_state,
      recovery_height: diag.recovery_height,
      live_sst_bytes: diag.live_sst_bytes,
      estimate_num_keys: diag.estimate_num_keys,
      actual_disk_bytes: diag.actual_disk_bytes,
      cf_sizes: diag.cf_sizes,
    })
    .collect();

  // Postgres pool stats
  let mut postgres_diag = ctx.pg_diag_base.clone();
  postgres_diag.write_pool_available = Some(ctx.write_pool.num_idle());
  postgres_diag.query_pool_available = Some(ctx.query_pool.num_idle());
  postgres_diag.admin_pool_available = Some(ctx.admin_pool.num_idle());
  let postgres_ok =
    ctx.write_pool.size() > 0 && ctx.query_pool.size() > 0 && ctx.admin_pool.size() > 0;

  refresh_pg_size_diag(ctx.admin_pool, state).await;
  let (pg_db_bytes, pg_table_sizes, pg_wal_bytes) = (
    state.pg_sizes.db_bytes,
    state.pg_sizes.table_sizes.clone(),
    state.pg_sizes.wal_bytes,
  );

  let rest_stats = ctx.source.rest_stats().unwrap_or_default();
  let core_info = ctx.source.core_info().unwrap_or_default();
  let rpc_diag = ctx.rpc.map(|rpc| rpc.diag()).unwrap_or_else(|| RpcDiag {
    enabled: ctx.config_snap.sources.rpc_enabled,
    ..RpcDiag::default()
  });
  let zmq_diag = ctx.zmq.map(|zmq| zmq.diag()).unwrap_or_else(|| ZmqDiag {
    enabled: ctx.config_snap.sources.zmq_enabled,
    endpoints: ctx.config_snap.sources.zmq_connect.clone(),
    subscriptions: ctx.config_snap.sources.zmq_subscriptions.clone(),
    ..ZmqDiag::default()
  });

  DaemonSnapshot {
    data_source,
    network_tip,
    ingestion_tip,
    ingestion_tip_hash,
    ingestion_diag,
    index_scheduler_diag,
    core_pipeline_diag,
    blocks_ingested_total: ingest.blocks_ingested_total,
    stages,
    chain_progress_diag,
    config: ctx.config_snap.clone(),
    uptime_secs: ctx.start_time.elapsed().as_secs(),
    version: env!("CARGO_PKG_VERSION").to_string(),
    rest_stats,
    server_stats: ctx.api_metrics.snapshot(),
    rocksdb_diag,
    secondary_store_diags,
    postgres_diag,
    core_info,
    postgres_ok,
    core_rest_ok: ping_ok,
    rpc_diag,
    zmq_diag,
    pausing_stages: ctx.pausing_rx.borrow().iter().cloned().collect(),
    suspended_stages: ctx.suspend_rx.borrow().iter().cloned().collect(),
    pg_db_bytes,
    pg_table_sizes,
    pg_wal_bytes,
  }
}

fn derive_core_pipeline_diag(
  ingestion_diag: &hyphend::wire::msg::IngestionDiag,
  scheduler_diag: &hyphend::wire::msg::IndexSchedulerDiag,
  now_epoch: u64,
) -> CorePipelineDiag {
  let last_ingest_success_epoch = ingestion_diag.last_progress_epoch;
  let last_forward_success_epoch = scheduler_diag.forward.last_success_epoch;
  let last_forward_attempt_epoch = scheduler_diag.forward.last_attempt_epoch;
  let last_forward_heartbeat_epoch = scheduler_diag.forward.last_heartbeat_epoch;
  let last_forward_progress_epoch = scheduler_diag.forward.last_progress_epoch;
  let scheduler_snapshot_epoch = scheduler_diag.snapshot_epoch;
  let last_turn_age_secs = scheduler_diag.last_turn_age_ms.map(|ms| ms / 1000);
  let forward_success_age_secs =
    last_forward_success_epoch.map(|epoch| now_epoch.saturating_sub(epoch));
  let forward_heartbeat_age_secs =
    last_forward_heartbeat_epoch.map(|epoch| now_epoch.saturating_sub(epoch));
  let forward_progress_age_secs =
    last_forward_progress_epoch.map(|epoch| now_epoch.saturating_sub(epoch));
  let scheduler_idle = scheduler_diag.active_lane.is_none()
    && scheduler_diag.active_lanes.is_empty()
    && scheduler_diag.forward.phase.is_none();
  let stale_scheduler_state = scheduler_diag
    .active_lanes
    .contains(&IndexSchedulerLane::Forward)
    && scheduler_diag.forward.phase.is_none()
    && last_turn_age_secs.is_some_and(|secs| secs >= SCHEDULER_STALE_SECS)
    && forward_success_age_secs.is_some_and(|secs| secs >= FORWARD_STALE_SECS);
  let stale_forward_heartbeat = scheduler_diag.forward.phase.is_some()
    && forward_heartbeat_age_secs.is_some_and(|secs| secs >= FORWARD_STALE_SECS);
  let forward_is_slow = scheduler_diag.forward.progress_state
    == hyphend::wire::msg::ForwardProgressState::Slow
    || (scheduler_diag.forward.phase.is_some()
      && forward_progress_age_secs.is_some_and(|secs| secs >= FORWARD_STALE_SECS)
      && !stale_forward_heartbeat);
  let forward_is_stale = scheduler_diag.forward.progress_state
    == hyphend::wire::msg::ForwardProgressState::Stale
    || stale_forward_heartbeat;
  let health = if ingestion_diag.paused {
    CorePipelineHealth::Degraded
  } else if ingestion_diag.stalled || scheduler_diag.forward.stalled || forward_is_stale {
    CorePipelineHealth::Stalled
  } else if ingestion_diag.throttled {
    if forward_is_slow {
      CorePipelineHealth::Degraded
    } else if forward_success_age_secs.is_some_and(|secs| secs <= FORWARD_STALE_SECS) {
      CorePipelineHealth::Backpressured
    } else if stale_scheduler_state {
      CorePipelineHealth::Stalled
    } else {
      CorePipelineHealth::Degraded
    }
  } else if scheduler_idle {
    CorePipelineHealth::Healthy
  } else if stale_scheduler_state
    || last_turn_age_secs.is_some_and(|secs| secs >= SCHEDULER_STALE_SECS)
  {
    CorePipelineHealth::Degraded
  } else {
    CorePipelineHealth::Healthy
  };

  CorePipelineDiag {
    health,
    paused: ingestion_diag.paused,
    backpressured: ingestion_diag.throttled,
    stale_scheduler_state,
    last_ingest_success_epoch,
    last_forward_success_epoch,
    last_forward_attempt_epoch,
    scheduler_snapshot_epoch,
  }
}

async fn refresh_pg_size_diag(pool: &crate::sql::SqlPool, state: &mut HeartbeatState) {
  let now = Instant::now();

  if should_refresh(
    state.pg_db_refreshed_at,
    Duration::from_secs(PG_DB_SIZE_REFRESH_SECS),
    now,
  ) {
    if let Some(db_bytes) = crate::sql::snapshot::database_size_diag(pool).await {
      state.pg_sizes.db_bytes = Some(db_bytes);
    }
    state.pg_db_refreshed_at = Some(now);
  }

  if should_refresh(
    state.pg_table_refreshed_at,
    Duration::from_secs(PG_TABLE_SIZE_REFRESH_SECS),
    now,
  ) {
    let table_sizes = crate::sql::snapshot::table_size_diag(pool).await;
    if !table_sizes.is_empty() || state.pg_sizes.table_sizes.is_empty() {
      state.pg_sizes.table_sizes = table_sizes;
    }
    state.pg_table_refreshed_at = Some(now);
  }

  if should_refresh(
    state.pg_wal_refreshed_at,
    Duration::from_secs(PG_WAL_SIZE_REFRESH_SECS),
    now,
  ) {
    if let Some(wal_bytes) = crate::sql::snapshot::wal_size_diag(pool).await {
      state.pg_sizes.wal_bytes = Some(wal_bytes);
    }
    state.pg_wal_refreshed_at = Some(now);
  }
}

fn should_refresh(last_refresh: Option<Instant>, interval: Duration, now: Instant) -> bool {
  last_refresh.is_none_or(|instant| now.duration_since(instant) >= interval)
}

#[cfg(test)]
mod tests {
  use super::derive_core_pipeline_diag;
  use hyphend::wire::msg::{
    CorePipelineHealth, ForwardLaneDiag, ForwardProgressState, IndexSchedulerDiag,
    IndexSchedulerLane, IngestionActivity, IngestionDiag, SchedulerLoopPhase,
  };

  #[test]
  fn core_pipeline_diag_marks_recent_throttle_as_backpressured() {
    let ingestion = IngestionDiag {
      activity: IngestionActivity::Sleeping,
      throttled: true,
      last_progress_epoch: Some(100),
      ..IngestionDiag::default()
    };
    let scheduler = IndexSchedulerDiag {
      active_lanes: vec![IndexSchedulerLane::Forward],
      last_turn_age_ms: Some(5_000),
      snapshot_epoch: Some(110),
      forward: ForwardLaneDiag {
        last_success_height: Some(42),
        last_success_epoch: Some(105),
        last_attempt_epoch: Some(108),
        ..Default::default()
      },
      ..Default::default()
    };

    let diag = derive_core_pipeline_diag(&ingestion, &scheduler, 110);

    assert_eq!(diag.health, CorePipelineHealth::Backpressured);
    assert!(!diag.stale_scheduler_state);
  }

  #[test]
  fn core_pipeline_diag_marks_stale_forward_under_throttle_as_stalled() {
    let ingestion = IngestionDiag {
      activity: IngestionActivity::Sleeping,
      throttled: true,
      last_progress_epoch: Some(100),
      ..IngestionDiag::default()
    };
    let scheduler = IndexSchedulerDiag {
      active_lanes: vec![IndexSchedulerLane::Forward],
      loop_phase: SchedulerLoopPhase::RunLane,
      last_turn_age_ms: Some(20_000),
      snapshot_epoch: Some(140),
      forward: ForwardLaneDiag {
        last_success_height: Some(42),
        last_success_epoch: Some(100),
        last_attempt_epoch: Some(101),
        ..Default::default()
      },
      ..Default::default()
    };

    let diag = derive_core_pipeline_diag(&ingestion, &scheduler, 140);

    assert_eq!(diag.health, CorePipelineHealth::Stalled);
    assert!(diag.stale_scheduler_state);
  }

  #[test]
  fn core_pipeline_diag_keeps_idle_synced_scheduler_healthy() {
    let ingestion = IngestionDiag {
      activity: IngestionActivity::Sleeping,
      last_progress_epoch: Some(100),
      ..IngestionDiag::default()
    };
    let scheduler = IndexSchedulerDiag {
      last_turn_age_ms: Some(120_000),
      snapshot_epoch: Some(220),
      forward: ForwardLaneDiag {
        last_success_height: Some(42),
        last_success_epoch: Some(100),
        last_attempt_epoch: Some(100),
        ..Default::default()
      },
      ..Default::default()
    };

    let diag = derive_core_pipeline_diag(&ingestion, &scheduler, 220);

    assert_eq!(diag.health, CorePipelineHealth::Healthy);
    assert!(!diag.stale_scheduler_state);
  }

  #[test]
  fn core_pipeline_diag_marks_stale_forward_heartbeat_as_stalled() {
    let ingestion = IngestionDiag {
      activity: IngestionActivity::Sleeping,
      throttled: true,
      last_progress_epoch: Some(100),
      ..IngestionDiag::default()
    };
    let scheduler = IndexSchedulerDiag {
      active_lanes: vec![IndexSchedulerLane::Forward],
      loop_phase: SchedulerLoopPhase::RunLane,
      last_turn_age_ms: Some(5_000),
      snapshot_epoch: Some(140),
      forward: ForwardLaneDiag {
        height: Some(42),
        phase: Some(hyphend::wire::msg::ForwardLanePhase::ApplyStagePlan),
        stage: Some("spent".to_owned()),
        substep: Some("spent_address_stats".to_owned()),
        progress_state: ForwardProgressState::Stale,
        last_success_height: Some(41),
        last_success_epoch: Some(136),
        last_attempt_epoch: Some(138),
        last_heartbeat_epoch: Some(100),
        last_progress_epoch: Some(100),
        ..Default::default()
      },
      ..Default::default()
    };

    let diag = derive_core_pipeline_diag(&ingestion, &scheduler, 140);

    assert_eq!(diag.health, CorePipelineHealth::Stalled);
    assert!(!diag.stale_scheduler_state);
  }

  #[test]
  fn core_pipeline_diag_keeps_recent_forward_heartbeat_backpressured() {
    let ingestion = IngestionDiag {
      activity: IngestionActivity::Sleeping,
      throttled: true,
      last_progress_epoch: Some(100),
      ..IngestionDiag::default()
    };
    let scheduler = IndexSchedulerDiag {
      active_lanes: vec![IndexSchedulerLane::Forward],
      loop_phase: SchedulerLoopPhase::RunLane,
      last_turn_age_ms: Some(5_000),
      snapshot_epoch: Some(140),
      forward: ForwardLaneDiag {
        height: Some(42),
        phase: Some(hyphend::wire::msg::ForwardLanePhase::ApplyStagePlan),
        stage: Some("spent".to_owned()),
        substep: Some("spent_address_stats".to_owned()),
        progress_state: ForwardProgressState::Active,
        last_success_height: Some(41),
        last_success_epoch: Some(136),
        last_attempt_epoch: Some(138),
        last_heartbeat_epoch: Some(136),
        last_progress_epoch: Some(136),
        ..Default::default()
      },
      ..Default::default()
    };

    let diag = derive_core_pipeline_diag(&ingestion, &scheduler, 140);

    assert_eq!(diag.health, CorePipelineHealth::Backpressured);
    assert!(!diag.stale_scheduler_state);
  }

  #[test]
  fn core_pipeline_diag_marks_slow_forward_work_as_degraded() {
    let ingestion = IngestionDiag {
      activity: IngestionActivity::Sleeping,
      throttled: true,
      last_progress_epoch: Some(100),
      ..IngestionDiag::default()
    };
    let scheduler = IndexSchedulerDiag {
      active_lanes: vec![IndexSchedulerLane::Forward],
      loop_phase: SchedulerLoopPhase::RunLane,
      last_turn_age_ms: Some(5_000),
      snapshot_epoch: Some(140),
      forward: ForwardLaneDiag {
        height: Some(42),
        phase: Some(hyphend::wire::msg::ForwardLanePhase::ApplyStagePlan),
        stage: Some("spent".to_owned()),
        substep: Some("chunked".to_owned()),
        progress_state: ForwardProgressState::Slow,
        last_success_height: Some(41),
        last_success_epoch: Some(136),
        last_attempt_epoch: Some(138),
        last_heartbeat_epoch: Some(139),
        last_progress_epoch: Some(100),
        ..Default::default()
      },
      ..Default::default()
    };

    let diag = derive_core_pipeline_diag(&ingestion, &scheduler, 140);

    assert_eq!(diag.health, CorePipelineHealth::Degraded);
    assert!(!diag.stale_scheduler_state);
  }
}
