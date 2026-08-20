//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! IPC message types exchanged between daemon and console.

use serde::{Deserialize, Serialize};

/// Connectivity state of the upstream REST source used for ingestion.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum RestSourceState {
  /// Upstream REST is unavailable and no ingestion is happening.
  Disconnected,
  /// Upstream REST is reachable or ingestion is actively progressing.
  Connected,
}

/// One auto-tuner decision emitted in heartbeat history.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchAdjustment {
  /// Duration of the measured batch in milliseconds.
  pub elapsed_ms: u64,
  /// Previous batch size before the adjustment.
  pub prev: u32,
  /// New batch size chosen after the adjustment.
  pub next: u32,
}

/// Progress snapshot for one indexing stage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StageProgress {
  /// Stable stage identifier.
  pub stage: String,
  /// Stage family label used for grouping in the console.
  pub stage_type: String,
  /// Human-readable stage name.
  pub display_name: String,
  /// Indexed height currently committed for the stage.
  pub indexed_height: i32,
  /// Best known upstream tip height for the stage.
  pub tip_height: i32,
  /// Unix timestamp when the stage last advanced.
  pub updated_at: i64,
  #[serde(default)]
  /// Optional dependency gate that must be satisfied before this stage runs.
  pub gate: Option<String>,
  #[serde(default)]
  /// Whether this stage is enabled in the resolved configuration.
  pub enabled: bool,
  #[serde(default)]
  /// Last reported stage error, if any.
  pub error: Option<String>,
  /// Current auto-tuner batch size for this stage.
  #[serde(default)]
  pub batch_size: Option<u32>,
  /// Duration of the last committed batch in milliseconds.
  #[serde(default)]
  pub last_batch_ms: Option<u64>,
  /// Last 3 batch-size adjustments: (elapsed_ms, prev_size, new_size).
  #[serde(default)]
  pub batch_history: Vec<BatchAdjustment>,
}

/// Durable PostgreSQL chain progress and recovery state.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ChainProgressDiag {
  /// Canonical PostgreSQL tip height.
  pub canonical_tip_height: i32,
  /// Projection tip height mirrored from PostgreSQL progress.
  pub projection_tip_height: i32,
  /// Highest height with undo coverage.
  pub undo_tip_height: i32,
  /// Durable recovery state label.
  pub recovery_state: String,
  /// Optional recovery progress height.
  pub recovery_height: Option<i32>,
  /// Percentage of blocks connected, of the node's tip. Exact, and a poor
  /// proxy for work done — see `tx_progress_pct`.
  #[serde(default)]
  pub block_progress_pct: Option<f64>,
  /// Transactions in the database: the startup count plus this run's absorbs.
  #[serde(default)]
  pub transactions_indexed: Option<u64>,
  /// Projected transactions at the node's tip, at the recently observed
  /// density. An estimate: the node is never asked for a chain-wide count.
  #[serde(default)]
  pub transactions_estimated_total: Option<u64>,
  /// Percentage of that projected total which is indexed. This is the figure
  /// that tracks remaining work; block percentage overstates progress because
  /// chain density rises roughly sixfold toward the tip.
  #[serde(default)]
  pub tx_progress_pct: Option<f64>,
  /// Smoothed transactions per block over the blocks absorbed this run.
  #[serde(default)]
  pub txs_per_block_recent: Option<f64>,
}

/// Aggregate metrics for the daemon's REST server.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ApiServerStats {
  /// Total handled requests since process start.
  pub total_requests: u64,
  /// Total requests that completed as errors since process start.
  pub total_errors: u64,
  /// Rolling average request latency in milliseconds when available.
  pub avg_latency_ms: Option<f64>,
}

/// Aggregate metrics for outbound REST calls made by the daemon.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RestClientStats {
  /// Total outbound REST requests since process start.
  pub total_requests: u64,
  /// Total outbound REST requests that failed.
  pub total_errors: u64,
  /// Rolling average outbound latency in milliseconds when available.
  pub avg_latency_ms: Option<f64>,
  /// Unix timestamp of the last outbound request.
  pub last_request_epoch: Option<u64>,
  /// Error counts grouped by normalized error label.
  pub error_counts: Vec<(String, u64)>,
}

