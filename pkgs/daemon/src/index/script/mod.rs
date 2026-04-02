//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Script index: derives addresses from output scripts and updates tx_outputs.

pub(crate) mod ipc;

use std::sync::{Arc, Mutex};

use super::{BlockCtx, IndexOps};
use crate::error::DaemonError;
pub(crate) use crate::sql::index::script as sql;

pub struct ScriptIndex {
  pub(crate) ensured_partition_start: Arc<Mutex<Option<i32>>>,
}

impl std::fmt::Debug for ScriptIndex {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ScriptIndex").finish()
  }
}

#[async_trait::async_trait]
impl IndexOps for ScriptIndex {
  async fn disconnect_block(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    height: u32,
  ) -> Result<(), DaemonError> {
    sql::restore_output_addresses(tx, height).await
  }

  fn reset_cache(&self) {
    if let Ok(mut cache) = self.ensured_partition_start.lock() {
      *cache = None;
    }
  }
}

impl ScriptIndex {
  pub(crate) fn plan_connect(&self, _block: &BlockCtx) -> Result<ScriptBatchPlan, DaemonError> {
    Ok(ScriptBatchPlan)
  }

  pub(crate) async fn apply_plan(
    &self,
    tx: &mut crate::sql::SqlTx<'_>,
    plan: &ScriptBatchPlan,
  ) -> Result<(), DaemonError> {
    plan.apply(tx).await
  }
}

#[derive(Default)]
pub(crate) struct ScriptBatchPlan;

impl ScriptBatchPlan {
  async fn apply(&self, tx: &mut crate::sql::SqlTx<'_>) -> Result<(), DaemonError> {
    let _ = tx;
    Ok(())
  }
}
