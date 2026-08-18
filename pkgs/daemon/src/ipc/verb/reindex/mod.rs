//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The retired per-stage reindex verb.
//!
//! The unified storage engine derives every index atomically per block —
//! there is no per-stage state that can drift and need rebuilding. A targeted
//! rebuild is therefore meaningless; the supported recovery is a rewind
//! (bounded) or a resync against a fresh database (full). The verb stays on
//! the wire so older consoles get an honest answer instead of silence.

use super::{emit_command_result, CommandCtx};

pub fn reindex(ctx: &CommandCtx, id: u64, stage: String) {
  emit_command_result(
    &ctx.event_tx,
    id,
    false,
    format!(
      "reindex of stage `{stage}` is not applicable: the unified store derives all \
       indexes atomically per block; use rewind, or resync against a fresh database"
    ),
  );
}
