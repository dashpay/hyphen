//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! ZMQ-backed governance delta ownership.

use std::sync::Arc;

use tokio::sync::broadcast;

use crate::lifecycle::Lifecycle;
use crate::live::state::LiveState;
use crate::live::stream::StreamHub;
use crate::sources::{CoreRpcClient, ZmqEvent};

/// Trigger governance refreshes from ZMQ governance deltas until shutdown.
pub async fn refresh_from_zmq_loop(
  live_state: Arc<LiveState>,
  rpc: Arc<CoreRpcClient>,
  rx: broadcast::Receiver<ZmqEvent>,
  lifecycle: Arc<Lifecycle>,
  stream_hub: Option<Arc<StreamHub>>,
) {
  super::state::refresh_from_zmq_loop(live_state, rpc, rx, lifecycle, stream_hub).await;
}
