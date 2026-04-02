//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Disconnect-side stage planning and apply orchestration.

use crate::sql::SqlPool;

use crate::error::DaemonError;

use super::batch::{
  BatchSizer, DEFAULT_COMMIT_BATCH_BLOCKS, DEFAULT_MAX_BATCH_SIZE, DEFAULT_MIN_BATCH_SIZE,
  DEFAULT_TARGET_BATCH_MS,
};
use super::block;
use super::metrics;
use super::script;
use super::spent;
use super::tx;
use super::validator;
use super::IndexOps;
use super::{disconnect_stage_order, Stage};

use std::sync::{Arc, Mutex};

use crate::index::connect::update_sync_state_with_chain_progress_state;

/// Disconnect-side orchestrator used by rewind and disconnect-tip paths.
pub struct DisconnectPipeline {
  block: block::BlockIndex,
  transaction: tx::TransactionIndex,
  script: script::ScriptIndex,
  spent: spent::SpentIndex,
  metrics: metrics::MetricsIndex,
  validator: validator::ValidatorIndex,
}

impl std::fmt::Debug for DisconnectPipeline {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("DisconnectPipeline").finish()
  }
}

pub struct DisconnectBlockPlan {
  height: u32,
  rewound_to: i32,
  stages: Vec<Stage>,
}

impl std::fmt::Debug for DisconnectBlockPlan {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("DisconnectBlockPlan")
      .field("height", &self.height)
      .field("rewound_to", &self.rewound_to)
      .field("stages", &self.stages)
      .finish()
  }
}

impl DisconnectBlockPlan {
  /// Apply all stage rewinds and move `sync_state` back one block.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` when any stage rewind or `sync_state`
  /// update fails.
  pub async fn apply(
    &self,
    pipeline: &DisconnectPipeline,
    tx: &mut crate::sql::SqlTx<'_>,
  ) -> Result<(), DaemonError> {
    pipeline.apply_disconnect(tx, self).await?;
    update_sync_state_with_chain_progress_state(
      tx,
      self.rewound_to,
      &self.stages,
      "disconnecting",
      Some(self.height as i32),
    )
    .await
  }
}

impl DisconnectPipeline {
  /// Build a disconnect pipeline covering all reversible stages.
  pub fn new(
    db: Option<Arc<rocksdb::DB>>,
    secondary_stores: Option<Arc<crate::store::manager::SecondaryStoreSet>>,
    p2pkh_version: u8,
    p2sh_version: u8,
    block_time_secs: f64,
    spent_cache: Arc<spent::cache::SpentQueryCache>,
  ) -> Self {
    Self {
      block: block::BlockIndex::new(),
      transaction: tx::TransactionIndex {
        db: db.clone(),
        secondary_stores: secondary_stores.clone(),
        p2pkh_version,
        p2sh_version,
        batch_sizer: Arc::new(Mutex::new(BatchSizer::new(
          DEFAULT_COMMIT_BATCH_BLOCKS,
          DEFAULT_MIN_BATCH_SIZE,
          DEFAULT_MAX_BATCH_SIZE,
          DEFAULT_TARGET_BATCH_MS,
        ))),
        ensured_partition_start: Arc::new(std::sync::Mutex::new(None)),
      },
      script: script::ScriptIndex {
        ensured_partition_start: Arc::new(std::sync::Mutex::new(None)),
      },
      spent: spent::SpentIndex {
        db,
        secondary_stores,
        batch_sizer: Arc::new(Mutex::new(BatchSizer::new(
          DEFAULT_COMMIT_BATCH_BLOCKS,
          DEFAULT_MIN_BATCH_SIZE,
          DEFAULT_MAX_BATCH_SIZE,
          DEFAULT_TARGET_BATCH_MS,
        ))),
        cache: spent_cache,
        p2pkh_version,
        p2sh_version,
        ensured_partition_start: Arc::new(std::sync::Mutex::new(None)),
      },
      metrics: metrics::MetricsIndex::new(block_time_secs),
      validator: validator::ValidatorIndex {
        p2pkh_version,
        p2sh_version,
      },
    }
  }