/// Aggregate metrics and connection state for outbound JSON-RPC calls.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct RpcDiag {
  /// Whether JSON-RPC support is configured.
  pub enabled: bool,
  /// Whether the most recent health check or call succeeded.
  pub connected: bool,
  /// Resolved authentication mode, when configured.
  pub auth_type: Option<String>,
  /// Total JSON-RPC calls since process start.
  pub total_calls: u64,
  /// Total JSON-RPC calls that failed.
  pub total_errors: u64,
  /// Unix timestamp of the last successful JSON-RPC call.
  pub last_success_epoch: Option<u64>,
  /// Per-method call counts.
  pub method_counts: Vec<(String, u64)>,
  /// Error counts grouped by normalized error label.
  pub error_counts: Vec<(String, u64)>,
}

/// Aggregate metrics and connection state for inbound ZMQ subscriptions.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ZmqDiag {
  /// Whether ZMQ support is configured.
  pub enabled: bool,
  /// Whether the subscriber is currently connected.
  pub connected: bool,
  #[serde(default)]
  /// Configured ZMQ endpoints.
  pub endpoints: Vec<String>,
  #[serde(default)]
  /// Configured subscription topics.
  pub subscriptions: Vec<String>,
  /// Total received ZMQ events since process start.
  pub total_events: u64,
  /// Total receive or decode errors since process start.
  pub total_errors: u64,
  /// Unix timestamp of the last successfully processed event.
  pub last_event_epoch: Option<u64>,
  /// Count of observed sequence gaps.
  pub sequence_gaps: u64,
  #[serde(default)]
  /// Per-topic event counts.
  pub topic_counts: Vec<(String, u64)>,
  #[serde(default)]
  /// Error counts grouped by normalized error label.
  pub error_counts: Vec<(String, u64)>,
}

/// Diagnostic snapshot of the PostgreSQL storage backend.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct StorageDiag {
  /// PostgreSQL server version string.
  pub server_version: Option<String>,
  /// Total size of the daemon's database in bytes.
  pub database_bytes: Option<u64>,
  /// Bytes of WAL written since server start (activity indicator).
  pub wal_bytes: Option<u64>,
  /// Connections currently open against the daemon's database.
  pub connections: Option<i64>,
  /// Server-wide `max_connections` setting.
  pub max_connections: Option<i64>,
  /// Autovacuum workers currently running.
  pub autovacuum_running: Option<i64>,
  /// Height span covered by the reorg undo tables (lo, hi).
  #[serde(default)]
  pub undo_span: Option<(u32, u32)>,
  /// Per-pool utilization: (pool name, configured size, idle now).
  #[serde(default)]
  pub pool_stats: Vec<(String, u32, u64)>,
}

/// Point-in-time ingestion-loop runtime state.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct IngestionDiag {
  /// High-level ingestion activity state.
  pub activity: IngestionActivity,
  /// Detailed ingest machine state.
  pub state: IngestionStateName,
  /// Ingestion is explicitly paused by the control plane.
  pub paused: bool,
  /// Index-maintenance regime: `realtime` or `stepped`. Defaulted
  /// so a console talking to an older daemon still decodes the frame.
  #[serde(default)]
  pub sync_mode: String,
  /// Most recent upstream tip observed by block ingestion polling.
  pub last_polled_tip: Option<u32>,
  /// Last time ingestion connected new blocks into the projection.
  pub last_progress_epoch: Option<u64>,
  /// Current retry attempt for the in-flight batch, when applicable.
  pub retry_attempt: Option<u32>,
  /// Epoch when the current backoff sleep should end.
  pub backoff_until_epoch: Option<u64>,
  /// Whether the watchdog thinks ingestion is unexpectedly stalled.
  pub stalled: bool,
  /// The deferred index build in flight, when there is one.
  ///
  /// Carried separately from `state` because it explains a state rather than
  /// replacing it: a build is taken on the write path, so ingestion really is
  /// committing-and-waiting, and every layer's rate really is zero. Without
  /// this the console cannot tell that stall from a stopped chain. Defaulted
  /// so a console talking to an older daemon still decodes the frame.
  #[serde(default)]
  pub index_build: Option<IndexBuildDiag>,
}

/// A deferred index build in progress.
#[derive(Debug, Clone, Default, Serialize, Deserialize, PartialEq, Eq)]
pub struct IndexBuildDiag {
  pub index: String,
  pub table: String,
  pub partition: i32,
  /// `true` when the partition has closed — the build the deferral was aiming
  /// at, as opposed to a reconcile after leaving stepped mode.
  pub closed: bool,
  /// Position in the run and its length, so a long stall reads as "3 of 7"
  /// rather than as an unbounded wait.
  pub nth: usize,
  pub of: usize,
  /// Seconds this build has been running.
  pub elapsed_secs: u64,
}

