//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Heartbeat snapshot builder for IPC broadcast.
//!
//! Every storage fact comes from the `hyphen-db` facade: the canonical tip,
//! relation sizes, server diagnostics, pool utilization, and undo coverage.
//! Ingest-loop state comes from the shared [`IngestStatus`], and the engine's
//! per-block progress from the `HyDbProgress` watch channel.

use std::time::Instant;
use std::time::{SystemTime, UNIX_EPOCH};

use hyphen_db::{HyDbProgress, HyDbStage, HyphenDb};

use crate::api_metrics::ApiServerMetrics;
use crate::index::{IndexStageFlags, Stage};
use crate::ingest::IngestStatus;
use crate::init::Options;
use crate::policy::api::API_DATA_SOURCE_TIMEOUT_SECS;
use crate::sources::{RestSource, RpcSource, ZmqSource};
use hyphend::wire::msg::{
  ChainProgressDiag, ConfigSnapshot, CorePipelineDiag, CorePipelineHealth, DaemonSnapshot,
  ForwardLaneDiag, ForwardLanePhase, ForwardProgressState, IndexSchedulerDiag, IndexSchedulerLane,
  IngestionActivity, IngestionDiag, IngestionStateName, PostgresDiag, RestSourceState, RpcDiag,
  SchedulerLoopPhase, StageProgress, StorageDiag, ZmqDiag,
};

const PG_SIZE_REFRESH_SECS: u64 = 30;

/// Read-only shared state for heartbeat snapshot building.
pub struct HeartbeatCtx<'a> {
  pub options: &'a Options,
  pub hydb: &'a HyphenDb,
  pub source: &'a dyn RestSource,
  pub rpc: Option<&'a dyn RpcSource>,
  pub zmq: Option<&'a dyn ZmqSource>,
  pub ingest_status: &'a IngestStatus,
  pub progress: HyDbProgress,
  pub config_snap: &'a ConfigSnapshot,
  pub pg_diag_base: &'a PostgresDiag,
  pub api_metrics: &'a ApiServerMetrics,
  pub start_time: Instant,
}

impl std::fmt::Debug for HeartbeatCtx<'_> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("HeartbeatCtx").finish_non_exhaustive()
  }
}

/// Mutable state that persists across heartbeat ticks (size probes are
/// refreshed on an interval, not every tick).
#[derive(Debug, Default)]
pub struct HeartbeatState {
  pub storage: StorageDiag,
  pub table_sizes: Vec<(String, u64)>,
  pub refreshed_at: Option<Instant>,
  pub health_notes: Vec<String>,
  /// Server-wide memory settings, refreshed with the size probes. Held here
  /// rather than on `StorageDiag` because the console renders them in the
  /// Postgres panel.
  pub shared_buffers: Option<String>,
  pub work_mem: Option<String>,
  pub effective_cache_size: Option<String>,
}

impl HeartbeatState {
  pub fn new() -> Self {
    Self::default()
  }
}

fn unix_now() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

fn stage_meta(stage: Stage) -> (&'static str, &'static str) {
  match stage {
    Stage::Block => ("Core", "Blocks"),
    Stage::Transaction => ("Core", "Transactions"),
    Stage::Script => ("Address", "Scripts"),
    Stage::Spent => ("Address", "Spent Outputs"),
    Stage::Metrics => ("Analytics", "Chain Metrics"),
    Stage::Validator => ("Validators", "Validator Registry"),
  }
}

