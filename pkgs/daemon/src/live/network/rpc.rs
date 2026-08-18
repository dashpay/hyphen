//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live network snapshot refresh backed by node JSON-RPC.

use std::sync::Arc;

use serde_json::json;

use crate::error::DaemonError;
use crate::lifecycle::Lifecycle;
use crate::live::state::{LiveState, NetworkSnapshot};
use crate::live::stream::{LiveEvent, StreamHub};
use crate::live::warn;
use crate::policy::api::API_REFRESH_WARN_INTERVAL_SECS;
use crate::sources::RpcSource;

/// Refresh the in-RAM network snapshot once from node JSON-RPC.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when the RPC request fails.
pub async fn refresh_once(
  live_state: &LiveState,
  rpc: &dyn RpcSource,
  stream_hub: Option<&StreamHub>,
) -> Result<(), DaemonError> {
  let network_info = rpc.get_network_info().await?;
  let peer_info = rpc.get_peer_info().await?;
  let payload = json!({
    "networkInfo": network_info,
    "peerInfo": peer_info,
    "peerCount": peer_info.as_array().map_or(0, |peers| peers.len() as i64),
  });
  live_state.set_network(NetworkSnapshot {
    payload: payload.clone(),
    updated_at: now_epoch_secs(),
  });
  if let Some(stream_hub) = stream_hub {
    stream_hub.publish(LiveEvent::network(payload));
  }
  Ok(())
}

/// Periodically refresh the in-RAM network snapshot until shutdown.
pub async fn refresh_loop(
  live_state: Arc<LiveState>,
  rpc: Arc<dyn RpcSource>,
  lifecycle: Arc<Lifecycle>,
  refresh_secs: u64,
  stream_hub: Option<Arc<StreamHub>>,
) {
  while lifecycle.running() {
    if let Err(err) = refresh_once(&live_state, rpc.as_ref(), stream_hub.as_deref()).await {
      warn::warn_every(
        "live_network_refresh_failed",
        std::time::Duration::from_secs(API_REFRESH_WARN_INTERVAL_SECS),
        |suppressed| {
          tracing::warn!(suppressed, "network snapshot refresh failed: {err}");
        },
      );
    }
    lifecycle
      .sleep(std::time::Duration::from_secs(refresh_secs))
      .await;
  }
}

fn now_epoch_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs() as i64
}
