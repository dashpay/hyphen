//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Transaction index: stores transactions, inputs, outputs in PostgreSQL.

pub(crate) mod batch;
pub(crate) mod ipc;
pub(crate) mod overlay;
pub(crate) mod rest;

use std::sync::{Arc, Mutex};

use crate::error::DaemonError;
use crate::index::batch::BatchSizer;
pub(crate) use crate::sql::index::tx as sql;
use crate::store::manager::SecondaryStoreSet;

use super::{BlockCtx, IndexOps};

pub(crate) use batch::TransactionBatchPlan;

pub struct TransactionIndex {
  pub db: Option<Arc<rocksdb::DB>>,
  pub secondary_stores: Option<Arc<SecondaryStoreSet>>,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub(crate) batch_sizer: Arc<Mutex<BatchSizer>>,
  pub(crate) ensured_partition_start: Arc<Mutex<Option<i32>>>,
}

impl std::fmt::Debug for TransactionIndex {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("TransactionIndex").finish()
  }
}

#[async_trait::async_trait]
impl IndexOps for TransactionIndex {
  async fn disconnect_block(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    height: u32,
  ) -> Result<(), DaemonError> {
    sql::delete_tx_outputs(tx, height).await?;
    sql::delete_tx_inputs(tx, height).await?;
    sql::delete_special_tx_payloads(tx, height).await?;
    sql::delete_transactions_exact_summary(tx, height).await?;
    if let (Some(db), Some(stores)) = (&self.db, &self.secondary_stores) {
      let block = crate::store::load_block_ctx(db, height)?;
      let next_tip = height.checked_sub(1);
      let next_hash = next_tip.and_then(|h| crate::db::get_hash_for_height(db, h));
      crate::store::prevout::disconnect_block(
        stores.db(crate::store::StoreId::PrevoutLookup)?,
        &block,
        next_tip,
        next_hash,
      )?;
    }
    Ok(())
  }
}

impl TransactionIndex {
  pub(crate) fn plan_connect(&self, block: &BlockCtx) -> Result<TransactionBatchPlan, DaemonError> {
    TransactionBatchPlan::build(
      block,
      self.p2pkh_version,
      self.p2sh_version,
      &self.batch_sizer,
    )
  }

  pub(crate) async fn apply_plan(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    plan: &TransactionBatchPlan,
    reporter: Option<&super::runner::ForwardRunReporter>,
  ) -> Result<(), DaemonError> {
    if let Some(stores) = &self.secondary_stores {
      plan.apply_prevout_store(
        stores.db(crate::store::StoreId::PrevoutLookup)?,
        self.p2pkh_version,
        self.p2sh_version,
      )?;
    }
    let partition_start = crate::sql::partition::canonical_partition_start(plan.height() as i32);
    let should_ensure = self
      .ensured_partition_start
      .lock()
      .map_err(|e| DaemonError::Index(format!("transaction partition cache lock: {e}")))?
      .is_none_or(|cached| cached != partition_start);
    if should_ensure {
      sql::ensure_canonical_partitions(tx, plan.height() as i32).await?;
      *self
        .ensured_partition_start
        .lock()
        .map_err(|e| DaemonError::Index(format!("transaction partition cache lock: {e}")))? =
        Some(partition_start);
    }

    let started_at = std::time::Instant::now();
    plan.apply(tx, reporter).await?;
    if let Ok(mut sizer) = self.batch_sizer.lock() {
      sizer.adjust(
        started_at.elapsed().as_millis() as u64,
        super::Stage::Transaction.name(),
      );
    }
    Ok(())
  }
}