/// Build one heartbeat snapshot.
pub async fn build_snapshot(ctx: &HeartbeatCtx<'_>, state: &mut HeartbeatState) -> DaemonSnapshot {
  let ping_ok = tokio::time::timeout(
    std::time::Duration::from_secs(API_DATA_SOURCE_TIMEOUT_SECS),
    ctx.source.ping(),
  )
  .await
  .is_ok();
  let ingesting = ctx.ingest_status.blocks_ingested_total() > 0;
  let data_source = if ping_ok || ingesting {
    RestSourceState::Connected
  } else {
    RestSourceState::Disconnected
  };

  let network_tip = ctx.ingest_status.network_tip();
  let tip = ctx.hydb.status().tip().await.ok().flatten();
  let ingestion_tip = tip.as_ref().map(|t| t.height.0);
  let ingestion_tip_hash = tip.as_ref().map(|t| format!("{:?}", t.hash));

  // Every stage is lock-step at the canonical tip: derivation is atomic per
  // block in the storage engine.
  let flags = IndexStageFlags {
    address: ctx.options.indices.address,
    metrics: ctx.options.indices.metrics,
    validator: ctx.options.indices.validator,
  };
  let indexed_height = ingestion_tip.map_or(-1i32, |h| h.min(i32::MAX as u32) as i32);
  let tip_height = network_tip.map_or(0i32, |h| h.min(i32::MAX as u32) as i32);
  let now_epoch = unix_now();
  let stage_error = (!state.health_notes.is_empty()).then(|| state.health_notes.join("; "));
  let stages: Vec<StageProgress> = [
    Stage::Block,
    Stage::Transaction,
    Stage::Script,
    Stage::Spent,
    Stage::Metrics,
    Stage::Validator,
  ]
  .into_iter()
  .map(|stage| {
    let (stage_type, display_name) = stage_meta(stage);
    let enabled = stage.enabled_with_flags(flags);
    StageProgress {
      stage: stage.name().to_owned(),
      stage_type: stage_type.to_owned(),
      display_name: display_name.to_owned(),
      indexed_height: if enabled { indexed_height } else { -1 },
      tip_height,
      updated_at: ctx
        .ingest_status
        .last_progress_epoch()
        .map_or(0, |epoch| epoch.min(i64::MAX as u64) as i64),
      gate: None,
      enabled,
      error: if stage == Stage::Block {
        stage_error.clone()
      } else {
        None
      },
      batch_size: None,
      last_batch_ms: None,
      batch_history: Vec::new(),
    }
  })
  .collect();

  refresh_storage(ctx, state).await;

  let sync_progress = ctx.ingest_status.sync_progress();
  let undo_span = ctx.hydb.status().undo_span().await.ok().flatten();
  let chain_progress_diag = ChainProgressDiag {
    canonical_tip_height: indexed_height,
    projection_tip_height: indexed_height,
    undo_tip_height: undo_span.map_or(-1, |(_, hi)| hi.0.min(i32::MAX as u32) as i32),
    recovery_state: "clean".to_owned(),
    recovery_height: None,
    block_progress_pct: sync_progress.block_pct,
    transactions_indexed: sync_progress.transactions_indexed,
    transactions_estimated_total: sync_progress.transactions_estimated_total,
    tx_progress_pct: sync_progress.tx_pct,
    txs_per_block_recent: sync_progress.txs_per_block_recent,
  };

  let ingestion_diag = IngestionDiag {
    activity: ctx.ingest_status.activity(),
    state: ctx.ingest_status.state_name(),
    paused: ctx.ingest_status.operator_pause_requested(),
    throttled: false,
    sync_mode: ctx.ingest_status.sync_mode().to_owned(),
    last_polled_tip: network_tip,
    lead_blocks: None,
    max_lead_blocks: 0,
    last_progress_epoch: ctx.ingest_status.last_progress_epoch(),
    last_throttle_clear_epoch: None,
    last_fetch_start_epoch: None,
    current_batch_start: None,
    current_batch_end: None,
    last_state_transition_epoch: None,
    retry_attempt: ctx.ingest_status.retry_attempt(),
    backoff_until_epoch: ctx.ingest_status.backoff_until_epoch(),
    stalled: ctx.ingest_status.stalled(),
  };

  let index_scheduler_diag = scheduler_diag(&ctx.progress, &ingestion_diag, now_epoch);
  let core_pipeline_diag = pipeline_diag(&ingestion_diag, now_epoch);

  // Pool utilization straight from the facade.
  let pool_stats = ctx.hydb.pool_stats();
  let mut storage_diag = state.storage.clone();
  storage_diag.undo_span = undo_span.map(|(lo, hi)| (lo.0, hi.0));
  storage_diag.pool_stats = pool_stats
    .iter()
    .map(|p| (p.name.clone(), p.size, p.idle as u64))
    .collect();

  let mut postgres_diag = ctx.pg_diag_base.clone();
  for pool in &pool_stats {
    let idle = Some(pool.idle);
    match pool.name.as_str() {
      "write" => postgres_diag.write_pool_available = idle,
      "query" => postgres_diag.query_pool_available = idle,
      "heavy_query" => postgres_diag.heavy_query_pool_available = idle,
      "admin" => postgres_diag.admin_pool_available = idle,
      _ => {}
    }
  }
  postgres_diag.version = Some(state.storage.server_version.clone().unwrap_or_default())
    .filter(|v| !v.is_empty())
    .or(postgres_diag.version);
  postgres_diag.max_connections = state
    .storage
    .max_connections
    .and_then(|v| i32::try_from(v).ok())
    .or(postgres_diag.max_connections);
  postgres_diag.shared_buffers = state
    .shared_buffers
    .clone()
    .or(postgres_diag.shared_buffers);
  postgres_diag.work_mem = state.work_mem.clone().or(postgres_diag.work_mem);
  postgres_diag.effective_cache_size = state
    .effective_cache_size
    .clone()
    .or(postgres_diag.effective_cache_size);
  let postgres_ok = pool_stats.iter().any(|p| p.size > 0);

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
    blocks_ingested_total: ctx.ingest_status.blocks_ingested_total(),
    stages,
    chain_progress_diag,
    config: ctx.config_snap.clone(),
    uptime_secs: ctx.start_time.elapsed().as_secs(),
    version: env!("CARGO_PKG_VERSION").to_string(),
    rest_stats,
    server_stats: ctx.api_metrics.snapshot(),
    storage_diag,
    postgres_diag,
    core_info,
    postgres_ok,
    core_rest_ok: ping_ok,
    rpc_diag,
    zmq_diag,
    pausing_stages: Vec::new(),
    suspended_stages: Vec::new(),
    pg_db_bytes: state.storage.database_bytes,
    pg_table_sizes: state.table_sizes.clone(),
    pg_wal_bytes: state.storage.wal_bytes,
  }
}

