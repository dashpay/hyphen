//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Forward block indexer runner and bounded prefetch helpers.

use std::collections::HashMap;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::db;
use crate::error::DaemonError;
use crate::policy::log::{log_threshold_met, LOG_INTERVAL_SECS};
use crate::policy::watchdog::{
  WATCHDOG_ACTIVITY_STALE_SECS, WATCHDOG_OPERATION_TIMEOUT_SECS, WATCHDOG_PROGRESS_STALE_SECS,
  WATCHDOG_RECOVERY_LIMIT, WATCHDOG_SLOW_OPERATION_MS,
};
use crate::visor::{ProgressState, RecoveryDisposition, RunState, ShutdownPhase, Supervisable};
use hyphend::wire;

use super::batch::{BatchSizer, DEFAULT_COMMIT_BATCH_BLOCKS};
use super::{BlockCtx, ForwardPipeline, IndexerHandle, IndexerLoopCtx, Stage};

const BLOCK_TIP_WAIT_SECS: u64 = 2;
const BLOCK_PAUSE_POLL_SECS: u64 = 1;
const BLOCK_PROGRESS_LOG_INTERVAL_SECS: u64 = LOG_INTERVAL_SECS;
const SCHEDULER_CYCLE_BUDGET_MS: u64 = 1_000;
const SCHEDULER_LANE_BASE_BUDGET_MS: u64 = 250;
const PG_SHUTDOWN_BACKOFF_SECS: u64 = 5;

fn now_epoch() -> u64 {
  SystemTime::now()
    .duration_since(UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

fn rebuild_progress_pct(start: u32, current: u32, tip: u32) -> u32 {
  if tip <= start {
    return 100;
  }
  let total = tip.saturating_sub(start).saturating_add(1);
  let done = current.saturating_sub(start).saturating_add(1);
  ((done.saturating_mul(100)) / total).min(100)
}

type PrefetchedBlock = Result<BlockCtx, DaemonError>;
type PrefetchReceiver = tokio::sync::mpsc::Receiver<PrefetchedBlock>;
type PrefetchTask = tokio::task::JoinHandle<Result<(), DaemonError>>;

async fn acquire_chain_state_read(
  ctx: &IndexerLoopCtx,
  height: u32,
  lane: &'static str,
) -> Result<tokio::sync::OwnedRwLockReadGuard<()>, DaemonError> {
  let wait_started = Instant::now();
  let guard = Arc::clone(&ctx.chain_state_lock).read_owned().await;
  let wait_ms = wait_started.elapsed().as_millis() as u64;
  if wait_ms >= 1_000 {
    tracing::warn!(lane, height, wait_ms, "chain-state read lock waited");
  } else {
    tracing::debug!(lane, height, wait_ms, "chain-state read lock acquired");
  }
  Ok(guard)
}

struct AbortOnDropTask<T> {
  handle: Option<tokio::task::JoinHandle<T>>,
}

impl<T> AbortOnDropTask<T> {
  fn new(handle: tokio::task::JoinHandle<T>) -> Self {
    Self {
      handle: Some(handle),
    }
  }
}

impl<T> Drop for AbortOnDropTask<T> {
  fn drop(&mut self) {
    if let Some(handle) = self.handle.take() {
      handle.abort();
    }
  }
}

impl<T> Future for AbortOnDropTask<T> {
  type Output = Result<T, tokio::task::JoinError>;

  fn poll(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Self::Output> {
    let handle = self
      .handle
      .as_mut()
      .expect("abort-on-drop task handle should be present");
    match Pin::new(handle).poll(cx) {
      Poll::Ready(result) => {
        self.handle.take();
        Poll::Ready(result)
      }
      Poll::Pending => Poll::Pending,
    }
  }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
enum SchedulerLane {
  Forward,
  TxTreeCatchUp,
  MetricsCatchUp,
  ValidatorCatchUp,
}

impl SchedulerLane {
  fn to_wire(self) -> wire::msg::IndexSchedulerLane {
    match self {
      Self::Forward => wire::msg::IndexSchedulerLane::Forward,
      Self::TxTreeCatchUp => wire::msg::IndexSchedulerLane::TxTreeCatchUp,
      Self::MetricsCatchUp => wire::msg::IndexSchedulerLane::MetricsCatchUp,
      Self::ValidatorCatchUp => wire::msg::IndexSchedulerLane::ValidatorCatchUp,
    }
  }
}

#[derive(Debug, Clone, Default)]
pub struct IndexSchedulerState {
  active_lane: Option<SchedulerLane>,
  active_lanes: Vec<SchedulerLane>,
  lane_turn_counts: HashMap<SchedulerLane, u64>,
  lane_last_turn_ms: HashMap<SchedulerLane, u64>,
  loop_phase: wire::msg::SchedulerLoopPhase,
  loop_phase_started_at: Option<Instant>,
  last_turn_finished_at: Option<Instant>,
  last_turn_finished_epoch: Option<u64>,
  forward: ForwardRuntimeState,
  catch_up: CatchUpRuntimeState,
}

#[derive(Debug, Clone, Default)]
struct ForwardRuntimeState {
  run_id: Option<u64>,
  height: Option<u32>,
  phase: Option<wire::msg::ForwardLanePhase>,
  stage: Option<Stage>,
  substage: Option<ForwardSubstage>,
  progress_state: wire::msg::ForwardProgressState,
  phase_started_at: Option<Instant>,
  last_success_height: Option<u32>,
  last_success_epoch: Option<u64>,
  last_attempt_epoch: Option<u64>,
  last_heartbeat_epoch: Option<u64>,
  last_progress_epoch: Option<u64>,
  chunk_index: Option<u32>,
  chunk_total: Option<u32>,
  recovery_attempts: u32,
  stalled: bool,
}

/// Stable diagnostic contract for forward-lane substages.
pub(crate) trait Substage:
  Copy + Eq + Send + Sync + std::fmt::Debug + Into<ForwardSubstage> + 'static
{
  /// Returns the forward phase that owns this substage.
  fn phase(self) -> wire::msg::ForwardLanePhase;

  /// Returns the stable diagnostic name for this substage.
  fn name(self) -> &'static str;
}

/// Fine-grained substages reported while building a forward plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum PlanConnectSubstage {
  Block,
  Transaction,
  Script,
  SpentCollectInputs,
  SpentSortInputs,
  SpentPrevoutLookup,
  SpentFeeUpdates,
  SpentAddressHistoryRows,
  SpentAddressStatsRows,
  Metrics,
  Validator,
}

impl Substage for PlanConnectSubstage {
  fn phase(self) -> wire::msg::ForwardLanePhase {
    wire::msg::ForwardLanePhase::PlanConnect
  }

  fn name(self) -> &'static str {
    match self {
      Self::Block => "block",
      Self::Transaction => "transaction",
      Self::Script => "script",
      Self::SpentCollectInputs => "spent_collect_inputs",
      Self::SpentSortInputs => "spent_sort_inputs",
      Self::SpentPrevoutLookup => "spent_prevout_lookup",
      Self::SpentFeeUpdates => "spent_fee_updates",
      Self::SpentAddressHistoryRows => "spent_address_history_rows",
      Self::SpentAddressStatsRows => "spent_address_stats_rows",
      Self::Metrics => "metrics",
      Self::Validator => "validator",
    }
  }
}

/// Fine-grained substages reported while applying a forward plan.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ApplyStagePlanSubstage {
  StageStart,
  TransactionRows,
  TransactionInputs,
  TransactionOutputsStage,
  TransactionOutputsApply,
  TransactionSpecialPayloads,
  SpentPrevoutLookup,
  SpentHistory,
  SpentMarkOutputs,
  SpentUpdateInputs,
  SpentUpdateFees,
  SpentAddressHistory,
  SpentAddressStats,
}

impl Substage for ApplyStagePlanSubstage {
  fn phase(self) -> wire::msg::ForwardLanePhase {
    wire::msg::ForwardLanePhase::ApplyStagePlan
  }

  fn name(self) -> &'static str {
    match self {
      Self::StageStart => "stage_start",
      Self::TransactionRows => "transaction_rows",
      Self::TransactionInputs => "transaction_inputs",
      Self::TransactionOutputsStage => "transaction_outputs_stage",
      Self::TransactionOutputsApply => "transaction_outputs_apply",
      Self::TransactionSpecialPayloads => "transaction_special_payloads",
      Self::SpentPrevoutLookup => "spent_prevout_lookup",
      Self::SpentHistory => "spent_history",
      Self::SpentMarkOutputs => "spent_mark_outputs",
      Self::SpentUpdateInputs => "spent_update_inputs",
      Self::SpentUpdateFees => "spent_update_fees",
      Self::SpentAddressHistory => "spent_address_history",
      Self::SpentAddressStats => "spent_address_stats",
    }
  }
}

/// Unified forward-lane substage envelope consumed by scheduler diagnostics.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ForwardSubstage {
  PlanConnect(PlanConnectSubstage),
  ApplyStagePlan(ApplyStagePlanSubstage),
}

impl ForwardSubstage {
  fn phase(self) -> wire::msg::ForwardLanePhase {
    match self {
      Self::PlanConnect(substage) => substage.phase(),
      Self::ApplyStagePlan(substage) => substage.phase(),
    }
  }

  fn name(self) -> &'static str {
    match self {
      Self::PlanConnect(substage) => substage.name(),
      Self::ApplyStagePlan(substage) => substage.name(),
    }
  }
}

impl From<PlanConnectSubstage> for ForwardSubstage {
  fn from(value: PlanConnectSubstage) -> Self {
    Self::PlanConnect(value)
  }
}

impl From<ApplyStagePlanSubstage> for ForwardSubstage {
  fn from(value: ApplyStagePlanSubstage) -> Self {
    Self::ApplyStagePlan(value)
  }
}

#[derive(Debug, Clone)]
struct CatchUpRuntimeState {
  run_id: Option<u64>,
  lane: Option<SchedulerLane>,
  height: Option<u32>,
  progress_state: ProgressState,
  last_activity_epoch: Option<u64>,
  last_progress_epoch: Option<u64>,
  recovery_attempts: u32,
}

impl Default for CatchUpRuntimeState {
  fn default() -> Self {
    Self {
      run_id: None,
      lane: None,
      height: None,
      progress_state: ProgressState::Idle,
      last_activity_epoch: None,
      last_progress_epoch: None,
      recovery_attempts: 0,
    }
  }
}

impl IndexSchedulerState {
  pub fn snapshot(&self) -> wire::msg::IndexSchedulerDiag {
    let mut turn_counts: Vec<(wire::msg::IndexSchedulerLane, u64)> = self
      .lane_turn_counts
      .iter()
      .map(|(lane, count)| (lane.to_wire(), *count))
      .collect();
    turn_counts.sort_by_key(|(lane, _)| lane_sort_key(*lane));

    let mut last_turn_ms: Vec<(wire::msg::IndexSchedulerLane, u64)> = self
      .lane_last_turn_ms
      .iter()
      .map(|(lane, elapsed)| (lane.to_wire(), *elapsed))
      .collect();
    last_turn_ms.sort_by_key(|(lane, _)| lane_sort_key(*lane));

    wire::msg::IndexSchedulerDiag {
      contention: self.active_lanes.len() > 1,
      active_lane: self.active_lane.map(SchedulerLane::to_wire),
      active_lanes: self
        .active_lanes
        .iter()
        .copied()
        .map(SchedulerLane::to_wire)
        .collect(),
      lane_turn_counts: turn_counts,
      lane_last_turn_ms: last_turn_ms,
      forward: wire::msg::ForwardLaneDiag {
        run_id: self.forward.run_id,
        height: self.forward.height,
        phase: self.forward.phase,
        stage: self.forward.stage.map(|stage| stage.name().to_owned()),
        substep: self
          .forward
          .substage
          .map(|substage| substage.name().to_owned()),
        phase_elapsed_ms: self
          .forward
          .phase_started_at
          .map(|started| started.elapsed().as_millis() as u64),
        last_success_height: self.forward.last_success_height,
        last_success_epoch: self.forward.last_success_epoch,
        last_attempt_epoch: self.forward.last_attempt_epoch,
        last_heartbeat_epoch: self.forward.last_heartbeat_epoch,
        last_progress_epoch: self.forward.last_progress_epoch,
        chunk_index: self.forward.chunk_index,
        chunk_total: self.forward.chunk_total,
        progress_state: self.forward.progress_state,
        recovery_attempts: self.forward.recovery_attempts,
        stalled: self.forward.stalled,
      },
      loop_phase: self.loop_phase,
      snapshot_epoch: Some(now_epoch()),
      loop_phase_elapsed_ms: self
        .loop_phase_started_at
        .map(|started| started.elapsed().as_millis() as u64),
      last_turn_age_ms: self
        .last_turn_finished_at
        .map(|finished| finished.elapsed().as_millis() as u64),
    }
  }

