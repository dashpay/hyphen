//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Connect-side stage planning and apply orchestration.

use std::sync::Arc;
use std::time::Instant;

use crate::error::DaemonError;
use crate::policy::watchdog::WATCHDOG_SLOW_OPERATION_MS;

use super::block;
use super::metrics;
use super::runner::{ApplyStagePlanSubstage, PlanConnectSubstage};
use super::script;
use super::spent;
use super::tx;
use super::validator;
use super::IndexOps;
use super::{connect_stage_order, exact_stage_order, BlockCtx, IndexerLoopCtx, Stage};

/// Connect-side stage orchestrator used by the block indexer loop.
pub struct ForwardPipeline {
  block: block::BlockIndex,
  transaction: tx::TransactionIndex,
  script: Option<script::ScriptIndex>,
  spent: Option<spent::SpentIndex>,
  metrics: Option<metrics::MetricsIndex>,
  validator: Option<validator::ValidatorIndex>,
}

impl std::fmt::Debug for ForwardPipeline {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ForwardPipeline").finish()
  }
}

enum ForwardStagePlan {
  Block(block::BlockInsertPlan),
  Transaction(tx::TransactionBatchPlan),
  Script(script::ScriptBatchPlan),
  Spent(spent::SpentBatchPlan),
  Metrics(metrics::MetricsBlockPlan),
  Validator(validator::ValidatorBlockPlan),
}

impl std::fmt::Debug for ForwardStagePlan {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    let stage = match self {
      Self::Block(_) => "block",
      Self::Transaction(_) => "transaction",
      Self::Script(_) => "script",
      Self::Spent(_) => "spent",
      Self::Metrics(_) => "metrics",
      Self::Validator(_) => "validator",
    };
    f.debug_tuple("ForwardStagePlan").field(&stage).finish()
  }
}

impl ForwardStagePlan {
  fn stage(&self) -> Stage {
    match self {
      Self::Block(_) => Stage::Block,
      Self::Transaction(_) => Stage::Transaction,
      Self::Script(_) => Stage::Script,
      Self::Spent(_) => Stage::Spent,
      Self::Metrics(_) => Stage::Metrics,
      Self::Validator(_) => Stage::Validator,
    }
  }
}

impl Stage {
  fn plan_connect_substage(self) -> PlanConnectSubstage {
    match self {
      Self::Block => PlanConnectSubstage::Block,
      Self::Transaction => PlanConnectSubstage::Transaction,
      Self::Script => PlanConnectSubstage::Script,
      Self::Spent => PlanConnectSubstage::SpentCollectInputs,
      Self::Metrics => PlanConnectSubstage::Metrics,
      Self::Validator => PlanConnectSubstage::Validator,
    }
  }
}

pub struct ForwardBlockPlan {
  stages: Vec<ForwardStagePlan>,
  height: u32,
  sync_stages: Vec<Stage>,
}

impl std::fmt::Debug for ForwardBlockPlan {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ForwardBlockPlan")
      .field("height", &self.height)
      .field("sync_stages", &self.sync_stages)
      .finish()
  }
}

impl ForwardBlockPlan {
  /// Apply all stage writes and advance `sync_state` for this block.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` when any stage apply step or `sync_state`
  /// update fails.
  pub(crate) async fn apply(
    &self,
    pipeline: &ForwardPipeline,
    tx: &mut crate::sql::SqlTx<'_>,
    reporter: Option<&super::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    pipeline.apply_connect(tx, self, reporter).await?;
    update_sync_state(tx, self.height as i32, &self.sync_stages).await
  }
}