/// Human-oriented ingestion activity state.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IngestionActivity {
  /// Ingestion is actively absorbing or connecting blocks.
  Active,
  /// Ingestion is sleeping due to pause or backpressure.
  Sleeping,
  /// Ingestion is idle because it is caught up or awaiting a new poll cycle.
  #[default]
  Idle,
}

/// Detailed ingest machine state exposed through IPC.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum IngestionStateName {
  #[default]
  Boot,
  PollingTip,
  Idle,
  Paused,
  Fetching,
  Committing,
  Backoff,
}

/// Scheduler lanes used by the transactional index coordinator.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum IndexSchedulerLane {
  #[default]
  Idle,
  Forward,
  TxTreeCatchUp,
  MetricsCatchUp,
  ValidatorCatchUp,
}

/// Execution phases for one forward block-lane turn.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ForwardLanePhase {
  #[default]
  Idle,
  AcquireChainState,
  BeginTx,
  FetchBlock,
  PlanConnect,
  ApplyStagePlan,
  CommitTx,
}

/// Supervisor-owned forward run liveness state.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum ForwardProgressState {
  #[default]
  Idle,
  Starting,
  Active,
  Slow,
  Stale,
  Recovering,
}

/// Outer scheduler-loop phases before a lane turn is actively running.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "snake_case")]
pub enum SchedulerLoopPhase {
  #[default]
  Idle,
  FetchTip,
  LoadStageHeights,
  ChooseLane,
  RunLane,
}

/// Point-in-time forward lane diagnostics for the block indexer.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ForwardLaneDiag {
  /// Supervisor-issued lease id for the current forward run.
  #[serde(default)]
  pub run_id: Option<u64>,
  /// Height currently being processed by the forward lane.
  pub height: Option<u32>,
  /// Active forward execution phase when the lane is busy.
  pub phase: Option<ForwardLanePhase>,
  /// Specific index stage currently being applied inside `ApplyStagePlan`.
  #[serde(default)]
  pub stage: Option<String>,
  /// Fine-grained substep currently executing inside the active stage.
  #[serde(default)]
  pub substep: Option<String>,
  /// Milliseconds spent in the current phase.
  pub phase_elapsed_ms: Option<u64>,
  /// Highest height fully committed by the forward lane.
  pub last_success_height: Option<u32>,
  /// Unix timestamp of the most recent successful forward commit.
  pub last_success_epoch: Option<u64>,
  /// Unix timestamp of the most recent forward attempt start.
  pub last_attempt_epoch: Option<u64>,
  /// Unix timestamp of the most recent heartbeat observed for the active run.
  #[serde(default)]
  pub last_heartbeat_epoch: Option<u64>,
  /// Current chunk index inside the active substep, when applicable.
  #[serde(default)]
  pub chunk_index: Option<u32>,
  /// Total chunks inside the active substep, when applicable.
  #[serde(default)]
  pub chunk_total: Option<u32>,
  /// Unix timestamp of the most recent durable progress observed for the active
  /// run.
  #[serde(default)]
  pub last_progress_epoch: Option<u64>,
  /// Supervisor-owned liveness state for the active forward run.
  #[serde(default)]
  pub progress_state: ForwardProgressState,
  /// Recovery attempts against the current stuck height.
  #[serde(default)]
  pub recovery_attempts: u32,
  /// Whether the forward lane watchdog considers the current phase stalled.
  #[serde(default)]
  pub stalled: bool,
}

/// Point-in-time diagnostics for the transactional index scheduler.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct IndexSchedulerDiag {
  /// Scheduler is sharing turns across multiple active lanes.
  pub contention: bool,
  /// Lane currently being executed, when any.
  pub active_lane: Option<IndexSchedulerLane>,
  /// Lanes that currently have runnable work.
  #[serde(default)]
  pub active_lanes: Vec<IndexSchedulerLane>,
  /// Per-lane turn counts since process start.
  #[serde(default)]
  pub lane_turn_counts: Vec<(IndexSchedulerLane, u64)>,
  /// Duration of the most recent turn per lane.
  #[serde(default)]
  pub lane_last_turn_ms: Vec<(IndexSchedulerLane, u64)>,
  /// Current forward-lane execution details.
  #[serde(default)]
  pub forward: ForwardLaneDiag,
  /// Current outer scheduler-loop phase.
  #[serde(default)]
  pub loop_phase: SchedulerLoopPhase,
  /// Unix timestamp when the scheduler snapshot was recorded.
  pub snapshot_epoch: Option<u64>,
  /// Milliseconds spent in the current outer scheduler-loop phase.
  pub loop_phase_elapsed_ms: Option<u64>,
  /// Milliseconds since the most recent completed lane turn.
  pub last_turn_age_ms: Option<u64>,
}

