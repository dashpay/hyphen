//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! ZMQ-backed mempool delta ownership.

use std::sync::Arc;

use crate::sql::SqlPool;
use tokio::sync::broadcast;

use crate::index::spent::cache::SpentQueryCache;
use crate::lifecycle::Lifecycle;
use crate::sources::ZmqEvent;

/// Persist recent mempool transaction changes from ZMQ until shutdown.
pub async fn sync_recent_loop(
  pool: SqlPool,
  rx: broadcast::Receiver<ZmqEvent>,
  lifecycle: Arc<Lifecycle>,
  keep: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
  spent_cache: Arc<SpentQueryCache>,
) {
  super::state::sync_recent_loop(
    pool,
    rx,
    lifecycle,
    keep,
    p2pkh_version,
    p2sh_version,
    spent_cache,
  )
  .await;
}