impl ForwardPipeline {
  /// Build a forward pipeline with only the enabled derived stages.
  pub(crate) fn new(ctx: &IndexerLoopCtx) -> Self {
    let address_enabled = ctx.enabled_stages.contains(&Stage::Script);
    let metrics_enabled = ctx.enabled_stages.contains(&Stage::Metrics);
    let validator_enabled = ctx.enabled_stages.contains(&Stage::Validator);

    Self {
      block: block::BlockIndex::new(),
      transaction: tx::TransactionIndex {
        db: Some(Arc::clone(&ctx.db)),
        secondary_stores: ctx.secondary_stores.clone(),
        p2pkh_version: ctx.p2pkh_version,
        p2sh_version: ctx.p2sh_version,
        batch_sizer: Arc::clone(&ctx.tx_batch_sizer),
        ensured_partition_start: Arc::clone(&ctx.canonical_partition_cache),
      },
      script: address_enabled.then_some(script::ScriptIndex {
        ensured_partition_start: Arc::clone(&ctx.history_partition_cache),
      }),
      spent: address_enabled.then_some(spent::SpentIndex {
        db: Some(Arc::clone(&ctx.db)),
        secondary_stores: ctx.secondary_stores.clone(),
        batch_sizer: Arc::clone(&ctx.spent_batch_sizer),
        cache: Arc::clone(&ctx.spent_cache),
        p2pkh_version: ctx.p2pkh_version,
        p2sh_version: ctx.p2sh_version,
        ensured_partition_start: Arc::clone(&ctx.history_partition_cache),
      }),
      metrics: metrics_enabled.then_some(metrics::MetricsIndex::new(ctx.block_time_secs)),
      validator: validator_enabled.then_some(validator::ValidatorIndex {
        p2pkh_version: ctx.p2pkh_version,
        p2sh_version: ctx.p2sh_version,
      }),
    }
  }