  pub(crate) fn set_idle(&mut self) {
    self.active_lane = None;
    self.active_lanes.clear();
    self.loop_phase = wire::msg::SchedulerLoopPhase::Idle;
    self.loop_phase_started_at = Some(Instant::now());
    self.clear_forward_runtime();
    self.clear_catch_up_runtime();
  }

  fn set_active_lanes(&mut self, lanes: &[SchedulerLane]) {
    self.active_lanes = lanes.to_vec();
    if !lanes.contains(&SchedulerLane::Forward) && self.active_lane != Some(SchedulerLane::Forward)
    {
      self.clear_forward_runtime();
    }
  }

  fn begin_turn(&mut self, lane: SchedulerLane) {
    self.active_lane = Some(lane);
    self.loop_phase = wire::msg::SchedulerLoopPhase::RunLane;
    self.loop_phase_started_at = Some(Instant::now());
    if lane == SchedulerLane::Forward {
      self.clear_catch_up_runtime();
    }
  }

  fn finish_turn(&mut self, lane: SchedulerLane, elapsed_ms: u64) {
    *self.lane_turn_counts.entry(lane).or_insert(0) += 1;
    self.lane_last_turn_ms.insert(lane, elapsed_ms);
    self.active_lane = None;
    self.last_turn_finished_at = Some(Instant::now());
    self.last_turn_finished_epoch = Some(now_epoch());
  }

  fn enter_loop_phase(&mut self, phase: wire::msg::SchedulerLoopPhase) {
    self.loop_phase = phase;
    self.loop_phase_started_at = Some(Instant::now());
  }

  fn enter_forward_phase(&mut self, height: u32, phase: wire::msg::ForwardLanePhase) {
    if self.forward.height != Some(height) {
      let next_run_id = self.forward.run_id.unwrap_or(0).saturating_add(1);
      self.forward.run_id = Some(next_run_id);
    }
    self.forward.height = Some(height);
    self.forward.phase = Some(phase);
    self.forward.stage = None;
    self.forward.substage = None;
    self.forward.phase_started_at = Some(Instant::now());
    let epoch = now_epoch();
    self.forward.last_attempt_epoch = Some(epoch);
    self.forward.last_heartbeat_epoch = Some(epoch);
    self.forward.last_progress_epoch = Some(epoch);
    self.forward.chunk_index = None;
    self.forward.chunk_total = None;
    self.forward.progress_state = wire::msg::ForwardProgressState::Active;
    self.forward.stalled = false;
  }

  fn start_forward_run(&mut self, height: u32) -> u64 {
    let next_run_id = self.forward.run_id.unwrap_or(0).saturating_add(1);
    self.forward.run_id = Some(next_run_id);
    self.forward.height = Some(height);
    self.forward.phase = None;
    self.forward.stage = None;
    self.forward.substage = None;
    self.forward.phase_started_at = Some(Instant::now());
    let epoch = now_epoch();
    self.forward.last_attempt_epoch = Some(epoch);
    self.forward.last_heartbeat_epoch = Some(epoch);
    self.forward.last_progress_epoch = None;
    self.forward.chunk_index = None;
    self.forward.chunk_total = None;
    self.forward.progress_state = wire::msg::ForwardProgressState::Starting;
    self.forward.stalled = false;
    next_run_id
  }

  pub(crate) fn forward_run_state(&self, shutdown_phase: ShutdownPhase) -> RunState<u64> {
    RunState {
      run_id: self.forward.run_id,
      phase: self.forward.phase.map(forward_phase_name).unwrap_or("-"),
      subphase: self.forward.stage.map(Stage::name),
      detail: self.forward.substage.map(ForwardSubstage::name),
      progress_state: match self.forward.progress_state {
        wire::msg::ForwardProgressState::Idle => ProgressState::Idle,
        wire::msg::ForwardProgressState::Starting => ProgressState::Starting,
        wire::msg::ForwardProgressState::Active => ProgressState::Active,
        wire::msg::ForwardProgressState::Slow => ProgressState::Slow,
        wire::msg::ForwardProgressState::Stale => ProgressState::Stale,
        wire::msg::ForwardProgressState::Recovering => ProgressState::Recovering,
      },
      last_activity_epoch: self.forward.last_heartbeat_epoch,
      last_progress_epoch: self.forward.last_progress_epoch,
      recovery_attempts: self.forward.recovery_attempts,
      shutdown_phase,
    }
  }

  pub(crate) fn active_run_state(&self, shutdown_phase: ShutdownPhase) -> RunState<u64> {
    if self.forward.run_id.is_some() || self.forward.phase.is_some() {
      return self.forward_run_state(shutdown_phase);
    }
    if let Some(lane) = self.catch_up.lane {
      return RunState {
        run_id: self.catch_up.run_id,
        phase: lane_name(lane),
        subphase: None,
        detail: None,
        progress_state: self.catch_up.progress_state,
        last_activity_epoch: self.catch_up.last_activity_epoch,
        last_progress_epoch: self.catch_up.last_progress_epoch,
        recovery_attempts: self.catch_up.recovery_attempts,
        shutdown_phase,
      };
    }
    RunState {
      run_id: None,
      phase: "-",
      subphase: None,
      detail: None,
      progress_state: ProgressState::Idle,
      last_activity_epoch: None,
      last_progress_epoch: None,
      recovery_attempts: 0,
      shutdown_phase,
    }
  }

  pub(crate) fn forward_recovery_disposition(&self, run_id: Option<u64>) -> RecoveryDisposition {
    if self.forward.run_id.is_none() || self.forward.height.is_none() {
      return RecoveryDisposition::NotNeeded;
    }
    if run_id.is_some_and(|run_id| !self.accept_forward_run(run_id)) {
      return RecoveryDisposition::Blocked("run mismatch");
    }
    match self.forward.progress_state {
      wire::msg::ForwardProgressState::Stale | wire::msg::ForwardProgressState::Recovering => {
        if self.forward.recovery_attempts >= WATCHDOG_RECOVERY_LIMIT {
          RecoveryDisposition::Exhausted
        } else {
          RecoveryDisposition::Eligible
        }
      }
      wire::msg::ForwardProgressState::Idle
      | wire::msg::ForwardProgressState::Starting
      | wire::msg::ForwardProgressState::Active
      | wire::msg::ForwardProgressState::Slow => RecoveryDisposition::NotNeeded,
    }
  }

  pub(crate) fn catch_up_recovery_disposition(&self, run_id: Option<u64>) -> RecoveryDisposition {
    if self.catch_up.run_id.is_none()
      || self.catch_up.height.is_none()
      || self.catch_up.lane.is_none()
    {
      return RecoveryDisposition::NotNeeded;
    }
    if run_id.is_some() && self.catch_up.run_id != run_id {
      return RecoveryDisposition::Blocked("run mismatch");
    }
    match self.catch_up.progress_state {
      ProgressState::Recovering | ProgressState::Stale => {
        if self.catch_up.recovery_attempts >= WATCHDOG_RECOVERY_LIMIT {
          RecoveryDisposition::Exhausted
        } else {
          RecoveryDisposition::Eligible
        }
      }
      ProgressState::Idle
      | ProgressState::Starting
      | ProgressState::Active
      | ProgressState::Slow => RecoveryDisposition::NotNeeded,
    }
  }

  pub(crate) fn active_recovery_disposition(&self, run_id: Option<u64>) -> RecoveryDisposition {
    if self.forward.run_id.is_some() || self.forward.height.is_some() {
      self.forward_recovery_disposition(run_id)
    } else {
      self.catch_up_recovery_disposition(run_id)
    }
  }

  pub(crate) fn forward_watchdog_disposition(
    &self,
    run_id: Option<u64>,
    now_epoch: u64,
  ) -> RecoveryDisposition {
    if self.forward.run_id.is_none() || self.forward.height.is_none() {
      return RecoveryDisposition::NotNeeded;
    }
    if run_id.is_some_and(|run_id| !self.accept_forward_run(run_id)) {
      return RecoveryDisposition::Blocked("run mismatch");
    }
    if matches!(
      self.forward.progress_state,
      wire::msg::ForwardProgressState::Recovering
    ) {
      return if self.forward.recovery_attempts >= WATCHDOG_RECOVERY_LIMIT {
        RecoveryDisposition::Exhausted
      } else {
        RecoveryDisposition::Eligible
      };
    }

    let Some(last_heartbeat_epoch) = self.forward.last_heartbeat_epoch else {
      return RecoveryDisposition::NotNeeded;
    };
    let activity_age = now_epoch.saturating_sub(last_heartbeat_epoch);
    let progress_age = self
      .forward
      .last_progress_epoch
      .map(|epoch| now_epoch.saturating_sub(epoch))
      .unwrap_or(0);
    if activity_age >= WATCHDOG_ACTIVITY_STALE_SECS {
      if self.forward.recovery_attempts >= WATCHDOG_RECOVERY_LIMIT {
        RecoveryDisposition::Exhausted
      } else {
        RecoveryDisposition::Eligible
      }
    } else {
      if progress_age >= WATCHDOG_PROGRESS_STALE_SECS {
        tracing::warn!(
          run_id = ?self.forward.run_id,
          height = self.forward.height,
          phase = ?self.forward.phase,
          stage = ?self.forward.stage,
          progress_age,
          activity_age,
          "forward progress is stale while activity remains fresh"
        );
      }
      RecoveryDisposition::NotNeeded
    }
  }

  pub(crate) fn catch_up_watchdog_disposition(
    &self,
    run_id: Option<u64>,
    now_epoch: u64,
  ) -> RecoveryDisposition {
    if self.catch_up.run_id.is_none()
      || self.catch_up.height.is_none()
      || self.catch_up.lane.is_none()
    {
      return RecoveryDisposition::NotNeeded;
    }
    if run_id.is_some() && self.catch_up.run_id != run_id {
      return RecoveryDisposition::Blocked("run mismatch");
    }
    if matches!(self.catch_up.progress_state, ProgressState::Recovering) {
      return if self.catch_up.recovery_attempts >= WATCHDOG_RECOVERY_LIMIT {
        RecoveryDisposition::Exhausted
      } else {
        RecoveryDisposition::Eligible
      };
    }

    let Some(last_activity_epoch) = self.catch_up.last_activity_epoch else {
      return RecoveryDisposition::NotNeeded;
    };
    let activity_age = now_epoch.saturating_sub(last_activity_epoch);
    let progress_age = self
      .catch_up
      .last_progress_epoch
      .map(|epoch| now_epoch.saturating_sub(epoch))
      .unwrap_or(0);
    if activity_age >= WATCHDOG_ACTIVITY_STALE_SECS || progress_age >= WATCHDOG_PROGRESS_STALE_SECS
    {
      if self.catch_up.recovery_attempts >= WATCHDOG_RECOVERY_LIMIT {
        RecoveryDisposition::Exhausted
      } else {
        RecoveryDisposition::Eligible
      }
    } else {
      RecoveryDisposition::NotNeeded
    }
  }

  pub(crate) fn active_watchdog_disposition(
    &self,
    run_id: Option<u64>,
    now_epoch: u64,
  ) -> RecoveryDisposition {
    if self.forward.run_id.is_some() || self.forward.height.is_some() {
      self.forward_watchdog_disposition(run_id, now_epoch)
    } else {
      self.catch_up_watchdog_disposition(run_id, now_epoch)
    }
  }

  pub(crate) fn record_forward_recovery_for_run(
    &mut self,
    run_id: Option<u64>,
  ) -> Option<(u32, wire::msg::ForwardLanePhase, u32)> {
    if run_id.is_some_and(|run_id| !self.accept_forward_run(run_id)) {
      return None;
    }
    let height = self.forward.height?;
    let phase = self
      .forward
      .phase
      .unwrap_or(wire::msg::ForwardLanePhase::AcquireChainState);
    let attempts = self.record_forward_recovery(height, phase);
    Some((height, phase, attempts))
  }

  pub(crate) fn record_active_recovery_for_run(&mut self, run_id: Option<u64>) -> bool {
    if self.forward.run_id.is_some() || self.forward.height.is_some() {
      return self.record_forward_recovery_for_run(run_id).is_some();
    }
    self.record_catch_up_recovery_for_run(run_id)
  }

  pub(crate) fn fence_forward_run(&mut self, run_id: Option<u64>) {
    if run_id.is_some_and(|run_id| !self.accept_forward_run(run_id)) {
      return;
    }
    self.forward.progress_state = wire::msg::ForwardProgressState::Stale;
    self.forward.stalled = true;
  }

  pub(crate) fn fence_active_run(&mut self, run_id: Option<u64>) {
    if self.forward.run_id.is_some() || self.forward.phase.is_some() {
      self.fence_forward_run(run_id);
      return;
    }
    self.fence_catch_up_run(run_id);
  }

  fn accept_forward_run(&self, run_id: u64) -> bool {
    self.forward.run_id == Some(run_id)
  }