  /// Plan a reverse transition for the canonical block at `height`.
  ///
  /// # Errors
  ///
  /// Returns a stage-specific `DaemonError` when any stage cannot derive
  /// its reverse plan for the block.
  pub async fn plan_disconnect(
    &self,
    pool: &SqlPool,
    height: u32,
  ) -> Result<DisconnectBlockPlan, DaemonError> {
    let mut stages = Vec::new();
    for &stage in disconnect_stage_order() {
      if stage_enabled_at_height_pool(pool, stage, height).await? {
        stages.push(stage);
      }
    }
    if !stages.contains(&Stage::Block) {
      stages.push(Stage::Block);
    }
    Ok(DisconnectBlockPlan {
      height,
      rewound_to: rewound_height(height),
      stages,
    })
  }

  /// Plan a reverse transition using the caller's active SQL transaction.
  ///
  /// This is used by undo backfill so temp-table shadow state stays visible to
  /// every planning query.
  pub async fn plan_disconnect_tx(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    height: u32,
  ) -> Result<DisconnectBlockPlan, DaemonError> {
    let mut stages = Vec::new();
    for &stage in disconnect_stage_order() {
      if stage_enabled_at_height_tx(tx, stage, height).await? {
        stages.push(stage);
      }
    }
    if !stages.contains(&Stage::Block) {
      stages.push(Stage::Block);
    }
    Ok(DisconnectBlockPlan {
      height,
      rewound_to: rewound_height(height),
      stages,
    })
  }

  async fn apply_disconnect(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    plan: &DisconnectBlockPlan,
  ) -> Result<(), DaemonError> {
    for stage in &plan.stages {
      self.disconnect_stage(tx, *stage, plan.height).await?;
    }
    Ok(())
  }

  async fn disconnect_stage(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    stage: Stage,
    height: u32,
  ) -> Result<(), DaemonError> {
    self.stage_driver(stage).disconnect(tx, height).await
  }

  fn stage_driver(&self, stage: Stage) -> StageDriverRef<'_> {
    match stage {
      Stage::Block => StageDriverRef::Block(&self.block),
      Stage::Transaction => StageDriverRef::Transaction(&self.transaction),
      Stage::Script => StageDriverRef::Script(&self.script),
      Stage::Spent => StageDriverRef::Spent(&self.spent),
      Stage::Metrics => StageDriverRef::Metrics(&self.metrics),
      Stage::Validator => StageDriverRef::Validator(&self.validator),
    }
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
  async fn disconnect(
    self,
    tx: &mut crate::sql::SqlTx<'_>,
    height: u32,
  ) -> Result<(), DaemonError> {
    match self {
      Self::Block(index) => index.disconnect_block(tx, height).await,
      Self::Transaction(index) => index.disconnect_block(tx, height).await,
      Self::Script(index) => index.disconnect_block(tx, height).await,
      Self::Spent(index) => index.disconnect_block(tx, height).await,
      Self::Metrics(index) => index.disconnect_block(tx, height).await,
      Self::Validator(index) => index.disconnect_block(tx, height).await,
    }
  }
}

pub(crate) fn rewound_height(height: u32) -> i32 {
  i32::try_from(height).map_or(i32::MAX, |value| value - 1)
}

async fn stage_enabled_at_height_pool(
  pool: &SqlPool,
  stage: Stage,
  height: u32,
) -> Result<bool, DaemonError> {
  Ok(crate::sql::query::indexed_stage_height(pool, stage).await? >= height as i32)
}

async fn stage_enabled_at_height_tx(
  tx: &mut crate::sql::SqlTx<'_>,
  stage: Stage,
  height: u32,
) -> Result<bool, DaemonError> {
  let indexed_height = crate::sql::query::indexed_stage_height_tx(tx, stage).await?;
  Ok(indexed_height >= height as i32)
}
