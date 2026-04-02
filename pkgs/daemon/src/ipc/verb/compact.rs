//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Force-compact a RocksDB column family.

use std::sync::Arc;

use tracing::{info, warn};

use super::CommandCtx;

/// Execute a RocksDB column-family compaction request from IPC.
pub async fn compact(ctx: CommandCtx, id: u64, cf_name: String) {
  let db2 = Arc::clone(&ctx.db);
  info!(cf = cf_name, "IPC: force_compact");
  super::emit_command_progress(&ctx.event_tx, id, format!("compact_started cf={cf_name}"));
  let (success, message) = tokio::task::spawn_blocking(move || match db2.cf_handle(&cf_name) {
    Some(cf_h) => {
      db2.compact_range_cf(cf_h, None::<&[u8]>, None::<&[u8]>);
      (true, format!("Compacted '{cf_name}'"))
    }
    None => (false, format!("Unknown column family '{cf_name}'")),
  })
  .await
  .unwrap_or_else(|e| (false, format!("compact task panicked: {e}")));
  if success {
    info!(%message, "IPC: force_compact complete");
    super::emit_command_progress(&ctx.event_tx, id, format!("compact_complete {message}"));
  } else {
    warn!(%message, "IPC: force_compact failed");
  }
  let _ = ctx
    .event_tx
    .send(hyphend::wire::msg::DaemonMessage::CommandResult {
      id,
      success,
      message,
    });
}