  fn record_forward_phase_event(
    &mut self,
    run_id: u64,
    height: u32,
    phase: wire::msg::ForwardLanePhase,
  ) {
    if self.accept_forward_run(run_id) {
      self.enter_forward_phase(height, phase);
    }
  }

  fn record_forward_stage_event(&mut self, run_id: u64, stage: Stage) {
    if self.accept_forward_run(run_id) {
      self.enter_forward_stage(stage);
    }
  }

  fn record_forward_activity_event(
    &mut self,
    run_id: u64,
    substage: ForwardSubstage,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
  ) {
    if self.accept_forward_run(run_id) {
      self.record_forward_activity(substage, chunk_index, chunk_total);
    }
  }

  fn record_forward_progress_event(
    &mut self,
    run_id: u64,
    substage: ForwardSubstage,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
  ) {
    if self.accept_forward_run(run_id) {
      self.record_forward_progress(substage, chunk_index, chunk_total);
    }
  }

  fn complete_forward_height_for_run(&mut self, run_id: u64, height: u32) {
    if self.accept_forward_run(run_id) {
      self.complete_forward_height(height);
    }
  }

  pub(crate) fn enter_forward_stage(&mut self, stage: Stage) {
    self.forward.stage = Some(stage);
    self.forward.substage = None;
    self.forward.chunk_index = None;
    self.forward.chunk_total = None;
    self.forward.last_heartbeat_epoch = Some(now_epoch());
    self.forward.progress_state = wire::msg::ForwardProgressState::Active;
  }

  pub(crate) fn record_forward_activity(
    &mut self,
    substage: ForwardSubstage,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
  ) {
    self.forward.substage = Some(substage);
    self.forward.chunk_index = chunk_index;
    self.forward.chunk_total = chunk_total;
    self.forward.last_heartbeat_epoch = Some(now_epoch());
    if self.forward.progress_state != wire::msg::ForwardProgressState::Recovering {
      self.forward.progress_state = wire::msg::ForwardProgressState::Active;
    }
  }

  pub(crate) fn record_forward_progress(
    &mut self,
    substage: ForwardSubstage,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
  ) {
    self.record_forward_activity(substage, chunk_index, chunk_total);
    self.forward.last_progress_epoch = self.forward.last_heartbeat_epoch;
  }

  fn complete_forward_height(&mut self, height: u32) {
    self.forward.last_success_height = Some(height);
    self.forward.last_success_epoch = Some(now_epoch());
    self.forward.height = None;
    self.forward.phase = None;
    self.forward.stage = None;
    self.forward.substage = None;
    self.forward.phase_started_at = None;
    self.forward.last_heartbeat_epoch = Some(now_epoch());
    self.forward.last_progress_epoch = self.forward.last_heartbeat_epoch;
    self.forward.chunk_index = None;
    self.forward.chunk_total = None;
    self.forward.progress_state = wire::msg::ForwardProgressState::Idle;
    self.forward.recovery_attempts = 0;
    self.forward.stalled = false;
  }

  fn record_forward_recovery(&mut self, height: u32, phase: wire::msg::ForwardLanePhase) -> u32 {
    if self.forward.height == Some(height) {
      self.forward.recovery_attempts += 1;
    } else {
      self.forward.recovery_attempts = 1;
      self.forward.height = Some(height);
    }
    self.forward.run_id = self
      .forward
      .run_id
      .map(|id| id.saturating_add(1))
      .or(Some(1));
    self.forward.phase = Some(phase);
    self.forward.stage = None;
    self.forward.substage = None;
    self.forward.phase_started_at = Some(Instant::now());
    let epoch = now_epoch();
    self.forward.last_attempt_epoch = Some(epoch);
    self.forward.last_heartbeat_epoch = Some(epoch);
    self.forward.last_progress_epoch = None;
    self.forward.chunk_index = None;
    self.forward.chunk_total = None;
    self.forward.progress_state = wire::msg::ForwardProgressState::Recovering;
    self.forward.stalled = true;
    self.forward.recovery_attempts
  }

  fn clear_forward_runtime(&mut self) {
    self.forward.run_id = None;
    self.forward.height = None;
    self.forward.phase = None;
    self.forward.stage = None;
    self.forward.substage = None;
    self.forward.phase_started_at = None;
    self.forward.last_heartbeat_epoch = None;
    self.forward.last_progress_epoch = None;
    self.forward.chunk_index = None;
    self.forward.chunk_total = None;
    self.forward.progress_state = wire::msg::ForwardProgressState::Idle;
    self.forward.recovery_attempts = 0;
    self.forward.stalled = false;
  }

  fn start_catch_up_run(&mut self, lane: SchedulerLane, height: u32) -> u64 {
    let next_run_id = self.catch_up.run_id.unwrap_or(0).saturating_add(1);
    let epoch = now_epoch();
    self.catch_up.run_id = Some(next_run_id);
    self.catch_up.lane = Some(lane);
    self.catch_up.height = Some(height);
    self.catch_up.last_activity_epoch = Some(epoch);
    self.catch_up.last_progress_epoch = None;
    self.catch_up.progress_state = ProgressState::Starting;
    self.catch_up.recovery_attempts = 0;
    next_run_id
  }

  fn record_catch_up_activity(&mut self, run_id: u64, lane: SchedulerLane, height: u32) {
    if self.catch_up.run_id == Some(run_id) && self.catch_up.lane == Some(lane) {
      self.catch_up.height = Some(height);
      self.catch_up.last_activity_epoch = Some(now_epoch());
      if self.catch_up.progress_state != ProgressState::Recovering {
        self.catch_up.progress_state = ProgressState::Active;
      }
    }
  }

  fn record_catch_up_progress(&mut self, run_id: u64, lane: SchedulerLane, height: u32) {
    if self.catch_up.run_id == Some(run_id) && self.catch_up.lane == Some(lane) {
      self.catch_up.height = Some(height);
      let epoch = now_epoch();
      self.catch_up.last_activity_epoch = Some(epoch);
      self.catch_up.last_progress_epoch = Some(epoch);
      if self.catch_up.progress_state != ProgressState::Recovering {
        self.catch_up.progress_state = ProgressState::Active;
      }
    }
  }

  fn record_catch_up_recovery_for_run(&mut self, run_id: Option<u64>) -> bool {
    if run_id.is_some() && self.catch_up.run_id != run_id {
      return false;
    }
    if self.catch_up.run_id.is_none()
      || self.catch_up.height.is_none()
      || self.catch_up.lane.is_none()
    {
      return false;
    }
    self.catch_up.recovery_attempts = self.catch_up.recovery_attempts.saturating_add(1);
    self.catch_up.run_id = self.catch_up.run_id.map(|run_id| run_id.saturating_add(1));
    let epoch = now_epoch();
    self.catch_up.last_activity_epoch = Some(epoch);
    self.catch_up.last_progress_epoch = Some(epoch);
    self.catch_up.progress_state = ProgressState::Recovering;
    true
  }

  fn fence_catch_up_run(&mut self, run_id: Option<u64>) {
    if run_id.is_some() && self.catch_up.run_id != run_id {
      return;
    }
    if self.catch_up.run_id.is_some() {
      self.catch_up.progress_state = ProgressState::Stale;
    }
  }

  fn clear_catch_up_runtime(&mut self) {
    self.catch_up.run_id = None;
    self.catch_up.lane = None;
    self.catch_up.height = None;
    self.catch_up.progress_state = ProgressState::Idle;
    self.catch_up.last_activity_epoch = None;
    self.catch_up.last_progress_epoch = None;
    self.catch_up.recovery_attempts = 0;
  }
}

#[derive(Debug)]
enum ForwardTurnError {
  Interrupted {
    height: u32,
    phase: Option<wire::msg::ForwardLanePhase>,
    reason: String,
  },
  Recoverable {
    height: u32,
    phase: wire::msg::ForwardLanePhase,
    reason: String,
  },
  Fatal(DaemonError),
}

#[derive(Debug)]
enum ForwardRunEvent {
  Phase {
    run_id: u64,
    height: u32,
    phase: wire::msg::ForwardLanePhase,
  },
  Stage {
    run_id: u64,
    stage: Stage,
  },
  Activity {
    run_id: u64,
    substage: ForwardSubstage,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
  },
  Progress {
    run_id: u64,
    substage: ForwardSubstage,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
  },
}

#[derive(Clone, Debug)]
pub(crate) struct ForwardRunReporter {
  run_id: u64,
  lifecycle: Arc<crate::lifecycle::Lifecycle>,
  operator_pause_requested: Arc<std::sync::atomic::AtomicBool>,
  tx: tokio::sync::mpsc::UnboundedSender<ForwardRunEvent>,
}

impl ForwardRunReporter {
  fn new(
    run_id: u64,
    lifecycle: Arc<crate::lifecycle::Lifecycle>,
    operator_pause_requested: Arc<std::sync::atomic::AtomicBool>,
    tx: tokio::sync::mpsc::UnboundedSender<ForwardRunEvent>,
  ) -> Self {
    Self {
      run_id,
      lifecycle,
      operator_pause_requested,
      tx,
    }
  }

  fn phase(&self, height: u32, phase: wire::msg::ForwardLanePhase) {
    let _ = self.tx.send(ForwardRunEvent::Phase {
      run_id: self.run_id,
      height,
      phase,
    });
  }

  pub(crate) fn stage(&self, stage: Stage) {
    let _ = self.tx.send(ForwardRunEvent::Stage {
      run_id: self.run_id,
      stage,
    });
  }

  pub(crate) fn activity<S: Substage>(
    &self,
    substage: S,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
  ) {
    let _ = self.tx.send(ForwardRunEvent::Activity {
      run_id: self.run_id,
      substage: substage.into(),
      chunk_index,
      chunk_total,
    });
  }

  pub(crate) fn progress<S: Substage>(
    &self,
    substage: S,
    chunk_index: Option<u32>,
    chunk_total: Option<u32>,
  ) {
    let _ = self.tx.send(ForwardRunEvent::Progress {
      run_id: self.run_id,
      substage: substage.into(),
      chunk_index,
      chunk_total,
    });
  }

  pub(crate) fn interrupt_requested(&self) -> bool {
    self.operator_pause_requested.load(Ordering::Relaxed) || !self.lifecycle.interruption_point()
  }
}

impl From<DaemonError> for ForwardTurnError {
  fn from(value: DaemonError) -> Self {
    Self::Fatal(value)
  }
}

#[derive(Debug, Clone)]
struct LaneBudgetCycle {
  budgets_ms: HashMap<SchedulerLane, u64>,
  cursor: usize,
}

impl LaneBudgetCycle {
  fn new(active_lanes: &[SchedulerLane]) -> Self {
    let mut budgets_ms: HashMap<SchedulerLane, u64> = active_lanes
      .iter()
      .copied()
      .map(|lane| (lane, SCHEDULER_LANE_BASE_BUDGET_MS))
      .collect();
    let used = SCHEDULER_LANE_BASE_BUDGET_MS.saturating_mul(active_lanes.len() as u64);
    let leftover = SCHEDULER_CYCLE_BUDGET_MS.saturating_sub(used);
    if leftover > 0 {
      for preferred in [
        SchedulerLane::TxTreeCatchUp,
        SchedulerLane::Forward,
        SchedulerLane::ValidatorCatchUp,
        SchedulerLane::MetricsCatchUp,
      ] {
        if let Some(budget) = budgets_ms.get_mut(&preferred) {
          *budget += leftover;
          break;
        }
      }
    }
    Self {
      budgets_ms,
      cursor: 0,
    }
  }

  fn matches(&self, active_lanes: &[SchedulerLane]) -> bool {
    self.budgets_ms.len() == active_lanes.len()
      && active_lanes
        .iter()
        .all(|lane| self.budgets_ms.contains_key(lane))
  }

  fn exhausted(&self) -> bool {
    self.budgets_ms.values().all(|budget| *budget == 0)
  }

  fn next_lane(&mut self, active_lanes: &[SchedulerLane]) -> Option<SchedulerLane> {
    if active_lanes.is_empty() {
      return None;
    }
    let len = active_lanes.len();
    for _ in 0..len {
      let lane = active_lanes[self.cursor % len];
      self.cursor = (self.cursor + 1) % len;
      if self.budgets_ms.get(&lane).copied().unwrap_or(0) > 0 {
        return Some(lane);
      }
    }
    None
  }

  fn remaining_ms(&self, lane: SchedulerLane) -> u64 {
    self.budgets_ms.get(&lane).copied().unwrap_or(0)
  }

  fn consume(&mut self, lane: SchedulerLane, elapsed_ms: u64) {
    if let Some(budget) = self.budgets_ms.get_mut(&lane) {
      *budget = budget.saturating_sub(elapsed_ms.max(1));
    }
  }
}

#[derive(Debug, Clone)]
struct LaneWork {
  lane: SchedulerLane,
  height: u32,
  active_stages: Vec<Stage>,
}