/// Map the engine's per-block progress feed onto the console's forward-lane
/// view: the engine's pipeline stages are the lane's phases.
fn scheduler_diag(
  progress: &HyDbProgress,
  ingestion: &IngestionDiag,
  now_epoch: u64,
) -> IndexSchedulerDiag {
  let (phase, busy) = match progress.stage {
    HyDbStage::Absorb => (ForwardLanePhase::FetchBlock, true),
    HyDbStage::Canonicalize => (ForwardLanePhase::PlanConnect, true),
    HyDbStage::Derive => (ForwardLanePhase::PlanConnect, true),
    HyDbStage::ResolveSpends => (ForwardLanePhase::PlanConnect, true),
    HyDbStage::Accumulate => (ForwardLanePhase::ApplyStagePlan, true),
    HyDbStage::Idle => (ForwardLanePhase::Idle, false),
  };
  let committing = ingestion.state == IngestionStateName::Committing;
  let forward = ForwardLaneDiag {
    run_id: None,
    height: progress.height.map(|h| h.0),
    phase: (busy || committing).then_some(phase),
    stage: None,
    substep: None,
    phase_elapsed_ms: None,
    last_success_height: progress.height.map(|h| h.0),
    last_success_epoch: ingestion.last_progress_epoch,
    last_attempt_epoch: ingestion.last_progress_epoch,
    last_heartbeat_epoch: Some(now_epoch),
    chunk_index: None,
    chunk_total: None,
    last_progress_epoch: ingestion.last_progress_epoch,
    progress_state: if ingestion.stalled {
      ForwardProgressState::Stale
    } else if busy || committing {
      ForwardProgressState::Active
    } else {
      ForwardProgressState::Idle
    },
    recovery_attempts: 0,
    stalled: ingestion.stalled,
  };
  let loop_phase = match ingestion.state {
    IngestionStateName::PollingTip => SchedulerLoopPhase::FetchTip,
    IngestionStateName::Committing | IngestionStateName::Fetching => SchedulerLoopPhase::RunLane,
    _ => SchedulerLoopPhase::Idle,
  };
  let active = busy || committing;
  IndexSchedulerDiag {
    contention: false,
    active_lane: active.then_some(IndexSchedulerLane::Forward),
    active_lanes: if active {
      vec![IndexSchedulerLane::Forward]
    } else {
      Vec::new()
    },
    lane_turn_counts: Vec::new(),
    lane_last_turn_ms: Vec::new(),
    forward,
    loop_phase,
    snapshot_epoch: Some(now_epoch),
    loop_phase_elapsed_ms: None,
    last_turn_age_ms: ingestion
      .last_progress_epoch
      .map(|epoch| now_epoch.saturating_sub(epoch).saturating_mul(1000)),
  }
}

