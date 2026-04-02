//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block-stage IPC snapshot helpers.

use crate::index::batch::BatchSizerSnapshot;
use crate::index::Stage;
use crate::sql::query::SyncStateRow;
use hyphend::wire::msg::{BatchAdjustment, StageProgress};

pub(crate) fn progress_row(
  row: &SyncStateRow,
  network_tip: Option<u32>,
  batch: Option<&BatchSizerSnapshot>,
) -> StageProgress {
  let (stage_type, display_name, _) = Stage::Block.meta();
  let (batch_size, last_batch_ms, batch_history) = batch
    .map(|snapshot| {
      let history = snapshot
        .history
        .iter()
        .map(|&(elapsed_ms, prev, next)| BatchAdjustment {
          elapsed_ms,
          prev,
          next,
        })
        .collect();
      (Some(snapshot.current), snapshot.last_batch_ms, history)
    })
    .unwrap_or((None, None, vec![]));

  StageProgress {
    stage: row.stage.clone(),
    stage_type: stage_type.to_owned(),
    display_name: display_name.to_owned(),
    indexed_height: row.indexed_height,
    tip_height: network_tip.unwrap_or(0) as i32,
    updated_at: row.updated_at,
    gate: None,
    enabled: true,
    error: None,
    batch_size,
    last_batch_ms,
    batch_history,
  }
}