/// High-level health classification for the core ingest/connect pipeline.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum CorePipelineHealth {
  /// Ingest and indexing are progressing normally.
  #[default]
  Healthy,
  /// Runtime truth is stale or progress is degraded but not fully stalled.
  Degraded,
  /// Core pipeline progress appears stalled.
  Stalled,
}

/// Derived core-pipeline truth surfaced to IPC and console clients.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CorePipelineDiag {
  /// Derived health classification for the core pipeline.
  #[serde(default)]
  pub health: CorePipelineHealth,
  /// Whether ingest is currently paused by the control plane.
  #[serde(default)]
  pub paused: bool,
  /// Unix timestamp of the last successful ingest commit.
  pub last_ingest_success_epoch: Option<u64>,
  /// Unix timestamp of the last successful forward block commit.
  pub last_forward_success_epoch: Option<u64>,
  /// Unix timestamp of the last attempted forward turn start.
  pub last_forward_attempt_epoch: Option<u64>,
  /// Unix timestamp when the current scheduler diagnostic snapshot was built.
  pub scheduler_snapshot_epoch: Option<u64>,
}

/// Diagnostic snapshot of the PostgreSQL connection pool and server.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct PostgresDiag {
  /// Server version string.
  pub version: Option<String>,
  /// Configured write-pool connection count.
  pub write_pool_size: Option<usize>,
  /// Currently available write-pool connections.
  pub write_pool_available: Option<usize>,
  /// Configured query-pool connection count.
  pub query_pool_size: Option<usize>,
  /// Currently available query-pool connections.
  pub query_pool_available: Option<usize>,
  /// Configured heavy-query-pool connection count.
  #[serde(default)]
  pub heavy_query_pool_size: Option<usize>,
  /// Currently available heavy-query-pool connections.
  #[serde(default)]
  pub heavy_query_pool_available: Option<usize>,
  /// Configured admin-pool connection count.
  pub admin_pool_size: Option<usize>,
  /// Currently available admin-pool connections.
  pub admin_pool_available: Option<usize>,
  /// Statements slower than this threshold are logged (milliseconds).
  #[serde(default)]
  pub slow_query_ms: Option<u64>,
  /// Server-wide `max_connections` setting.
  pub max_connections: Option<i32>,
  /// Server-wide `shared_buffers` setting.
  pub shared_buffers: Option<String>,
  /// Server-wide `work_mem` setting.
  pub work_mem: Option<String>,
  /// Server-wide `effective_cache_size` setting.
  pub effective_cache_size: Option<String>,
}

/// High-level upstream node identity fields surfaced to the console.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct CoreInfo {
  /// Upstream version number when available.
  pub version: Option<u64>,
  /// Upstream subversion string when available.
  pub subversion: Option<String>,
}

/// Resolved daemon configuration published to the console.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ConfigSnapshot {
  /// Resolved upstream source configuration.
  pub sources: ConfigSourcesSnapshot,
  /// Resolved public REST API configuration.
  pub api: ConfigApiSnapshot,
  /// Resolved enabled PostgreSQL-backed indices.
  pub indices: ConfigIndexSnapshot,
  /// Resolved live feature toggles and tuning.
  pub live: ConfigLiveSnapshot,
  /// Resolved overlay feature toggles.
  pub overlays: ConfigOverlaySnapshot,
  /// Resolved PostgreSQL connection settings (URL redacted).
  #[serde(default)]
  pub postgres: ConfigPostgresSnapshot,
}