fn pipeline_diag(ingestion: &IngestionDiag, _now_epoch: u64) -> CorePipelineDiag {
  let health = if ingestion.stalled || ingestion.paused {
    // A pause is a stall by intent: nothing will advance until a human acts,
    // so it must not read as merely degraded.
    CorePipelineHealth::Stalled
  } else if ingestion.state == IngestionStateName::Backoff {
    CorePipelineHealth::Degraded
  } else if ingestion.activity == IngestionActivity::Active {
    CorePipelineHealth::Healthy
  } else {
    CorePipelineHealth::Healthy
  };
  CorePipelineDiag {
    health,
    paused: ingestion.paused,
    backpressured: false,
    stale_scheduler_state: false,
    last_ingest_success_epoch: ingestion.last_progress_epoch,
    last_forward_success_epoch: ingestion.last_progress_epoch,
    last_forward_attempt_epoch: ingestion.last_progress_epoch,
    scheduler_snapshot_epoch: Some(_now_epoch),
  }
}

/// Refresh the (relatively expensive) storage probes on an interval.
async fn refresh_storage(ctx: &HeartbeatCtx<'_>, state: &mut HeartbeatState) {
  let now = Instant::now();
  let due = state
    .refreshed_at
    .is_none_or(|at| now.duration_since(at).as_secs() >= PG_SIZE_REFRESH_SECS);
  if !due {
    return;
  }
  state.refreshed_at = Some(now);

  match ctx.hydb.status().storage_diag().await {
    Ok(diag) => {
      state.storage.server_version = Some(diag.server_version);
      state.storage.database_bytes = u64::try_from(diag.database_bytes).ok();
      state.storage.wal_bytes = u64::try_from(diag.wal_bytes).ok();
      state.storage.connections = Some(diag.connections);
      state.storage.max_connections = Some(diag.max_connections);
      state.storage.autovacuum_running = Some(diag.autovacuum_running);
      state.shared_buffers = Some(diag.shared_buffers);
      state.work_mem = Some(diag.work_mem);
      state.effective_cache_size = Some(diag.effective_cache_size);
    }
    Err(e) => tracing::debug!(error = %e, "storage diag probe failed"),
  }
  match ctx.hydb.status().sizes().await {
    Ok(sizes) => {
      state.table_sizes = sizes
        .into_iter()
        .filter_map(|s| u64::try_from(s.bytes).ok().map(|b| (s.relation, b)))
        .collect();
    }
    Err(e) => tracing::debug!(error = %e, "relation size probe failed"),
  }
  match ctx.hydb.status().health().await {
    Ok(health) => state.health_notes = health.notes,
    Err(e) => state.health_notes = vec![format!("health probe failed: {e}")],
  }
}