struct TurnShared<'a> {
  ctx: &'a IndexerLoopCtx,
  pipeline: &'a ForwardPipeline,
  rocks_tip: u32,
  batch_sizer: &'a Arc<Mutex<BatchSizer>>,
  prefetch_blocks: u32,
}

/// Block indexer loop: reads blocks from RocksDB and writes them to PG.
/// Not gated by sync_state; instead checks RocksDB tip directly.
pub(crate) async fn block_indexer_loop(
  ctx: IndexerLoopCtx,
  batch_sizer: Arc<Mutex<BatchSizer>>,
  prefetch_blocks: u32,
  mut handle: IndexerHandle,
) -> Result<(), DaemonError> {
  let pipeline = ForwardPipeline::new(&ctx);
  let mut lane_cycle: Option<LaneBudgetCycle> = None;

  while ctx.lifecycle.running() {
    if wait_if_block_paused(&mut handle).await {
      pipeline.reset_caches();
    }

    if !ctx.lifecycle.interruption_point() {
      break;
    }

    if let Ok(mut state) = ctx.scheduler_state.lock() {
      state.enter_loop_phase(wire::msg::SchedulerLoopPhase::FetchRocksTip);
    }
    let rocks_tip = tokio::time::timeout(
      Duration::from_secs(WATCHDOG_OPERATION_TIMEOUT_SECS),
      async {
        let db = Arc::clone(&ctx.db);
        tokio::task::spawn_blocking(move || db::get_tip_height(&db))
          .await
          .map_err(|e| DaemonError::Index(format!("spawn_blocking: {e}")))
      },
    )
    .await
    .map_err(|_| {
      DaemonError::Index(format!(
        "scheduler stalled while fetching rocks tip for {}s",
        WATCHDOG_OPERATION_TIMEOUT_SECS
      ))
    })??;

    let rocks_tip = match rocks_tip {
      Some(h) => h,
      None => {
        ctx
          .lifecycle
          .sleep(Duration::from_secs(BLOCK_TIP_WAIT_SECS))
          .await;
        continue;
      }
    };

    if let Ok(mut state) = ctx.scheduler_state.lock() {
      state.enter_loop_phase(wire::msg::SchedulerLoopPhase::LoadStageHeights);
    }
    let mut stage_heights = tokio::time::timeout(
      Duration::from_secs(WATCHDOG_OPERATION_TIMEOUT_SECS),
      load_stage_heights(&ctx),
    )
    .await
    .map_err(|_| {
      DaemonError::Index(format!(
        "scheduler stalled while loading stage heights for {}s",
        WATCHDOG_OPERATION_TIMEOUT_SECS
      ))
    })??;
    if let Ok(mut state) = ctx.scheduler_state.lock() {
      state.enter_loop_phase(wire::msg::SchedulerLoopPhase::ChooseLane);
    }
    let mut lanes = active_lanes(&ctx.enabled_stages, &stage_heights, rocks_tip);
    update_scheduler_lanes(&ctx, &lanes);

    if lanes.is_empty() {
      if let Ok(mut state) = ctx.scheduler_state.lock() {
        state.set_idle();
      }
      lane_cycle = None;
      ctx
        .lifecycle
        .sleep(Duration::from_secs(BLOCK_TIP_WAIT_SECS))
        .await;
      continue;
    }

    if lane_cycle
      .as_ref()
      .is_none_or(|cycle| !cycle.matches(&lanes) || cycle.exhausted())
    {
      lane_cycle = Some(LaneBudgetCycle::new(&lanes));
    }
    let Some(cycle) = lane_cycle.as_mut() else {
      continue;
    };
    let Some(lane) = cycle.next_lane(&lanes) else {
      lane_cycle = None;
      continue;
    };

    let lane_budget_ms = cycle.remaining_ms(lane);
    let shared = TurnShared {
      ctx: &ctx,
      pipeline: &pipeline,
      rocks_tip,
      batch_sizer: &batch_sizer,
      prefetch_blocks,
    };
    let elapsed_ms = run_lane_turn(&shared, &mut stage_heights, lane, lane_budget_ms).await?;
    cycle.consume(lane, elapsed_ms);

    lanes = active_lanes(&ctx.enabled_stages, &stage_heights, rocks_tip);
    update_scheduler_lanes(&ctx, &lanes);
  }

  Ok(())
}

/// Rebuild PostgreSQL index stages from the local RocksDB block store, then
/// exit once the selected stages reach the current local tip.
pub(crate) async fn rebuild_from_local_rocks(
  ctx: IndexerLoopCtx,
  batch_sizer: Arc<Mutex<BatchSizer>>,
  prefetch_blocks: u32,
) -> Result<(), DaemonError> {
  let pipeline = ForwardPipeline::new(&ctx);

  if let Ok(mut state) = ctx.scheduler_state.lock() {
    state.enter_loop_phase(wire::msg::SchedulerLoopPhase::FetchRocksTip);
  }
  let rocks_tip = tokio::time::timeout(
    Duration::from_secs(WATCHDOG_OPERATION_TIMEOUT_SECS),
    async {
      let db = Arc::clone(&ctx.db);
      tokio::task::spawn_blocking(move || db::get_tip_height(&db))
        .await
        .map_err(|e| DaemonError::Index(format!("spawn_blocking: {e}")))
    },
  )
  .await
  .map_err(|_| {
    DaemonError::Index(format!(
      "reindex stalled while fetching rocks tip for {}s",
      WATCHDOG_OPERATION_TIMEOUT_SECS
    ))
  })??;

  let Some(rocks_tip) = rocks_tip else {
    if let Ok(mut state) = ctx.scheduler_state.lock() {
      state.set_idle();
    }
    return Err(DaemonError::Index(
      "cannot reindex from local RocksDB: canonical tip is missing".to_owned(),
    ));
  };

  if let Ok(mut state) = ctx.scheduler_state.lock() {
    state.enter_loop_phase(wire::msg::SchedulerLoopPhase::LoadStageHeights);
  }
  let mut stage_heights = tokio::time::timeout(
    Duration::from_secs(WATCHDOG_OPERATION_TIMEOUT_SECS),
    load_stage_heights(&ctx),
  )
  .await
  .map_err(|_| {
    DaemonError::Index(format!(
      "reindex stalled while loading stage heights for {}s",
      WATCHDOG_OPERATION_TIMEOUT_SECS
    ))
  })??;
  let start_height = rebuild_progress_height(&ctx.enabled_stages, &stage_heights);
  tracing::info!(
    start_height,
    rocks_tip,
    stages = %join_stage_names(&ctx.enabled_stages),
    "Starting postgres reindex replay from local RocksDB"
  );
  let rebuild_started = Instant::now();
  let mut last_rebuild_log: Option<(u32, Instant)> = None;

  loop {
    if let Ok(mut state) = ctx.scheduler_state.lock() {
      state.enter_loop_phase(wire::msg::SchedulerLoopPhase::ChooseLane);
    }
    let lanes = active_lanes(&ctx.enabled_stages, &stage_heights, rocks_tip);
    update_scheduler_lanes(&ctx, &lanes);
    let Some(lane) = lanes.first().copied() else {
      if let Ok(mut state) = ctx.scheduler_state.lock() {
        state.set_idle();
      }
      let indexed_height = rebuild_progress_height(&ctx.enabled_stages, &stage_heights);
      tracing::info!(
        indexed_height,
        rocks_tip,
        elapsed_ms = rebuild_started.elapsed().as_millis() as u64,
        stages = %join_stage_names(&ctx.enabled_stages),
        "Finished postgres reindex replay from local RocksDB"
      );
      return Ok(());
    };

    let shared = TurnShared {
      ctx: &ctx,
      pipeline: &pipeline,
      rocks_tip,
      batch_sizer: &batch_sizer,
      prefetch_blocks,
    };
    run_rebuild_lane_turn(&shared, &mut stage_heights, lane).await?;
    let indexed_height = rebuild_progress_height(&ctx.enabled_stages, &stage_heights);
    if indexed_height >= 0 {
      let indexed_height = indexed_height as u32;
      if log_threshold_met(last_rebuild_log, indexed_height, rocks_tip) {
        tracing::info!(
          indexed_height,
          rocks_tip,
          remaining_blocks = rocks_tip.saturating_sub(indexed_height),
          rebuild_pct = rebuild_progress_pct(start_height.max(0) as u32, indexed_height, rocks_tip),
          stages = %join_stage_names(&ctx.enabled_stages),
          "postgres reindex progress from local RocksDB"
        );
        last_rebuild_log = Some((indexed_height, Instant::now()));
      }
    }
  }
}

async fn run_lane_turn(
  shared: &TurnShared<'_>,
  stage_heights: &mut HashMap<Stage, i32>,
  lane: SchedulerLane,
  lane_budget_ms: u64,
) -> Result<u64, DaemonError> {
  if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
    state.begin_turn(lane);
  }
  let turn_started = Instant::now();
  let result = match lane {
    SchedulerLane::Forward => match run_forward_turn(shared, stage_heights, lane_budget_ms).await {
      Ok(()) => Ok(()),
      Err(ForwardTurnError::Interrupted {
        height,
        phase,
        reason,
      }) => {
        tracing::info!(
          lane = "forward",
          height,
          phase = ?phase,
          reason,
          "forward run interrupted during graceful shutdown"
        );
        if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
          state.clear_forward_runtime();
        }
        Ok(())
      }
      Err(ForwardTurnError::Fatal(e)) => Err(e),
      Err(ForwardTurnError::Recoverable {
        height,
        phase,
        reason,
      }) => {
        let attempts = recover_stale_forward_run(shared.ctx, &reason).await?;
        tracing::warn!(
          lane = "forward",
          height,
          phase = ?phase,
          recovery_attempt = attempts,
          max_recovery_attempts = WATCHDOG_RECOVERY_LIMIT,
          reason,
          "forward lane stalled; retrying turn"
        );
        if attempts >= WATCHDOG_RECOVERY_LIMIT {
          Err(DaemonError::Index(format!(
            "forward lane stalled repeatedly at height {height} phase {phase:?}: {reason}"
          )))
        } else {
          Ok(())
        }
      }
    },
    SchedulerLane::TxTreeCatchUp => {
      run_catch_up_turn(
        shared.ctx,
        shared.pipeline,
        stage_heights,
        tx_tree_catch_up_work(stage_heights),
        lane_budget_ms,
      )
      .await
    }
    SchedulerLane::MetricsCatchUp => {
      run_catch_up_turn(
        shared.ctx,
        shared.pipeline,
        stage_heights,
        lane_catch_up_work(stage_heights, SchedulerLane::MetricsCatchUp),
        lane_budget_ms,
      )
      .await
    }
    SchedulerLane::ValidatorCatchUp => {
      run_catch_up_turn(
        shared.ctx,
        shared.pipeline,
        stage_heights,
        lane_catch_up_work(stage_heights, SchedulerLane::ValidatorCatchUp),
        lane_budget_ms,
      )
      .await
    }
  };
  let result = match result {
    Err(error) if shared.ctx.lifecycle.interrupted() && is_pg_cancelled_error(&error) => {
      tracing::info!(
        lane = match lane {
          SchedulerLane::Forward => "forward",
          SchedulerLane::TxTreeCatchUp => "tx_tree_catch_up",
          SchedulerLane::MetricsCatchUp => "metrics_catch_up",
          SchedulerLane::ValidatorCatchUp => "validator_catch_up",
        },
        detail = %error,
        "postgres statement cancelled during graceful shutdown"
      );
      if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
        state.set_idle();
      }
      Ok(())
    }
    Err(error) if is_pg_shutdown_error(&error) => {
      back_off_for_pg_shutdown(shared.ctx, &error).await?;
      Ok(())
    }
    other => other,
  };
  if lane != SchedulerLane::Forward {
    if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
      state.clear_catch_up_runtime();
    }
  }
  let elapsed_ms = turn_started.elapsed().as_millis() as u64;
  if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
    state.finish_turn(lane, elapsed_ms);
  }
  result.map(|_| elapsed_ms)
}

