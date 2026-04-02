//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live ChainLock refresh backed by node JSON-RPC.

use std::sync::Arc;

use crate::sql::SqlPool;

use crate::error::DaemonError;
use crate::lifecycle::Lifecycle;
use crate::live::stream::{LiveEvent, StreamHub};
use crate::live::warn;
use crate::policy::api::API_REFRESH_WARN_INTERVAL_SECS;
use crate::sources::RpcSource;
use crate::sql;

/// Refresh the persisted latest ChainLock snapshot once.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when the RPC request fails or
/// `DaemonError::Pg` when PostgreSQL rejects the write.
pub async fn refresh_once(
  pool: &SqlPool,
  rpc: &dyn RpcSource,
  start_block: u32,
  stream_hub: Option<&StreamHub>,
) -> Result<(), DaemonError> {
  let chainlock = rpc.get_best_chainlock().await?;
  if chainlock.height < start_block {
    return Ok(());
  }

  let row = sql::live::chainlock::ChainLockSnapshotRow {
    block_hash: hex::decode(&chainlock.blockhash).map_err(|e| {
      DaemonError::RestSource(format!(
        "decode chainlock block hash {}: {e}",
        chainlock.blockhash
      ))
    })?,
    block_height: Some(chainlock.height as i32),
    core_height: Some(chainlock.height as i32),
    raw_chainlock: hex::decode(&chainlock.hex).map_err(|e| {
      DaemonError::RestSource(format!(
        "decode chainlock hex for {}: {e}",
        chainlock.blockhash
      ))
    })?,
    seen_at: now_epoch_secs(),
  };

  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin chainlock snapshot tx: {e}")))?;
  sql::live::chainlock::upsert_snapshot(&mut tx, &row).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit chainlock snapshot tx: {e}")))?;
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
  pool: SqlPool,
  rpc: Arc<dyn RpcSource>,
  lifecycle: Arc<Lifecycle>,
  refresh_secs: u64,
  start_block: u32,
  stream_hub: Option<Arc<StreamHub>>,
) {
  while lifecycle.running() {
    if let Err(err) = refresh_once(&pool, rpc.as_ref(), start_block, stream_hub.as_deref()).await {
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
