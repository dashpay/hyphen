//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! ZMQ-backed mempool delta ownership.

use std::sync::Arc;

use hyphen_db::HyphenDb;
use tokio::sync::broadcast;

use crate::lifecycle::Lifecycle;
use crate::live::state::LiveState;
use crate::sources::ZmqEvent;

/// Track recent mempool transaction changes in RAM from ZMQ until shutdown.
#[expect(
  clippy::too_many_arguments,
  reason = "each argument is a distinct subscription input with no natural grouping"
)]
pub async fn sync_recent_loop(
  db: HyphenDb,
  live_state: Arc<LiveState>,
  rx: broadcast::Receiver<ZmqEvent>,
  lifecycle: Arc<Lifecycle>,
  keep: usize,
  p2pkh_version: u8,
  p2sh_version: u8,
) {
  super::state::sync_recent_loop(
    db,
    live_state,
    rx,
    lifecycle,
    keep,
    p2pkh_version,
    p2sh_version,
  )
  .await;
}
