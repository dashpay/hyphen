//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Upstream tip polling.

use crate::error::DaemonError;

use super::IngestCtx;

/// Poll the upstream chain height once.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when the upstream REST source cannot
/// return the current block count.
pub async fn poll_tip(ctx: &IngestCtx) -> Result<u32, DaemonError> {
  let tip = ctx.source.block_count().await?;
  ctx.runtime.record_polled_tip(tip);
  Ok(tip)
}