/// Resolved upstream and storage source configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigSourcesSnapshot {
  /// Resolved upstream REST base URL.
  pub rest_url: String,
  /// Whether PostgreSQL-backed indexing is enabled.
  pub postgres_enabled: bool,
  /// Resolved PostgreSQL write-pool size.
  pub postgres_write_pool_size: Option<usize>,
  #[serde(default)]
  /// Resolved PostgreSQL query-pool size.
  pub postgres_query_pool_size: Option<usize>,
  #[serde(default)]
  /// Resolved PostgreSQL admin-pool size.
  pub postgres_admin_pool_size: Option<usize>,
  #[serde(default)]
  /// Whether upstream JSON-RPC support is enabled.
  pub rpc_enabled: bool,
  #[serde(default)]
  /// Resolved upstream JSON-RPC URL.
  pub rpc_url: Option<String>,
  #[serde(default)]
  /// Resolved JSON-RPC authentication type.
  pub rpc_auth_type: Option<String>,
  #[serde(default)]
  /// Whether ZMQ support is enabled.
  pub zmq_enabled: bool,
  #[serde(default)]
  /// Configured ZMQ connection endpoints.
  pub zmq_connect: Vec<String>,
  #[serde(default)]
  /// Configured ZMQ subscription topics.
  pub zmq_subscriptions: Vec<String>,
  #[serde(default)]
  /// Redacted indicator that PostgreSQL is configured.
  pub postgres_url: Option<String>,
}

/// Resolved public REST API configuration.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigApiSnapshot {
  /// Whether the public REST API is enabled.
  pub enabled: bool,
  /// Resolved REST bind address.
  pub bind: Option<String>,
  #[serde(default)]
  /// Configured REST concurrency limit.
  pub rest_concurrency: Option<usize>,
}

/// Resolved enabled PostgreSQL-backed indices.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigIndexSnapshot {
  /// Whether the address/spent index is enabled.
  pub address: bool,
  /// Whether the validator index is enabled.
  pub validator: bool,
  /// Whether the metrics index is enabled.
  pub metrics: bool,
}

/// Resolved live feature toggles and tuning.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigLiveSnapshot {
  #[serde(default)]
  /// Whether live mempool support is enabled.
  pub mempool: bool,
  #[serde(default)]
  /// Whether live InstantSend tagging is enabled.
  pub islock: bool,
  #[serde(default)]
  /// Whether live ChainLock tagging is enabled.
  pub chainlock: bool,
  #[serde(default)]
  /// First height at which ChainLock tagging becomes meaningful.
  pub chainlock_start_block: Option<u32>,
  #[serde(default)]
  /// Whether live governance support is enabled.
  pub governance: bool,
  #[serde(default)]
  /// Whether live network snapshots are enabled.
  pub network: bool,
  #[serde(default)]
  /// Whether the Streaming HTTP endpoint is enabled.
  pub stream: bool,
  #[serde(default)]
  /// Maximum number of recent mempool transactions retained.
  pub recent_mempool_txs: Option<usize>,
  #[serde(default)]
  /// Per-client stream buffer capacity.
  pub stream_client_buffer: Option<usize>,
  #[serde(default)]
  /// Streaming HTTP heartbeat interval in seconds.
  pub stream_heartbeat_secs: Option<u64>,
}

/// Resolved overlay feature toggles.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigOverlaySnapshot {
  #[serde(default)]
  /// Whether the validator protx overlay is enabled.
  pub protx: bool,
  #[serde(default)]
  /// Whether the block ChainLock overlay is enabled.
  pub chainlock: bool,
}

/// Resolved PostgreSQL connection settings published to the console.
/// The URL is redacted before it ever reaches this struct.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ConfigPostgresSnapshot {
  /// Redacted connection URL (never contains a password).
  #[serde(default)]
  pub url_redacted: Option<String>,
  #[serde(default)]
  pub write_pool_size: Option<usize>,
  #[serde(default)]
  pub query_pool_size: Option<usize>,
  #[serde(default)]
  pub heavy_query_pool_size: Option<usize>,
  #[serde(default)]
  pub admin_pool_size: Option<usize>,
  #[serde(default)]
  pub slow_query_ms: Option<u64>,
}