async fn run_rebuild_lane_turn(
  shared: &TurnShared<'_>,
  stage_heights: &mut HashMap<Stage, i32>,
  lane: SchedulerLane,
) -> Result<(), DaemonError> {
  if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
    state.begin_turn(lane);
  }
  let result = match lane {
    SchedulerLane::Forward => match run_forward_turn(shared, stage_heights, u64::MAX / 2).await {
      Ok(()) => Ok(()),
      Err(ForwardTurnError::Interrupted {
        height,
        phase,
        reason,
      }) => Err(DaemonError::Index(format!(
        "reindex interrupted at height {height} phase {phase:?}: {reason}"
      ))),
      Err(ForwardTurnError::Fatal(error)) => Err(error),
      Err(ForwardTurnError::Recoverable {
        height,
        phase,
        reason,
      }) => Err(DaemonError::Index(format!(
        "reindex stalled at height {height} phase {phase:?}: {reason}"
      ))),
    },
    SchedulerLane::TxTreeCatchUp => {
      run_rebuild_catch_up_turn(
        shared.ctx,
        shared.pipeline,
        stage_heights,
        tx_tree_catch_up_work(stage_heights),
      )
      .await
    }
    SchedulerLane::MetricsCatchUp => {
      run_rebuild_catch_up_turn(
        shared.ctx,
        shared.pipeline,
        stage_heights,
        lane_catch_up_work(stage_heights, SchedulerLane::MetricsCatchUp),
      )
      .await
    }
    SchedulerLane::ValidatorCatchUp => {
      run_rebuild_catch_up_turn(
        shared.ctx,
        shared.pipeline,
        stage_heights,
        lane_catch_up_work(stage_heights, SchedulerLane::ValidatorCatchUp),
      )
      .await
    }
  };
  if lane != SchedulerLane::Forward {
    if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
      state.clear_catch_up_runtime();
    }
  }
  if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
    state.finish_turn(lane, 0);
  }
  result
}

fn is_pg_shutdown_error(error: &DaemonError) -> bool {
  let DaemonError::Pg(message) = error else {
    return false;
  };
  let lower = message.to_ascii_lowercase();
  [
    "admin shutdown",
    "terminating connection due to administrator command",
    "server closed the connection unexpectedly",
    "connection refused",
    "connection reset by peer",
    "broken pipe",
    "pool closed",
    "failed to acquire connection from pool",
    "closing down",
    "the database system is shutting down",
    "the database system is starting up",
    "could not connect to server",
  ]
  .iter()
  .any(|needle| lower.contains(needle))
}

fn is_pg_cancelled_error(error: &DaemonError) -> bool {
  let DaemonError::Pg(message) = error else {
    return false;
  };
  let lower = message.to_ascii_lowercase();
  ["canceling statement due to user request", "query canceled"]
    .iter()
    .any(|needle| lower.contains(needle))
}

async fn back_off_for_pg_shutdown(
  ctx: &IndexerLoopCtx,
  error: &DaemonError,
) -> Result<(), DaemonError> {
  tracing::warn!(
    backoff_secs = PG_SHUTDOWN_BACKOFF_SECS,
    detail = %error,
    "postgres is shutting down or unavailable; pausing ingest, flushing rocksdb, and backing off"
  );
  ctx
    .block_worker
    .recover_pg_shutdown(error, PG_SHUTDOWN_BACKOFF_SECS)
    .await
}

async fn recover_stale_forward_run(
  ctx: &IndexerLoopCtx,
  _reason: &str,
) -> Result<u32, DaemonError> {
  let attempts = ctx
    .scheduler_state
    .lock()
    .map(|state| {
      state
        .forward_run_state(ShutdownPhase::Running)
        .recovery_attempts
    })
    .unwrap_or(0)
    .saturating_add(1);
  ctx.block_worker.recover(None).await?;
  Ok(attempts)
}

async fn run_catch_up_turn(
  ctx: &IndexerLoopCtx,
  pipeline: &ForwardPipeline,
  stage_heights: &mut HashMap<Stage, i32>,
  mut work: Option<LaneWork>,
  lane_budget_ms: u64,
) -> Result<(), DaemonError> {
  let started = Instant::now();
  let mut run_id: Option<u64> = None;
  while let Some(item) = work {
    if !ctx.lifecycle.interruption_point() {
      return Ok(());
    }
    if let Ok(mut state) = ctx.scheduler_state.lock() {
      let current_run_id =
        run_id.unwrap_or_else(|| state.start_catch_up_run(item.lane, item.height));
      state.record_catch_up_activity(current_run_id, item.lane, item.height);
      run_id = Some(current_run_id);
    }
    execute_height(
      ctx,
      pipeline,
      item.height,
      &item.active_stages,
      Some(item.lane),
    )
    .await?;
    if let (Some(run_id), Ok(mut state)) = (run_id, ctx.scheduler_state.lock()) {
      state.record_catch_up_progress(run_id, item.lane, item.height);
    }
    update_stage_heights(stage_heights, item.height, &item.active_stages);
    if started.elapsed().as_millis() as u64 >= lane_budget_ms {
      break;
    }
    work = match item.lane {
      SchedulerLane::TxTreeCatchUp => tx_tree_catch_up_work(stage_heights),
      SchedulerLane::MetricsCatchUp | SchedulerLane::ValidatorCatchUp => {
        lane_catch_up_work(stage_heights, item.lane)
      }
      SchedulerLane::Forward => None,
    };
  }
  if let Ok(mut state) = ctx.scheduler_state.lock() {
    state.clear_catch_up_runtime();
  }
  Ok(())
}

async fn run_rebuild_catch_up_turn(
  ctx: &IndexerLoopCtx,
  pipeline: &ForwardPipeline,
  stage_heights: &mut HashMap<Stage, i32>,
  mut work: Option<LaneWork>,
) -> Result<(), DaemonError> {
  let mut run_id: Option<u64> = None;
  while let Some(item) = work {
    if let Ok(mut state) = ctx.scheduler_state.lock() {
      let current_run_id =
        run_id.unwrap_or_else(|| state.start_catch_up_run(item.lane, item.height));
      state.record_catch_up_activity(current_run_id, item.lane, item.height);
      run_id = Some(current_run_id);
    }
    execute_catch_up_height(
      ctx,
      pipeline,
      item.height,
      &item.active_stages,
      Some(item.lane),
      true,
    )
    .await?;
    if let (Some(run_id), Ok(mut state)) = (run_id, ctx.scheduler_state.lock()) {
      state.record_catch_up_progress(run_id, item.lane, item.height);
    }
    update_stage_heights(stage_heights, item.height, &item.active_stages);
    work = match item.lane {
      SchedulerLane::TxTreeCatchUp => tx_tree_catch_up_work(stage_heights),
      SchedulerLane::MetricsCatchUp | SchedulerLane::ValidatorCatchUp => {
        lane_catch_up_work(stage_heights, item.lane)
      }
      SchedulerLane::Forward => None,
    };
  }
  Ok(())
}

async fn run_forward_turn(
  shared: &TurnShared<'_>,
  stage_heights: &mut HashMap<Stage, i32>,
  lane_budget_ms: u64,
) -> Result<(), ForwardTurnError> {
  let indexed = stage_heights.get(&Stage::Block).copied().unwrap_or(-1);
  let start = (indexed + 1) as u32;
  if start > shared.rocks_tip {
    if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
      state.clear_forward_runtime();
    }
    return Ok(());
  }

  let current_batch_size = shared
    .batch_sizer
    .lock()
    .map(|sizer| sizer.current.max(1))
    .unwrap_or(DEFAULT_COMMIT_BATCH_BLOCKS);
  let batch_end = if has_catch_up_work(stage_heights) {
    start
  } else {
    (start + current_batch_size - 1).min(shared.rocks_tip)
  };
  let batch_start_time = Instant::now();
  let mut last_progress_log: Option<Instant> = None;
  let mut last_rebuild_log: Option<(u32, Instant)> = None;

  let Some((mut prefetched_blocks, prefetch_task)) = prefetch_block_ctx_batch(
    Arc::clone(&shared.ctx.lifecycle),
    Arc::clone(&shared.ctx.db),
    start,
    batch_end,
    shared.prefetch_blocks.max(1),
  ) else {
    return Ok(());
  };
  let turn_result = async {
    let mut next_height = start;
    loop {
      check_forward_interrupt(shared.ctx, next_height, None)?;
      let block = match next_forward_block(shared.ctx, next_height, &mut prefetched_blocks).await? {
        Some(block) => block,
        None => break,
      };
      let forward_stages =
        forward_ready_stages(&shared.ctx.enabled_stages, stage_heights, block.height());
      supervise_forward_height(shared, &block, &forward_stages).await?;
      update_stage_heights(stage_heights, block.height(), &forward_stages);

      let should_log_progress = block.height() == shared.rocks_tip
        || last_progress_log.is_none_or(|instant| {
          instant.elapsed() >= Duration::from_secs(BLOCK_PROGRESS_LOG_INTERVAL_SECS)
        });
      if should_log_progress {
        tracing::info!(
          lane = "forward",
          height = block.height(),
          tip = shared.rocks_tip,
          stages = %join_stage_names(&forward_stages),
          "block transition progress"
        );
        last_progress_log = Some(Instant::now());
      }

      if block.height() < shared.rocks_tip {
        let should_log_rebuild =
          log_threshold_met(last_rebuild_log, block.height(), shared.rocks_tip);
        if should_log_rebuild {
          tracing::info!(
            indexed_height = block.height(),
            rocks_tip = shared.rocks_tip,
            remaining_blocks = shared.rocks_tip.saturating_sub(block.height()),
            rebuild_pct = rebuild_progress_pct(start, block.height(), shared.rocks_tip),
            "postgres rebuild progress from local RocksDB"
          );
          last_rebuild_log = Some((block.height(), Instant::now()));
        }
      }

      let _ = shared
        .ctx
        .event_tx
        .send(wire::msg::DaemonMessage::IndexProgress {
          stage: Stage::Block.name().to_string(),
          height: block.height(),
          tip: shared.rocks_tip,
        });

      next_height = block.height() + 1;
      if batch_start_time.elapsed().as_millis() as u64 >= lane_budget_ms {
        break;
      }
    }
    Ok::<(), ForwardTurnError>(())
  }
  .await;

  match turn_result {
    Ok(()) => {
      // Stop the producer before awaiting it. Otherwise a budget-limited turn can
      // leave unread prefetched blocks in the bounded channel and block the
      // prefetch thread forever on `blocking_send(...)`.
      drop(prefetched_blocks);
      prefetch_task
        .await
        .map_err(|e| DaemonError::Index(format!("prefetch task: {e}")))?
        .map_err(ForwardTurnError::Fatal)?;
    }
    Err(err) => {
      prefetch_task.abort();
      let _ = prefetch_task.await;
      return Err(err);
    }
  }

  let elapsed_ms = batch_start_time.elapsed().as_millis() as u64;
  if let Ok(mut sizer) = shared.batch_sizer.lock() {
    sizer.adjust(elapsed_ms, Stage::Block.name());
  }

  Ok(())
}

async fn next_forward_block(
  ctx: &IndexerLoopCtx,
  height: u32,
  prefetched_blocks: &mut PrefetchReceiver,
) -> Result<Option<BlockCtx>, ForwardTurnError> {
  let mut shutdown = ctx.lifecycle.subscribe();
  tokio::select! {
    prefetched = prefetched_blocks.recv() => {
      match prefetched {
        Some(prefetched) => prefetched.map(Some).map_err(ForwardTurnError::Fatal),
        None => Ok(None),
      }
    }
    _ = shutdown.changed() => Err(ForwardTurnError::Interrupted {
      height,
      phase: Some(wire::msg::ForwardLanePhase::AcquireChainState),
      reason: "forward run interrupted".to_owned(),
    }),
  }
}

async fn execute_forward_height(
  ctx: &IndexerLoopCtx,
  pipeline: &ForwardPipeline,
  block: &BlockCtx,
  active_stages: &[Stage],
  reporter: &ForwardRunReporter,
) -> Result<(), ForwardTurnError> {
  let height = block.height();
  check_forward_interrupt(ctx, height, None)?;
  let _chain_guard = run_forward_phase(
    ctx,
    height,
    wire::msg::ForwardLanePhase::AcquireChainState,
    active_stages,
    reporter,
    async {
      let mut shutdown = ctx.lifecycle.subscribe();
      tokio::select! {
        guard = acquire_chain_state_read(ctx, height, "forward") => {
          guard
        }
        _ = shutdown.changed() => {
          Err(DaemonError::Index("forward run interrupted".to_owned()))
        }
      }
    },
  )
  .await?;

  tracing::debug!(
    lane = "forward",
    height,
    stages = %join_stage_names(active_stages),
    "forward turn start"
  );

  check_forward_interrupt(ctx, height, Some(wire::msg::ForwardLanePhase::BeginTx))?;
  let mut pg_tx = run_forward_phase(
    ctx,
    height,
    wire::msg::ForwardLanePhase::BeginTx,
    active_stages,
    reporter,
    async {
      let mut pg_tx = ctx
        .pool
        .begin()
        .await
        .map_err(|e| DaemonError::Pg(format!("forward begin: {e}")))?;
      crate::sql::query::set_local_async_commit(&mut pg_tx).await?;
      Ok(pg_tx)
    },
  )
  .await?;

  check_forward_interrupt(ctx, height, Some(wire::msg::ForwardLanePhase::PlanConnect))?;
  let transition = run_forward_phase(
    ctx,
    height,
    wire::msg::ForwardLanePhase::PlanConnect,
    active_stages,
    reporter,
    async {
      pipeline
        .plan_connect_exact(&mut pg_tx, block, active_stages, Some(reporter))
        .await
    },
  )
  .await?;

  check_forward_interrupt(
    ctx,
    height,
    Some(wire::msg::ForwardLanePhase::ApplyStagePlan),
  )?;
  run_forward_phase(
    ctx,
    height,
    wire::msg::ForwardLanePhase::ApplyStagePlan,
    active_stages,
    reporter,
    async { transition.apply(pipeline, &mut pg_tx, Some(reporter)).await },
  )
  .await?;

  check_forward_interrupt(ctx, height, Some(wire::msg::ForwardLanePhase::CommitTx))?;
  run_forward_phase(
    ctx,
    height,
    wire::msg::ForwardLanePhase::CommitTx,
    active_stages,
    reporter,
    async {
      pg_tx
        .commit()
        .await
        .map_err(|e| DaemonError::Pg(format!("forward COMMIT: {e}")))
    },
  )
  .await?;

  tracing::debug!(
    lane = "forward",
    height,
    stages = %join_stage_names(active_stages),
    "forward turn complete"
  );

  for stage in active_stages.iter().copied() {
    let _ = ctx.event_tx.send(wire::msg::DaemonMessage::IndexProgress {
      stage: stage.name().to_string(),
      height,
      tip: height,
    });
  }

  Ok(())
}

