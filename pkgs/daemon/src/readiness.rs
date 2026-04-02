//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared `sync_state` readiness cache for REST and IPC snapshot reads.

use std::collections::HashMap;
use std::sync::Arc;

use crate::sql::SqlPool;
use tokio::sync::{Mutex, Notify, RwLock};

use crate::error::DaemonError;
use crate::index::Stage;
use crate::sql::query::SyncStateRow;

/// Cached `sync_state` rows keyed by stage name.
#[derive(Debug, Clone, Default)]
pub struct SyncStateSnapshot {
  heights: HashMap<String, i32>,
}

impl SyncStateSnapshot {
  fn from_rows(rows: &[SyncStateRow]) -> Self {
    Self {
      heights: rows
        .iter()
        .map(|row| (row.stage.clone(), row.indexed_height))
        .collect(),
    }
  }

  fn weight_bytes(&self) -> usize {
    self
      .heights
      .keys()
      .map(|stage| stage.len() + std::mem::size_of::<i32>())
      .sum()
  }

  /// Return the cached indexed height for one stage when present.
  pub fn height(&self, stage: Stage) -> Option<i32> {
    self.heights.get(stage.name()).copied()
  }
}

/// Shared on-demand cache for `sync_state` snapshots.
#[derive(Debug)]
pub struct ReadinessCache {
  enabled: bool,
  budget_bytes: usize,
  cached: RwLock<Option<Arc<SyncStateSnapshot>>>,
  inflight: Mutex<Option<Arc<Notify>>>,
}

impl ReadinessCache {
  /// Build one readiness cache with the configured budget.
  pub fn new(enabled: bool, budget_bytes: usize) -> Self {
    Self {
      enabled,
      budget_bytes,
      cached: RwLock::new(None),
      inflight: Mutex::new(None),
    }
  }

  /// Replace the cached snapshot from already-fetched `sync_state` rows.
  pub async fn replace_from_rows(&self, rows: &[SyncStateRow]) {
    if !self.enabled {
      return;
    }
    let next = Arc::new(SyncStateSnapshot::from_rows(rows));
    let mut cached = self.cached.write().await;
    if next.weight_bytes() <= self.budget_bytes {
      *cached = Some(next);
    } else {
      *cached = None;
    }
  }
  /// Read the current cached snapshot or query Postgres on demand.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError` when the backing `sync_state` query fails.
  pub async fn snapshot(&self, pool: &SqlPool) -> Result<Arc<SyncStateSnapshot>, DaemonError> {
    if !self.enabled {
      let rows = crate::sql::query::sync_state_rows(pool).await?;
      return Ok(Arc::new(SyncStateSnapshot::from_rows(&rows)));
    }

    if let Some(snapshot) = self.cached.read().await.as_ref().map(Arc::clone) {
      return Ok(snapshot);
    }

    loop {
      let wait = {
        let mut inflight = self.inflight.lock().await;
        if let Some(wait) = inflight.as_ref() {
          Some(Arc::clone(wait))
        } else {
          *inflight = Some(Arc::new(Notify::new()));
          None
        }
      };
      if let Some(wait) = wait {
        wait.notified().await;
        if let Some(snapshot) = self.cached.read().await.as_ref().map(Arc::clone) {
          return Ok(snapshot);
        }
        continue;
      }

      let rows = crate::sql::query::sync_state_rows(pool).await;
      let mut inflight = self.inflight.lock().await;
      let wait = inflight.take();
      if let Ok(rows) = rows {
        drop(inflight);
        self.replace_from_rows(&rows).await;
        if let Some(wait) = wait {
          wait.notify_waiters();
        }
        if let Some(snapshot) = self.cached.read().await.as_ref().map(Arc::clone) {
          return Ok(snapshot);
        }
        return Ok(Arc::new(SyncStateSnapshot::from_rows(&rows)));
      }
      if let Some(wait) = wait {
        wait.notify_waiters();
      }
      return rows.map(|_| Arc::new(SyncStateSnapshot::default()));
    }
  }
}