  /// Plan all enabled stage effects for one block before any writes occur.
  ///
  /// # Errors
  ///
  /// Returns a stage-specific `DaemonError` when any stage cannot plan
  /// its contribution for the block.
  #[cfg_attr(
    not(test),
    expect(dead_code, reason = "exercised by index integration tests")
  )]
  pub(crate) async fn plan_connect(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    block: &BlockCtx,
    enabled_stages: &[Stage],
  ) -> Result<ForwardBlockPlan, DaemonError> {
    self
      .plan_connect_exact(tx, block, &connect_stage_order(enabled_stages), None)
      .await
  }

  /// Plan an explicit ordered subset of stages for one block.
  ///
  /// # Errors
  ///
  /// Returns a stage-specific `DaemonError` when any stage cannot plan
  /// its contribution for the block.
  pub(crate) async fn plan_connect_exact(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    block: &BlockCtx,
    stages_to_run: &[Stage],
    reporter: Option<&super::runner::ForwardRunReporter>,
  ) -> Result<ForwardBlockPlan, DaemonError> {
    let ordered = exact_stage_order(stages_to_run);
    let mut stages = Vec::new();
    let stage_total = ordered.len() as u32;
    for (index, stage) in ordered.iter().copied().enumerate() {
      let stage_index = (index + 1) as u32;
      let substage = stage.plan_connect_substage();
      if let Some(reporter) = reporter {
        reporter.stage(stage);
        reporter.activity(substage, Some(stage_index), Some(stage_total));
      }
      tracing::debug!(
        height = block.height(),
        stage = stage.name(),
        stage_index,
        stage_total,
        "forward stage plan start"
      );
      let started = Instant::now();
      let stage_plan = self.plan_stage(tx, stage, block, reporter).await?;
      let elapsed_ms = started.elapsed().as_millis() as u64;
      if elapsed_ms >= WATCHDOG_SLOW_OPERATION_MS {
        tracing::warn!(
          height = block.height(),
          stage = stage.name(),
          stage_index,
          stage_total,
          elapsed_ms,
          "forward stage plan is slow"
        );
      }
      tracing::debug!(
        height = block.height(),
        stage = stage.name(),
        stage_index,
        stage_total,
        elapsed_ms,
        "forward stage plan complete"
      );
      if let Some(reporter) = reporter {
        reporter.progress(substage, Some(stage_index), Some(stage_total));
      }
      stages.push(stage_plan);
    }

    Ok(ForwardBlockPlan {
      stages,
      height: block.height(),
      sync_stages: ordered,
    })
  }

  async fn apply_connect(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    plan: &ForwardBlockPlan,
    reporter: Option<&super::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    for stage in &plan.stages {
      if let Some(reporter) = reporter {
        reporter.stage(stage.stage());
        reporter.progress(ApplyStagePlanSubstage::StageStart, None, None);
      }
      self.apply_forward_stage(tx, stage, reporter).await?;
    }
    Ok(())
  }

  /// Clear any stage-local caches held by the forward pipeline.
  pub fn reset_caches(&self) {
    for stage in connect_stage_order(&super::enabled_stage_order(super::IndexStageFlags {
      address: self.script.is_some(),
      metrics: self.metrics.is_some(),
      validator: self.validator.is_some(),
    })) {
      self.reset_stage_cache(stage);
    }
  }

  async fn plan_stage(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    stage: Stage,
    block: &BlockCtx,
    reporter: Option<&super::runner::ForwardRunReporter>,
  ) -> Result<ForwardStagePlan, DaemonError> {
    self
      .stage_driver(stage, StageAccess::Plan)?
      .plan_connect(tx, block, reporter)
      .await
  }

  async fn apply_forward_stage(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    stage: &ForwardStagePlan,
    reporter: Option<&super::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    let stage_name = stage.stage().name();
    tracing::debug!(stage = stage_name, "forward stage apply start");
    self
      .stage_driver(stage.stage(), StageAccess::Apply)?
      .apply_forward(tx, stage, reporter)
      .await?;
    tracing::debug!(stage = stage_name, "forward stage apply complete");
    Ok(())
  }

  fn reset_stage_cache(&self, stage: Stage) {
    if let Ok(driver) = self.stage_driver(stage, StageAccess::Reset) {
      driver.reset_cache();
    }
  }

  fn stage_driver(
    &self,
    stage: Stage,
    access: StageAccess,
  ) -> Result<StageDriverRef<'_>, DaemonError> {
    match stage {
      Stage::Block => Ok(StageDriverRef::Block(&self.block)),
      Stage::Transaction => Ok(StageDriverRef::Transaction(&self.transaction)),
      Stage::Script => Ok(StageDriverRef::Script(self.require_script(access)?)),
      Stage::Spent => Ok(StageDriverRef::Spent(self.require_spent(access)?)),
      Stage::Metrics => Ok(StageDriverRef::Metrics(self.require_metrics(access)?)),
      Stage::Validator => Ok(StageDriverRef::Validator(self.require_validator(access)?)),
    }
  }

  fn require_script(&self, access: StageAccess) -> Result<&script::ScriptIndex, DaemonError> {
    self
      .script
      .as_ref()
      .ok_or_else(|| access.missing(Stage::Script))
  }

  fn require_spent(&self, access: StageAccess) -> Result<&spent::SpentIndex, DaemonError> {
    self
      .spent
      .as_ref()
      .ok_or_else(|| access.missing(Stage::Spent))
  }

  fn require_metrics(&self, access: StageAccess) -> Result<&metrics::MetricsIndex, DaemonError> {
    self
      .metrics
      .as_ref()
      .ok_or_else(|| access.missing(Stage::Metrics))
  }

  fn require_validator(
    &self,
    access: StageAccess,
  ) -> Result<&validator::ValidatorIndex, DaemonError> {
    self
      .validator
      .as_ref()
      .ok_or_else(|| access.missing(Stage::Validator))
  }
}

#[derive(Clone, Copy)]
enum StageAccess {
  Plan,
  Apply,
  Reset,
}

impl StageAccess {
  fn missing(self, stage: Stage) -> DaemonError {
    let action = match self {
      Self::Plan => "plan",
      Self::Apply => "apply",
      Self::Reset => "reset",
    };
    DaemonError::Index(format!("{} stage missing during {action}", stage.name()))
  }
}

