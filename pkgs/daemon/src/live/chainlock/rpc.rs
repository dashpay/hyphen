//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live ChainLock refresh backed by node JSON-RPC.

use std::sync::Arc;

use crate::error::{DaemonError, RestErrorKind};
use crate::lifecycle::Lifecycle;
use crate::live::state::{ChainLockSnapshot, LiveState};
use crate::live::stream::{LiveEvent, StreamHub};
use crate::live::warn;
use crate::policy::api::API_REFRESH_WARN_INTERVAL_SECS;
use crate::sources::RpcSource;

/// Refresh the in-RAM latest ChainLock snapshot once from node JSON-RPC.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when the RPC request fails or the
/// ChainLock hex fields cannot be decoded.
pub async fn refresh_once(
  live_state: &LiveState,
  rpc: &dyn RpcSource,
  start_block: u32,
  stream_hub: Option<&StreamHub>,
) -> Result<(), DaemonError> {
  let chainlock = rpc.get_best_chainlock().await?;
  if chainlock.height < start_block {
    return Ok(());
  }

  let snapshot = ChainLockSnapshot {
    block_hash: hex::decode(&chainlock.blockhash).map_err(|e| DaemonError::RestSource {
      message: format!("decode chainlock block hash {}: {e}", chainlock.blockhash),
      kind: RestErrorKind::Other,
    })?,
    block_height: Some(chainlock.height as i32),
    core_height: Some(chainlock.height as i32),
    raw_chainlock: hex::decode(&chainlock.hex).map_err(|e| DaemonError::RestSource {
      message: format!("decode chainlock hex for {}: {e}", chainlock.blockhash),
      kind: RestErrorKind::Other,
    })?,
    seen_at: now_epoch_secs(),
  };

  live_state.set_chainlock(snapshot);
  if let Some(stream_hub) = stream_hub {
    stream_hub.publish(LiveEvent::chainlock(
      chainlock.blockhash,
      Some(chainlock.height as i32),
      chainlock.hex,
    ));
  }
  Ok(())
}

/// Periodically refresh ChainLock state until shutdown.
pub async fn refresh_loop(
  live_state: Arc<LiveState>,
  rpc: Arc<dyn RpcSource>,
  lifecycle: Arc<Lifecycle>,
  refresh_secs: u64,
  start_block: u32,
  stream_hub: Option<Arc<StreamHub>>,
) {
  while lifecycle.running() {
    if let Err(err) = refresh_once(
      &live_state,
      rpc.as_ref(),
      start_block,
      stream_hub.as_deref(),
    )
    .await
    {
      warn::warn_every(
        "live_chainlock_refresh_failed",
        std::time::Duration::from_secs(API_REFRESH_WARN_INTERVAL_SECS),
        |suppressed| {
          tracing::warn!(suppressed, "chainlock refresh failed: {err}");
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