async fn supervise_forward_height(
  shared: &TurnShared<'_>,
  block: &BlockCtx,
  active_stages: &[Stage],
) -> Result<(), ForwardTurnError> {
  let height = block.height();
  let run_id = shared
    .ctx
    .scheduler_state
    .lock()
    .map(|mut state| state.start_forward_run(height))
    .unwrap_or(1);
  let phase = Arc::new(std::sync::Mutex::new(None::<wire::msg::ForwardLanePhase>));
  let (event_tx, mut event_rx) = tokio::sync::mpsc::unbounded_channel();
  let reporter = ForwardRunReporter::new(
    run_id,
    Arc::clone(&shared.ctx.lifecycle),
    Arc::clone(&shared.ctx.operator_pause_requested),
    event_tx,
  );
  let child_ctx = shared.ctx.clone();
  let child_block = block.clone();
  let child_stages = active_stages.to_vec();
  let child = AbortOnDropTask::new(tokio::spawn(async move {
    let pipeline = ForwardPipeline::new(&child_ctx);
    execute_forward_height(
      &child_ctx,
      &pipeline,
      &child_block,
      &child_stages,
      &reporter,
    )
    .await
  }));
  tokio::pin!(child);

  loop {
    tokio::select! {
      maybe_event = event_rx.recv() => {
        let Some(event) = maybe_event else {
          continue;
        };
        if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
          match event {
            ForwardRunEvent::Phase { run_id, height, phase: event_phase } => {
              state.record_forward_phase_event(run_id, height, event_phase);
              if let Ok(mut phase_guard) = phase.lock() {
                *phase_guard = Some(event_phase);
              }
            }
            ForwardRunEvent::Stage { run_id, stage } => {
              state.record_forward_stage_event(run_id, stage);
            }
            ForwardRunEvent::Activity {
              run_id,
              substage,
              chunk_index,
              chunk_total,
            } => {
              state.record_forward_activity_event(run_id, substage, chunk_index, chunk_total);
            }
            ForwardRunEvent::Progress {
              run_id,
              substage,
              chunk_index,
              chunk_total,
            } => {
              state.record_forward_progress_event(run_id, substage, chunk_index, chunk_total);
            }
          }
        }
      }
      join_result = &mut child => {
        match join_result {
          Ok(Ok(())) => {
            if let Ok(mut state) = shared.ctx.scheduler_state.lock() {
              state.complete_forward_height_for_run(run_id, height);
            }
            return Ok(());
          }
          Ok(Err(error)) => return Err(error),
          Err(join_error) => {
            return Err(ForwardTurnError::Fatal(DaemonError::Index(format!(
              "forward child task join: {join_error}"
            ))));
          }
        }
      }
    }
  }
}

fn is_forward_interrupt_error(error: &DaemonError) -> bool {
  matches!(error, DaemonError::Index(message) if message == "forward run interrupted")
}

fn is_catch_up_interrupt_error(error: &DaemonError) -> bool {
  matches!(error, DaemonError::Index(message) if message == "catch-up run interrupted")
}

async fn run_forward_phase<T, F>(
  _ctx: &IndexerLoopCtx,
  height: u32,
  phase: wire::msg::ForwardLanePhase,
  active_stages: &[Stage],
  reporter: &ForwardRunReporter,
  future: F,
) -> Result<T, ForwardTurnError>
where
  F: Future<Output = Result<T, DaemonError>>,
{
  reporter.phase(height, phase);
  tracing::debug!(
    lane = "forward",
    height,
    phase = ?phase,
    stages = %join_stage_names(active_stages),
    "forward phase start"
  );
  let started = Instant::now();
  let result =
    tokio::time::timeout(Duration::from_secs(WATCHDOG_OPERATION_TIMEOUT_SECS), future).await;
  let elapsed_ms = started.elapsed().as_millis() as u64;

  match result {
    Ok(Ok(value)) => {
      if elapsed_ms >= WATCHDOG_SLOW_OPERATION_MS {
        tracing::warn!(
          lane = "forward",
          height,
          phase = ?phase,
          elapsed_ms,
          stages = %join_stage_names(active_stages),
          "forward phase is slow"
        );
      }
      tracing::debug!(
        lane = "forward",
        height,
        phase = ?phase,
        elapsed_ms,
        "forward phase complete"
      );
      Ok(value)
    }
    Ok(Err(err)) => {
      if is_forward_interrupt_error(&err) {
        Err(ForwardTurnError::Interrupted {
          height,
          phase: Some(phase),
          reason: "forward run interrupted".to_owned(),
        })
      } else {
        Err(ForwardTurnError::Fatal(err))
      }
    }
    Err(_) => Err(ForwardTurnError::Recoverable {
      height,
      phase,
      reason: format!("timed out after {}s", WATCHDOG_OPERATION_TIMEOUT_SECS),
    }),
  }
}

fn check_forward_interrupt(
  ctx: &IndexerLoopCtx,
  height: u32,
  phase: Option<wire::msg::ForwardLanePhase>,
) -> Result<(), ForwardTurnError> {
  if ctx.lifecycle.interruption_point() && !ctx.operator_pause_requested.load(Ordering::Relaxed) {
    return Ok(());
  }
  Err(ForwardTurnError::Interrupted {
    height,
    phase,
    reason: "forward run interrupted".to_owned(),
  })
}

fn join_stage_names(stages: &[Stage]) -> String {
  if stages.is_empty() {
    "-".to_owned()
  } else {
    stages
      .iter()
      .copied()
      .map(Stage::name)
      .collect::<Vec<_>>()
      .join(",")
  }
}

async fn execute_height(
  ctx: &IndexerLoopCtx,
  pipeline: &ForwardPipeline,
  height: u32,
  active_stages: &[Stage],
  lane: Option<SchedulerLane>,
) -> Result<(), DaemonError> {
  if lane != Some(SchedulerLane::Forward) {
    return execute_catch_up_height(ctx, pipeline, height, active_stages, lane, false).await;
  }

  execute_height_inner(ctx, pipeline, height, active_stages, lane, false).await
}

async fn execute_catch_up_height(
  ctx: &IndexerLoopCtx,
  pipeline: &ForwardPipeline,
  height: u32,
  active_stages: &[Stage],
  lane: Option<SchedulerLane>,
  skip_chain_lock: bool,
) -> Result<(), DaemonError> {
  let lane_name = lane.map(lane_name).unwrap_or("catch_up");
  tracing::debug!(
    lane = lane_name,
    height,
    stages = %join_stage_names(active_stages),
    "catch-up turn start"
  );
  let started = Instant::now();
  let result = tokio::time::timeout(
    Duration::from_secs(WATCHDOG_OPERATION_TIMEOUT_SECS),
    execute_height_inner(ctx, pipeline, height, active_stages, lane, skip_chain_lock),
  )
  .await;
  let elapsed_ms = started.elapsed().as_millis() as u64;
  match result {
    Ok(Ok(())) => {
      if elapsed_ms >= WATCHDOG_SLOW_OPERATION_MS {
        tracing::warn!(
          lane = lane_name,
          height,
          elapsed_ms,
          stages = %join_stage_names(active_stages),
          "catch-up turn is slow"
        );
      }
      tracing::debug!(
        lane = lane_name,
        height,
        elapsed_ms,
        "catch-up turn complete"
      );
      Ok(())
    }
    Ok(Err(err)) if is_catch_up_interrupt_error(&err) => Ok(()),
    Ok(Err(err)) => Err(err),
    Err(_) => Err(DaemonError::Index(format!(
      "{lane_name} stalled at height {height} for {}s",
      WATCHDOG_OPERATION_TIMEOUT_SECS
    ))),
  }
}

async fn execute_height_inner(
  ctx: &IndexerLoopCtx,
  pipeline: &ForwardPipeline,
  height: u32,
  active_stages: &[Stage],
  lane: Option<SchedulerLane>,
  skip_chain_lock: bool,
) -> Result<(), DaemonError> {
  let mut shutdown = ctx.lifecycle.subscribe();
  let lane_label = lane.map(lane_name).unwrap_or("catch_up");
  tracing::debug!(
    lane = lane_label,
    height,
    step = "acquire_chain_state",
    "catch-up step start"
  );
  let acquire_started = Instant::now();
  let _chain_guard = if skip_chain_lock {
    tracing::debug!(
      lane = lane_label,
      height,
      step = "acquire_chain_state",
      "catch-up step skipped"
    );
    None
  } else {
    let guard = tokio::select! {
      guard = acquire_chain_state_read(ctx, height, lane_label) => guard?,
      _ = shutdown.changed() => {
        return Err(DaemonError::Index("catch-up run interrupted".to_owned()));
      }
    };
    let acquire_elapsed_ms = acquire_started.elapsed().as_millis() as u64;
    log_catch_up_step_complete(
      lane_label,
      height,
      "acquire_chain_state",
      acquire_elapsed_ms,
      active_stages,
    );
    Some(guard)
  };
  tracing::debug!(
    lane = lane_label,
    height,
    step = "fetch_block",
    "catch-up step start"
  );
  let fetch_started = Instant::now();
  let block = fetch_block_ctx(Arc::clone(&ctx.db), height).await?;
  let fetch_elapsed_ms = fetch_started.elapsed().as_millis() as u64;
  log_catch_up_step_complete(
    lane_label,
    height,
    "fetch_block",
    fetch_elapsed_ms,
    active_stages,
  );
  tracing::debug!(
    lane = lane_label,
    height,
    step = "begin_tx",
    "catch-up step start"
  );
  let begin_started = Instant::now();
  let mut pg_tx = ctx
    .pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("catch-up begin: {e}")))?;

  crate::sql::query::set_local_async_commit(&mut pg_tx).await?;
  let begin_elapsed_ms = begin_started.elapsed().as_millis() as u64;
  log_catch_up_step_complete(
    lane_label,
    height,
    "begin_tx",
    begin_elapsed_ms,
    active_stages,
  );

  tracing::debug!(
    lane = lane_label,
    height,
    step = "plan_connect",
    "catch-up step start"
  );
  let plan_started = Instant::now();
  let transition = pipeline
    .plan_connect_exact(&mut pg_tx, &block, active_stages, None)
    .await?;
  let plan_elapsed_ms = plan_started.elapsed().as_millis() as u64;
  log_catch_up_step_complete(
    lane_label,
    height,
    "plan_connect",
    plan_elapsed_ms,
    active_stages,
  );
  tracing::debug!(
    lane = lane_label,
    height,
    step = "apply_stage_plan",
    "catch-up step start"
  );
  let apply_started = Instant::now();
  transition.apply(pipeline, &mut pg_tx, None).await?;
  let apply_elapsed_ms = apply_started.elapsed().as_millis() as u64;
  log_catch_up_step_complete(
    lane_label,
    height,
    "apply_stage_plan",
    apply_elapsed_ms,
    active_stages,
  );

  tracing::debug!(
    lane = lane_label,
    height,
    step = "commit_tx",
    "catch-up step start"
  );
  let commit_started = Instant::now();
  pg_tx
    .commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("catch-up COMMIT: {e}")))?;
  let commit_elapsed_ms = commit_started.elapsed().as_millis() as u64;
  log_catch_up_step_complete(
    lane_label,
    height,
    "commit_tx",
    commit_elapsed_ms,
    active_stages,
  );

  if lane != Some(SchedulerLane::Forward) && !skip_chain_lock {
    let stage_names: Vec<&str> = active_stages.iter().copied().map(Stage::name).collect();
    tracing::info!(
      height,
      stages = %stage_names.join(","),
      "stage catch-up progress"
    );
  }

  for stage in active_stages.iter().copied() {
    let _ = ctx.event_tx.send(wire::msg::DaemonMessage::IndexProgress {
      stage: stage.name().to_string(),
      height,
      tip: height,
    });
  }

  Ok(())
}