enum StageDriverRef<'a> {
  Block(&'a block::BlockIndex),
  Transaction(&'a tx::TransactionIndex),
  Script(&'a script::ScriptIndex),
  Spent(&'a spent::SpentIndex),
  Metrics(&'a metrics::MetricsIndex),
  Validator(&'a validator::ValidatorIndex),
}

impl StageDriverRef<'_> {
  async fn plan_connect(
    self,
    tx: &mut crate::sql::SqlTx<'_>,
    block: &BlockCtx,
    reporter: Option<&super::runner::ForwardRunReporter>,
  ) -> Result<ForwardStagePlan, DaemonError> {
    match self {
      Self::Block(index) => Ok(ForwardStagePlan::Block(
        index.plan_connect(tx, block).await?,
      )),
      Self::Transaction(index) => Ok(ForwardStagePlan::Transaction(index.plan_connect(block)?)),
      Self::Script(index) => Ok(ForwardStagePlan::Script(index.plan_connect(block)?)),
      Self::Spent(index) => Ok(ForwardStagePlan::Spent(
        index.plan_connect(block, reporter).await?,
      )),
      Self::Metrics(index) => Ok(ForwardStagePlan::Metrics(
        index.plan_connect(tx, block).await?,
      )),
      Self::Validator(index) => Ok(ForwardStagePlan::Validator(index.plan_connect(block)?)),
    }
  }

  async fn apply_forward(
    self,
    tx: &mut crate::sql::SqlTx<'_>,
    plan: &ForwardStagePlan,
    reporter: Option<&super::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    match (self, plan) {
      (Self::Block(index), ForwardStagePlan::Block(stage_plan)) => {
        index.apply_plan(tx, stage_plan).await
      }
      (Self::Transaction(index), ForwardStagePlan::Transaction(stage_plan)) => {
        index.apply_plan(tx, stage_plan, reporter).await
      }
      (Self::Script(index), ForwardStagePlan::Script(stage_plan)) => {
        index.apply_plan(tx, stage_plan).await
      }
      (Self::Spent(index), ForwardStagePlan::Spent(stage_plan)) => {
        index.apply_plan(tx, stage_plan, reporter).await
      }
      (Self::Metrics(index), ForwardStagePlan::Metrics(stage_plan)) => {
        index.apply_plan(tx, stage_plan).await
      }
      (Self::Validator(index), ForwardStagePlan::Validator(stage_plan)) => {
        index.apply_plan(tx, stage_plan).await
      }
      (driver, stage_plan) => Err(DaemonError::Index(format!(
        "stage {} cannot apply planned {}",
        driver.stage().name(),
        stage_plan.stage().name()
      ))),
    }
  }

  fn reset_cache(self) {
    match self {
      Self::Block(index) => index.reset_cache(),
      Self::Transaction(index) => index.reset_cache(),
      Self::Script(index) => index.reset_cache(),
      Self::Spent(index) => index.reset_cache(),
      Self::Metrics(index) => index.reset_cache(),
      Self::Validator(index) => index.reset_cache(),
    }
  }

  fn stage(self) -> Stage {
    match self {
      Self::Block(_) => Stage::Block,
      Self::Transaction(_) => Stage::Transaction,
      Self::Script(_) => Stage::Script,
      Self::Spent(_) => Stage::Spent,
      Self::Metrics(_) => Stage::Metrics,
      Self::Validator(_) => Stage::Validator,
    }
  }
}

#[cfg_attr(
  not(test),
  expect(dead_code, reason = "exercised by index integration tests")
)]
pub(crate) fn sync_stages_for(enabled_stages: &[Stage]) -> Vec<Stage> {
  connect_stage_order(enabled_stages)
}

pub(crate) async fn update_sync_state(
  tx: &mut crate::sql::SqlTx<'_>,
  height: i32,
  stages: &[Stage],
) -> Result<(), DaemonError> {
  update_sync_state_with_chain_progress_state(tx, height, stages, "clean", None).await
}

pub(crate) async fn update_sync_state_with_chain_progress_state(
  tx: &mut crate::sql::SqlTx<'_>,
  height: i32,
  stages: &[Stage],
  recovery_state: &str,
  recovery_height: Option<i32>,
) -> Result<(), DaemonError> {
  let now = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs() as i64)
    .unwrap_or(0);

  for stage in stages {
    crate::sql::query::update_indexed_stage_height_tx(tx, *stage, height, now).await?;
  }

  if stages.contains(&Stage::Block) {
    let tip_hash = if height >= 0 {
      crate::sql::query::block_hash_at_height_tx(tx, height).await?
    } else {
      None
    };
    crate::sql::progress::sync_chain_progress_state_tx(
      tx,
      height,
      tip_hash.as_deref(),
      now,
      recovery_state,
      recovery_height,
    )
    .await?;
  }

  Ok(())
}
