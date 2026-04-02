//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Spent index: marks spent outputs, computes fees, builds address balances.

pub(crate) mod batch;
pub(crate) mod cache;
pub(crate) mod ipc;
pub(crate) mod rest;

use std::sync::{Arc, Mutex};

use crate::error::DaemonError;
use crate::index::batch::BatchSizer;
pub(crate) use crate::sql::index::spent as sql;
use crate::store::manager::SecondaryStoreSet;

use super::{BlockCtx, IndexOps};

pub(crate) use batch::SpentBatchPlan;

pub struct SpentIndex {
  pub db: Option<Arc<rocksdb::DB>>,
  pub secondary_stores: Option<Arc<SecondaryStoreSet>>,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub(crate) batch_sizer: Arc<Mutex<BatchSizer>>,
  pub(crate) cache: Arc<cache::SpentQueryCache>,
  pub(crate) ensured_partition_start: Arc<Mutex<Option<i32>>>,
}

impl std::fmt::Debug for SpentIndex {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("SpentIndex").finish()
  }
}

#[async_trait::async_trait]
impl IndexOps for SpentIndex {
  async fn disconnect_block(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    height: u32,
  ) -> Result<(), DaemonError> {
    let h = height as i32;
    let Some(db) = &self.db else {
      return Err(DaemonError::Index(
        "raw block store required for address history disconnect".to_owned(),
      ));
    };
    let Some(stores) = &self.secondary_stores else {
      return Err(DaemonError::Index(
        "address history secondary store not available".to_owned(),
      ));
    };

    sql::restore_spent_at_height(tx, h).await?;
    let block = crate::store::load_block_ctx(db, height)?;
    let next_tip = height.checked_sub(1);
    let next_hash = next_tip.and_then(|next| crate::db::get_hash_for_height(db, next));
    crate::store::address_history::disconnect_block(
      stores.db(crate::store::StoreId::AddressHistory)?,
      stores.db(crate::store::StoreId::PrevoutLookup)?,
      &block,
      self.p2pkh_version,
      self.p2sh_version,
      next_tip,
      next_hash,
    )?;
    sql::reverse_address_stats_at_height(tx, h).await?;
    sql::reverse_address_balances_at_height(tx, h).await?;

    Ok(())
  }

  fn reset_cache(&self) {
    if let Ok(mut cache) = self.ensured_partition_start.lock() {
      *cache = None;
    }
    self.cache.reset_stage_generation();
  }
}

impl SpentIndex {
  pub(crate) async fn plan_connect(
    &self,
    block: &BlockCtx,
    reporter: Option<&super::runner::ForwardRunReporter>,
  ) -> Result<SpentBatchPlan, DaemonError> {
    batch::SpentBatchPlan::build(
      block,
      self.p2pkh_version,
      self.p2sh_version,
      &self.batch_sizer,
      reporter,
    )
    .await
  }

  pub(crate) async fn apply_plan(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    plan: &SpentBatchPlan,
    reporter: Option<&super::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    let prevout_db = self
      .secondary_stores
      .as_ref()
      .ok_or_else(|| DaemonError::Index("prevout secondary store not available".to_owned()))?
      .db(crate::store::StoreId::PrevoutLookup)?;
    let address_history_db = self
      .secondary_stores
      .as_ref()
      .map(|stores| stores.db(crate::store::StoreId::AddressHistory))
      .transpose()?
      .map(std::ops::Deref::deref);
    let partition_start =
      crate::sql::partition::history_partition_start(plan.history_height() as i32);
    let should_ensure = self
      .ensured_partition_start
      .lock()
      .map_err(|e| DaemonError::Index(format!("spent partition cache lock: {e}")))?
      .is_none_or(|cached| cached != partition_start);
    if should_ensure {
      sql::ensure_history_partitions(tx, plan.history_height() as i32).await?;
      *self
        .ensured_partition_start
        .lock()
        .map_err(|e| DaemonError::Index(format!("spent partition cache lock: {e}")))? =
        Some(partition_start);
    }

    let started_at = std::time::Instant::now();
    plan
      .apply(
        tx,
        prevout_db,
        address_history_db,
        false,
        reporter,
      )
      .await?;
    if let Ok(mut sizer) = self.batch_sizer.lock() {
      sizer.adjust(
        started_at.elapsed().as_millis() as u64,
        super::Stage::Spent.name(),
      );
    }
    Ok(())
  }
}
