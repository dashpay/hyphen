//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! JSON-RPC-backed governance refresh ownership.

use std::sync::Arc;

use crate::lifecycle::Lifecycle;
use crate::live::state::LiveState;
use crate::live::stream::StreamHub;
use crate::sources::CoreRpcClient;

/// Periodically refresh governance state until shutdown.
pub async fn refresh_loop(
  live_state: Arc<LiveState>,
  rpc: Arc<CoreRpcClient>,
  lifecycle: Arc<Lifecycle>,
  refresh_secs: u64,
  stream_hub: Option<Arc<StreamHub>>,
) {
  super::state::refresh_loop(live_state, rpc, lifecycle, refresh_secs, stream_hub).await;
}