fn lane_name(lane: SchedulerLane) -> &'static str {
  match lane {
    SchedulerLane::Forward => "forward",
    SchedulerLane::TxTreeCatchUp => "tx_tree_catch_up",
    SchedulerLane::MetricsCatchUp => "metrics_catch_up",
    SchedulerLane::ValidatorCatchUp => "validator_catch_up",
  }
}

fn log_catch_up_step_complete(
  lane: &str,
  height: u32,
  step: &'static str,
  elapsed_ms: u64,
  active_stages: &[Stage],
) {
  if elapsed_ms >= WATCHDOG_SLOW_OPERATION_MS {
    tracing::warn!(
      lane,
      height,
      step,
      elapsed_ms,
      stages = %join_stage_names(active_stages),
      "catch-up step is slow"
    );
  }
  tracing::debug!(
    lane,
    height,
    step,
    elapsed_ms,
    stages = %join_stage_names(active_stages),
    "catch-up step complete"
  );
}

fn forward_phase_name(phase: wire::msg::ForwardLanePhase) -> &'static str {
  match phase {
    wire::msg::ForwardLanePhase::Idle => "Idle",
    wire::msg::ForwardLanePhase::AcquireChainState => "AcquireChainState",
    wire::msg::ForwardLanePhase::FetchBlock => "FetchBlock",
    wire::msg::ForwardLanePhase::BeginTx => "BeginTx",
    wire::msg::ForwardLanePhase::PlanConnect => "PlanConnect",
    wire::msg::ForwardLanePhase::ApplyStagePlan => "ApplyStagePlan",
    wire::msg::ForwardLanePhase::CommitTx => "CommitTx",
  }
}

async fn load_stage_heights(ctx: &IndexerLoopCtx) -> Result<HashMap<Stage, i32>, DaemonError> {
  let mut heights = HashMap::new();
  heights.insert(
    Stage::Block,
    crate::sql::query::indexed_stage_height(&ctx.pool, Stage::Block).await?,
  );
  for stage in ctx.enabled_stages.iter().copied() {
    heights.insert(
      stage,
      crate::sql::query::indexed_stage_height(&ctx.pool, stage).await?,
    );
  }
  Ok(heights)
}

fn rebuild_progress_height(enabled_stages: &[Stage], stage_heights: &HashMap<Stage, i32>) -> i32 {
  if enabled_stages.is_empty() {
    return stage_heights.get(&Stage::Block).copied().unwrap_or(-1);
  }
  enabled_stages
    .iter()
    .filter_map(|stage| stage_heights.get(stage).copied())
    .min()
    .unwrap_or(-1)
}

fn next_lagging_height(
  enabled_stages: &[Stage],
  stage_heights: &HashMap<Stage, i32>,
) -> Option<(u32, Vec<Stage>)> {
  let mut next_height: Option<i32> = None;
  for stage in enabled_stages.iter().copied() {
    let indexed = stage_heights.get(&stage).copied().unwrap_or(-1);
    let gate_height = stage
      .gate_stage()
      .and_then(|gate| stage_heights.get(&gate).copied())?;
    if indexed < gate_height {
      let stage_next = indexed + 1;
      next_height = Some(next_height.map_or(stage_next, |current| current.min(stage_next)));
    }
  }

  let next_height = next_height?;
  let candidates: Vec<Stage> = enabled_stages
    .iter()
    .copied()
    .filter(|stage| stage_heights.get(stage).copied().unwrap_or(-1) + 1 == next_height)
    .collect();
  let active_stages: Vec<Stage> = candidates
    .iter()
    .copied()
    .filter(|stage| {
      stage.gate_stage().is_none_or(|gate| {
        gate == Stage::Block
          || stage_heights.get(&gate).copied().unwrap_or(-1) >= next_height
          || candidates.contains(&gate)
      })
    })
    .collect();

  if active_stages.is_empty() {
    return None;
  }

  Some((next_height as u32, active_stages))
}

fn tx_tree_catch_up_work(stage_heights: &HashMap<Stage, i32>) -> Option<LaneWork> {
  let stages = [Stage::Transaction, Stage::Script, Stage::Spent];
  let (height, active_stages) = next_lagging_height(&stages, stage_heights)?;
  Some(LaneWork {
    lane: SchedulerLane::TxTreeCatchUp,
    height,
    active_stages,
  })
}

fn lane_catch_up_work(
  stage_heights: &HashMap<Stage, i32>,
  lane: SchedulerLane,
) -> Option<LaneWork> {
  let stages: &[Stage] = match lane {
    SchedulerLane::MetricsCatchUp => &[Stage::Metrics],
    SchedulerLane::ValidatorCatchUp => &[Stage::Validator],
    _ => return None,
  };
  let (height, active_stages) = next_lagging_height(stages, stage_heights)?;
  Some(LaneWork {
    lane,
    height,
    active_stages,
  })
}

fn active_lanes(
  enabled_stages: &[Stage],
  stage_heights: &HashMap<Stage, i32>,
  rocks_tip: u32,
) -> Vec<SchedulerLane> {
  let mut lanes = Vec::new();
  let indexed = stage_heights.get(&Stage::Block).copied().unwrap_or(-1);
  if (indexed + 1) as u32 <= rocks_tip {
    lanes.push(SchedulerLane::Forward);
  }
  if tx_tree_catch_up_work(stage_heights).is_some() && enabled_stages.contains(&Stage::Transaction)
  {
    lanes.push(SchedulerLane::TxTreeCatchUp);
  }
  if lane_catch_up_work(stage_heights, SchedulerLane::MetricsCatchUp).is_some()
    && enabled_stages.contains(&Stage::Metrics)
  {
    lanes.push(SchedulerLane::MetricsCatchUp);
  }
  if lane_catch_up_work(stage_heights, SchedulerLane::ValidatorCatchUp).is_some()
    && enabled_stages.contains(&Stage::Validator)
  {
    lanes.push(SchedulerLane::ValidatorCatchUp);
  }
  lanes
}

fn has_catch_up_work(stage_heights: &HashMap<Stage, i32>) -> bool {
  tx_tree_catch_up_work(stage_heights).is_some()
    || lane_catch_up_work(stage_heights, SchedulerLane::MetricsCatchUp).is_some()
    || lane_catch_up_work(stage_heights, SchedulerLane::ValidatorCatchUp).is_some()
}

fn update_scheduler_lanes(ctx: &IndexerLoopCtx, lanes: &[SchedulerLane]) {
  if let Ok(mut state) = ctx.scheduler_state.lock() {
    if lanes.is_empty() {
      state.set_idle();
    } else {
      state.set_active_lanes(lanes);
    }
  }
}

fn forward_ready_stages(
  enabled_stages: &[Stage],
  stage_heights: &HashMap<Stage, i32>,
  next_block_height: u32,
) -> Vec<Stage> {
  let mut ready = vec![Stage::Block];
  let next_height = next_block_height as i32;
  for stage in enabled_stages.iter().copied() {
    let indexed = stage_heights.get(&stage).copied().unwrap_or(-1);
    if indexed + 1 != next_height {
      continue;
    }
    let gate_ready = stage.gate_stage().is_none_or(|gate| {
      gate == Stage::Block
        || ready.contains(&gate)
        || stage_heights.get(&gate).copied().unwrap_or(-1) >= next_height
    });
    if gate_ready {
      ready.push(stage);
    }
  }
  ready
}

fn update_stage_heights(stage_heights: &mut HashMap<Stage, i32>, height: u32, stages: &[Stage]) {
  for stage in stages {
    stage_heights.insert(*stage, height as i32);
  }
}

fn lane_sort_key(lane: wire::msg::IndexSchedulerLane) -> u8 {
  match lane {
    wire::msg::IndexSchedulerLane::Idle => 0,
    wire::msg::IndexSchedulerLane::Forward => 1,
    wire::msg::IndexSchedulerLane::TxTreeCatchUp => 2,
    wire::msg::IndexSchedulerLane::MetricsCatchUp => 3,
    wire::msg::IndexSchedulerLane::ValidatorCatchUp => 4,
  }
}

async fn fetch_block_ctx(db: Arc<rocksdb::DB>, height: u32) -> Result<BlockCtx, DaemonError> {
  tokio::task::spawn_blocking(move || {
    let cf = db
      .cf_handle(db::CF_BLOCKS)
      .ok_or_else(|| DaemonError::Db("blocks CF missing".to_owned()))?;
    let heights_cf = db
      .cf_handle(db::CF_HEIGHTS)
      .ok_or_else(|| DaemonError::Db("heights CF missing".to_owned()))?;
    let hash = db
      .get_cf(heights_cf, height.to_be_bytes())
      .map_err(|e| DaemonError::Db(e.to_string()))?
      .ok_or_else(|| DaemonError::Index(format!("height {height} not in RocksDB")))?;
    let raw = db
      .get_cf(cf, &hash)
      .map_err(|e| DaemonError::Db(e.to_string()))?
      .ok_or_else(|| DaemonError::Index(format!("block at {height} not in RocksDB")))?;
    BlockCtx::from_raw(height, raw)
  })
  .await
  .map_err(|e| DaemonError::Index(format!("spawn_blocking: {e}")))?
}

fn prefetch_block_ctx_batch(
  lifecycle: Arc<crate::lifecycle::Lifecycle>,
  db: Arc<rocksdb::DB>,
  start: u32,
  end: u32,
  capacity: u32,
) -> Option<(PrefetchReceiver, PrefetchTask)> {
  if !lifecycle.interruption_point() {
    return None;
  }
  let (tx, rx) = tokio::sync::mpsc::channel(capacity as usize);
  let task = tokio::task::spawn_blocking(move || {
    let cf = db
      .cf_handle(db::CF_BLOCKS)
      .ok_or_else(|| DaemonError::Db("blocks CF missing".to_owned()))?;
    let heights_cf = db
      .cf_handle(db::CF_HEIGHTS)
      .ok_or_else(|| DaemonError::Db("heights CF missing".to_owned()))?;

    for height in start..=end {
      if !lifecycle.interruption_point() {
        return Ok(());
      }
      let hash = db
        .get_cf(heights_cf, height.to_be_bytes())
        .map_err(|e| DaemonError::Db(e.to_string()))?
        .ok_or_else(|| DaemonError::Index(format!("height {height} not in RocksDB")))?;
      let raw = db
        .get_cf(cf, &hash)
        .map_err(|e| DaemonError::Db(e.to_string()))?
        .ok_or_else(|| DaemonError::Index(format!("block at {height} not in RocksDB")))?;
      let block = BlockCtx::from_raw(height, raw);
      if tx.blocking_send(block).is_err() {
        return Ok(());
      }
    }

    Ok::<_, DaemonError>(())
  });
  Some((rx, task))
}

async fn wait_if_block_paused(handle: &mut IndexerHandle) -> bool {
  if *handle.pause_rx.borrow() {
    handle.idle_tx.send_replace(true);
    let mut rx = handle.pause_rx.clone();
    loop {
      tokio::select! {
        r = rx.changed() => {
          if r.is_err() {
            handle.idle_tx.send_replace(false);
            return true;
          }
          if !*rx.borrow_and_update() {
            break;
          }
          handle.idle_tx.send_replace(true);
        }
        _ = tokio::time::sleep(Duration::from_secs(BLOCK_PAUSE_POLL_SECS)) => {
          handle.idle_tx.send_replace(true);
        }
      }
    }
    handle.idle_tx.send_replace(false);
    true
  } else {
    false
  }
}

#[cfg(test)]
mod tests {
  use std::collections::HashMap;

  use super::{
    forward_ready_stages, is_pg_cancelled_error, is_pg_shutdown_error, next_lagging_height,
    now_epoch, ApplyStagePlanSubstage, IndexSchedulerState, SchedulerLane,
  };
  use crate::error::DaemonError;
  use crate::index::Stage;
  use crate::policy::watchdog::{
    WATCHDOG_ACTIVITY_STALE_SECS, WATCHDOG_PROGRESS_STALE_SECS, WATCHDOG_RECOVERY_LIMIT,
  };
  use crate::visor::{ProgressState, RecoveryDisposition, ShutdownPhase};
  use hyphend::wire;

  #[test]
  fn validator_only_reset_gets_its_own_catch_up_height() {
    let mut heights = HashMap::new();
    heights.insert(Stage::Block, 100);
    heights.insert(Stage::Transaction, 100);
    heights.insert(Stage::Metrics, 100);
    heights.insert(Stage::Validator, -1);

    let next = next_lagging_height(
      &[Stage::Transaction, Stage::Metrics, Stage::Validator],
      &heights,
    );

    assert_eq!(next, Some((0, vec![Stage::Validator])));
  }

