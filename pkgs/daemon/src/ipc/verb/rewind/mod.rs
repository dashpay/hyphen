//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Operator-initiated rewind verbs.
//!
//! Both verbs go through [`hyphen_db::HyDbIngest::rewind_to`], which
//! disconnects canonical blocks tip-first via the same undo machinery a reorg
//! uses. The raw blocks stay absorbed, so the ingest loop reconnects them on
//! its next cycle unless the node also rewound.
//!
//! `target_height` arrives off the wire and is *not* checked here. It used to
//! be checked nowhere at all; it is now bounded inside `rewind_to`, against
//! `[db.ingest] max_reorg_depth` and against the undo rows actually on disk,
//! before that call mutates anything. That is the right place for it — the
//! bound is a property of the projection's retention, not of the IPC surface,
//! and a check here would be a second copy of it that could drift. What this
//! module owes the operator is that the refusal reaches them intact, target
//! and stopping height included, which is why the failure arm below repeats
//! the target rather than emitting a bare `{e}`.

use hyphen_db::HyDbHeight;

use super::{emit_command_progress, emit_command_result, CommandCtx};

/// Rewind the projection so its tip is `target_height`.
pub async fn rewind(ctx: &CommandCtx, id: u64, target_height: u32) {
  emit_command_progress(
    &ctx.event_tx,
    id,
    format!("rewind_started target={target_height}"),
  );
  match ctx.ingest.rewind_to(HyDbHeight(target_height)).await {
    Ok(report) => {
      let removed = report.reorg.map_or(0, |r| r.disconnected.len());
      emit_command_result(
        &ctx.event_tx,
        id,
        true,
        format!(
          "rewind_complete tip={} disconnected={removed}",
          report.to.map_or(-1, |h| i64::from(h.0))
        ),
      );
    }
    Err(e) => emit_command_result(
      &ctx.event_tx,
      id,
      false,
      format!("rewind to {target_height} failed: {e}"),
    ),
  }
}

/// Disconnect the single block at the current tip.
pub async fn disconnect_tip(ctx: &CommandCtx, id: u64) {
  let tip = match ctx.hydb.status().tip().await {
    Ok(tip) => tip,
    Err(e) => {
      emit_command_result(&ctx.event_tx, id, false, format!("read tip failed: {e}"));
      return;
    }
  };
  match tip {
    Some(tip) if tip.height.0 > 0 => rewind(ctx, id, tip.height.0 - 1).await,
    Some(_) => emit_command_result(
      &ctx.event_tx,
      id,
      false,
      "cannot disconnect the genesis block",
    ),
    None => emit_command_result(&ctx.event_tx, id, false, "projection is empty"),
  }
}