/// Periodic daemon heartbeat payload sent to IPC clients.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DaemonSnapshot {
  /// Current connectivity state of the upstream REST source.
  pub data_source: RestSourceState,
  /// Best known upstream network tip height.
  pub network_tip: Option<u32>,
  /// Highest block fully ingested into durable storage.
  pub ingestion_tip: Option<u32>,
  /// Hash of the most recently ingested block.
  pub ingestion_tip_hash: Option<String>,
  /// Ingestion runtime state such as pause/throttle activity.
  #[serde(default)]
  pub ingestion_diag: IngestionDiag,
  /// Index scheduler runtime state and lane diagnostics.
  #[serde(default)]
  pub index_scheduler_diag: IndexSchedulerDiag,
  /// Derived core pipeline health and freshness state.
  #[serde(default)]
  pub core_pipeline_diag: CorePipelineDiag,
  /// Total blocks ingested since process start.
  pub blocks_ingested_total: u64,
  /// Per-stage progress snapshots.
  pub stages: Vec<StageProgress>,
  /// Durable PostgreSQL chain progress and recovery state.
  #[serde(default)]
  pub chain_progress_diag: ChainProgressDiag,
  /// Resolved runtime configuration snapshot.
  pub config: ConfigSnapshot,
  /// Process uptime in seconds.
  pub uptime_secs: u64,
  /// Daemon version string.
  pub version: String,
  #[serde(default)]
  /// Aggregate outbound REST client metrics.
  pub rest_stats: RestClientStats,
  /// REST API server (hyphend) request metrics.
  #[serde(default)]
  pub server_stats: ApiServerStats,
  #[serde(default)]
  /// PostgreSQL storage diagnostics snapshot.
  pub storage_diag: StorageDiag,
  #[serde(default)]
  /// PostgreSQL diagnostics snapshot.
  pub postgres_diag: PostgresDiag,
  #[serde(default)]
  /// High-level upstream node identity fields.
  pub core_info: CoreInfo,
  /// PostgreSQL pool is reachable.
  #[serde(default)]
  pub postgres_ok: bool,
  /// Upstream REST interface is reachable.
  #[serde(default)]
  pub core_rest_ok: bool,
  /// Optional upstream JSON-RPC diagnostics.
  #[serde(default)]
  pub rpc_diag: RpcDiag,
  /// Optional upstream ZMQ diagnostics.
  #[serde(default)]
  pub zmq_diag: ZmqDiag,
  /// Stages signalled to pause but not yet idle.
  #[serde(default)]
  pub pausing_stages: Vec<String>,
  /// Stages idle and suspended for reindex/rewind.
  #[serde(default)]
  pub suspended_stages: Vec<String>,
  /// Total PostgreSQL database size in bytes.
  #[serde(default)]
  pub pg_db_bytes: Option<u64>,
  /// Per-table PostgreSQL sizes (table_name, bytes).
  #[serde(default)]
  pub pg_table_sizes: Vec<(String, u64)>,
  /// Size of the Postgres WAL directory in bytes.
  #[serde(default)]
  pub pg_wal_bytes: Option<u64>,
}

/// Daemon -> client broadcast messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonMessage {
  /// Full heartbeat snapshot.
  Snapshot(Box<DaemonSnapshot>),
  /// Notification that a block was ingested.
  BlockIngested {
    /// Height of the ingested block.
    height: u32,
    /// Hash of the ingested block.
    hash: [u8; 32],
  },
  /// Notification that a transaction was observed.
  TxReceived {
    /// Transaction identifier.
    txid: [u8; 32],
  },
  /// Incremental index progress update.
  IndexProgress {
    /// Stable stage identifier.
    stage: String,
    /// Indexed height now reached by the stage.
    height: u32,
    /// Best known tip height for the stage.
    tip: u32,
  },
  /// Progress update for a long-running control-plane command.
  CommandProgress {
    /// Correlation identifier chosen by the client.
    id: u64,
    /// Human-readable command progress message.
    message: String,
  },
  /// Response to a command.
  CommandResult {
    /// Correlation identifier chosen by the client.
    id: u64,
    /// Whether the command completed successfully.
    success: bool,
    /// Human-readable command result message.
    message: String,
  },
}

/// Client -> daemon commands.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum DaemonCommand {
  /// Rewind all indices to a specific height.
  Rewind {
    /// Client-chosen correlation identifier.
    id: u64,
    /// Target height to rewind to.
    target_height: u32,
  },
  /// Disconnect a single block at the current tip.
  DisconnectTip {
    /// Client-chosen correlation identifier.
    id: u64,
  },
  /// Force a full reindex of the named stage.
  ForceReindex {
    /// Client-chosen correlation identifier.
    id: u64,
    /// Stable stage identifier to rebuild.
    stage: String,
  },
}