  #[test]
  fn transaction_tree_reset_replays_in_dependency_order_per_height() {
    let mut heights = HashMap::new();
    heights.insert(Stage::Block, 25);
    heights.insert(Stage::Transaction, -1);
    heights.insert(Stage::Script, -1);
    heights.insert(Stage::Spent, -1);

    let next = next_lagging_height(&[Stage::Transaction, Stage::Script, Stage::Spent], &heights);

    assert_eq!(
      next,
      Some((0, vec![Stage::Transaction, Stage::Script, Stage::Spent]))
    );
  }

  #[test]
  fn forward_ready_stages_excludes_lagging_validator_during_ibd() {
    let mut heights = HashMap::new();
    heights.insert(Stage::Block, 100);
    heights.insert(Stage::Transaction, 100);
    heights.insert(Stage::Metrics, 100);
    heights.insert(Stage::Validator, -1);

    let ready = forward_ready_stages(
      &[Stage::Transaction, Stage::Metrics, Stage::Validator],
      &heights,
      101,
    );

    assert_eq!(
      ready,
      vec![Stage::Block, Stage::Transaction, Stage::Metrics]
    );
  }

  #[test]
  fn pg_shutdown_classifier_matches_shutdown_and_disconnect_errors() {
    assert!(is_pg_shutdown_error(&DaemonError::Pg(
      "forward begin: error returned from database: terminating connection due to administrator command"
        .to_owned(),
    )));
    assert!(is_pg_shutdown_error(&DaemonError::Pg(
      "forward COMMIT: pool closed".to_owned(),
    )));
    assert!(!is_pg_shutdown_error(&DaemonError::Pg(
      "forward COMMIT: duplicate key value violates unique constraint".to_owned(),
    )));
    assert!(!is_pg_shutdown_error(&DaemonError::Index(
      "not a postgres error".to_owned(),
    )));
  }

  #[test]
  fn pg_cancel_classifier_matches_statement_cancellation_errors() {
    assert!(is_pg_cancelled_error(&DaemonError::Pg(
      "forward apply: error returned from database: canceling statement due to user request"
        .to_owned(),
    )));
    assert!(is_pg_cancelled_error(&DaemonError::Pg(
      "forward apply: query canceled".to_owned(),
    )));
    assert!(!is_pg_cancelled_error(&DaemonError::Pg(
      "forward apply: terminating connection due to administrator command".to_owned(),
    )));
    assert!(!is_pg_cancelled_error(&DaemonError::Index(
      "not a postgres error".to_owned(),
    )));
  }

  #[test]
  fn forward_run_events_ignore_stale_run_ids() {
    let mut state = IndexSchedulerState::default();
    let stale_run = state.start_forward_run(7);
    let active_run = state.start_forward_run(8);

    state.record_forward_activity_event(
      stale_run,
      ApplyStagePlanSubstage::StageStart.into(),
      Some(9),
      Some(9),
    );

    assert_eq!(state.forward.run_id, Some(active_run));
    assert_eq!(state.forward.height, Some(8));
    assert_eq!(state.forward.substage, None);
    assert_eq!(state.forward.chunk_index, None);

    state.record_forward_activity_event(
      active_run,
      ApplyStagePlanSubstage::SpentAddressStats.into(),
      Some(1),
      Some(3),
    );

    assert_eq!(
      state.forward.substage,
      Some(ApplyStagePlanSubstage::SpentAddressStats.into())
    );
    assert_eq!(state.forward.chunk_index, Some(1));
    assert_eq!(state.forward.chunk_total, Some(3));
  }

  #[tokio::test]
  async fn abort_on_drop_task_aborts_child_when_owner_drops() {
    #[derive(Clone)]
    struct DropSignal(std::sync::Arc<std::sync::atomic::AtomicBool>);

    impl Drop for DropSignal {
      fn drop(&mut self) {
        self.0.store(true, std::sync::atomic::Ordering::SeqCst);
      }
    }

    let dropped = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
    let signal = DropSignal(std::sync::Arc::clone(&dropped));
    let (tx, rx) = tokio::sync::oneshot::channel();
    let task = super::AbortOnDropTask::new(tokio::spawn(async move {
      let guard = signal;
      let _ = tx.send(());
      tokio::time::sleep(std::time::Duration::from_secs(60)).await;
      drop(guard);
    }));

    let _ = rx.await;
    drop(task);

    tokio::time::timeout(std::time::Duration::from_secs(1), async {
      while !dropped.load(std::sync::atomic::Ordering::SeqCst) {
        tokio::task::yield_now().await;
      }
    })
    .await
    .expect("child task should be aborted when owner drops");
  }

  #[test]
  fn forward_run_state_maps_runtime_to_supervision_contract() {
    let mut state = IndexSchedulerState::default();
    let run_id = state.start_forward_run(42);
    let provisional_state = state.forward_run_state(ShutdownPhase::Running);
    assert_eq!(provisional_state.run_id, Some(run_id));
    assert_eq!(provisional_state.phase, "-");
    assert_eq!(provisional_state.progress_state, ProgressState::Starting);
    assert!(provisional_state.last_activity_epoch.is_some());
    assert_eq!(provisional_state.last_progress_epoch, None);

    state.record_forward_phase_event(run_id, 42, wire::msg::ForwardLanePhase::AcquireChainState);
    assert_eq!(state.forward.run_id, Some(run_id));
    state.record_forward_progress_event(
      run_id,
      ApplyStagePlanSubstage::SpentAddressStats.into(),
      Some(2),
      Some(4),
    );

    let run_state = state.forward_run_state(ShutdownPhase::Running);

    assert_eq!(run_state.run_id, Some(run_id));
    assert_eq!(run_state.phase, "AcquireChainState");
    assert_eq!(run_state.subphase, None);
    assert_eq!(run_state.detail, Some("spent_address_stats"));
    assert_eq!(run_state.progress_state, ProgressState::Active);
    assert_eq!(run_state.shutdown_phase, ShutdownPhase::Running);
    assert!(run_state.last_activity_epoch.is_some());
    assert!(run_state.last_progress_epoch.is_some());
  }

  #[test]
  fn forward_run_state_surfaces_stage_and_typed_substage_detail() {
    let mut state = IndexSchedulerState::default();
    let run_id = state.start_forward_run(42);

    state.record_forward_phase_event(run_id, 42, wire::msg::ForwardLanePhase::ApplyStagePlan);
    state.record_forward_stage_event(run_id, Stage::Spent);
    state.record_forward_progress_event(
      run_id,
      ApplyStagePlanSubstage::SpentMarkOutputs.into(),
      Some(3),
      Some(9),
    );

    let run_state = state.forward_run_state(ShutdownPhase::Running);

    assert_eq!(run_state.phase, "ApplyStagePlan");
    assert_eq!(run_state.subphase, Some("spent"));
    assert_eq!(run_state.detail, Some("spent_mark_outputs"));
  }

  #[test]
  fn active_run_state_surfaces_catch_up_lane_work() {
    let mut state = IndexSchedulerState::default();
    let run_id = state.start_catch_up_run(SchedulerLane::MetricsCatchUp, 55);
    state.record_catch_up_activity(run_id, SchedulerLane::MetricsCatchUp, 55);
    state.record_catch_up_progress(run_id, SchedulerLane::MetricsCatchUp, 55);

    let run_state = state.active_run_state(ShutdownPhase::Running);

    assert_eq!(run_state.run_id, Some(run_id));
    assert_eq!(run_state.phase, "metrics_catch_up");
    assert_eq!(run_state.subphase, None);
    assert_eq!(run_state.detail, None);
    assert_eq!(run_state.progress_state, ProgressState::Active);
    assert!(run_state.last_activity_epoch.is_some());
    assert!(run_state.last_progress_epoch.is_some());
  }

  #[test]
  fn catch_up_watchdog_disposition_detects_frozen_lane_run() {
    let mut state = IndexSchedulerState::default();
    let run_id = state.start_catch_up_run(SchedulerLane::MetricsCatchUp, 55);
    let now = now_epoch();

    state.catch_up.last_activity_epoch = Some(now.saturating_sub(WATCHDOG_ACTIVITY_STALE_SECS));
    state.catch_up.last_progress_epoch = Some(now.saturating_sub(WATCHDOG_ACTIVITY_STALE_SECS));
    state.catch_up.progress_state = ProgressState::Active;

    assert_eq!(
      state.active_watchdog_disposition(Some(run_id), now),
      RecoveryDisposition::Eligible
    );

    assert!(state.record_active_recovery_for_run(Some(run_id)));
    assert_eq!(state.catch_up.progress_state, ProgressState::Recovering);
  }

  #[test]
  fn forward_recovery_disposition_tracks_stale_and_exhausted_runs() {
    let mut state = IndexSchedulerState::default();
    let run_id = state.start_forward_run(77);
    state.record_forward_phase_event(run_id, 77, wire::msg::ForwardLanePhase::ApplyStagePlan);
    assert_eq!(state.forward.run_id, Some(run_id));
    state.forward.progress_state = wire::msg::ForwardProgressState::Stale;
    state.forward.stalled = true;

    assert_eq!(
      state.forward_recovery_disposition(Some(run_id)),
      RecoveryDisposition::Eligible
    );

    state.forward.recovery_attempts = WATCHDOG_RECOVERY_LIMIT;
    assert_eq!(
      state.forward_recovery_disposition(Some(run_id)),
      RecoveryDisposition::Exhausted
    );
  }

  #[test]
  fn forward_watchdog_disposition_detects_frozen_active_run() {
    let mut state = IndexSchedulerState::default();
    let run_id = state.start_forward_run(123);
    state.record_forward_phase_event(run_id, 123, wire::msg::ForwardLanePhase::AcquireChainState);
    assert_eq!(state.forward.run_id, Some(run_id));
    let now = now_epoch();
    state.forward.last_heartbeat_epoch = Some(now.saturating_sub(WATCHDOG_ACTIVITY_STALE_SECS));
    state.forward.last_progress_epoch = Some(now.saturating_sub(WATCHDOG_ACTIVITY_STALE_SECS));
    state.forward.progress_state = wire::msg::ForwardProgressState::Active;

    assert_eq!(
      state.forward_watchdog_disposition(Some(run_id), now),
      RecoveryDisposition::Eligible
    );

    state.forward.recovery_attempts = WATCHDOG_RECOVERY_LIMIT;
    assert_eq!(
      state.forward_watchdog_disposition(Some(run_id), now),
      RecoveryDisposition::Exhausted
    );
  }

  #[test]
  fn forward_watchdog_disposition_tolerates_stale_progress_with_fresh_activity() {
    let mut state = IndexSchedulerState::default();
    let run_id = state.start_forward_run(222);
    state.record_forward_phase_event(run_id, 222, wire::msg::ForwardLanePhase::ApplyStagePlan);
    state.record_forward_stage_event(run_id, Stage::Spent);
    let now = now_epoch();
    state.forward.last_heartbeat_epoch = Some(now);
    state.forward.last_progress_epoch = Some(now.saturating_sub(WATCHDOG_PROGRESS_STALE_SECS));
    state.forward.progress_state = wire::msg::ForwardProgressState::Active;

    assert_eq!(
      state.forward_watchdog_disposition(Some(run_id), now),
      RecoveryDisposition::NotNeeded
    );
  }

  #[test]
  fn forward_watchdog_disposition_keeps_provisional_runs_recoverable() {
    let mut state = IndexSchedulerState::default();
    let run_id = state.start_forward_run(91);
    let now = now_epoch();

    state.forward.last_heartbeat_epoch = Some(now.saturating_sub(WATCHDOG_ACTIVITY_STALE_SECS));

    assert_eq!(
      state.forward_watchdog_disposition(Some(run_id), now),
      RecoveryDisposition::Eligible
    );

    let recovery = state.record_forward_recovery_for_run(Some(run_id));
    assert_eq!(
      recovery,
      Some((91, wire::msg::ForwardLanePhase::AcquireChainState, 1))
    );
  }

  #[test]
  fn forward_phase_events_do_not_rotate_run_id() {
    let mut state = IndexSchedulerState::default();
    let run_id = state.start_forward_run(144);

    state.record_forward_phase_event(run_id, 144, wire::msg::ForwardLanePhase::AcquireChainState);
    state.record_forward_stage_event(run_id, Stage::Spent);
    state.record_forward_progress_event(
      run_id,
      ApplyStagePlanSubstage::StageStart.into(),
      Some(1),
      Some(2),
    );

    assert_eq!(state.forward.run_id, Some(run_id));
    assert_eq!(
      state.forward.phase,
      Some(wire::msg::ForwardLanePhase::AcquireChainState)
    );
    assert_eq!(state.forward.stage, Some(Stage::Spent));
    assert_eq!(
      state.forward.substage,
      Some(ApplyStagePlanSubstage::StageStart.into())
    );
    assert_eq!(state.forward.chunk_index, Some(1));
    assert_eq!(state.forward.chunk_total, Some(2));
  }
}
