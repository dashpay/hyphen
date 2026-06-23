//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Batch fetch task.

use std::time::Duration;

use crate::error::{DaemonError, RestErrorKind};

use super::state::{FetchedBlock, InFlightBatch};
use super::IngestCtx;

const FETCH_BATCH_TIMEOUT_SECS: u64 = 120;

/// Fetch one admitted block batch from the upstream REST source.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when the fetch times out or the upstream
/// REST source rejects one of the requested heights.
pub async fn fetch_batch(
  ctx: &IngestCtx,
  batch: &InFlightBatch,
) -> Result<Vec<FetchedBlock>, DaemonError> {
  ctx.runtime.record_fetch_start(batch);
  tracing::debug!(
    attempt = batch.attempt,
    start = batch.start_height,
    end = batch.end_height,
    "fetching ingest batch"
  );
  let heights: Vec<u32> = (batch.start_height..=batch.end_height).collect();
  let rows = tokio::time::timeout(
    Duration::from_secs(FETCH_BATCH_TIMEOUT_SECS),
    ctx.source.fetch_blocks_batch(&heights),
  )
  .await
  .map_err(|_| {
    DaemonError::RestSource {
      message: format!(
        "ingest batch fetch timed out for heights {}..={} after {}s",
        batch.start_height, batch.end_height, FETCH_BATCH_TIMEOUT_SECS
      ),
      kind: RestErrorKind::Timeout,
    }
  })??;
  tracing::debug!(
    attempt = batch.attempt,
    start = batch.start_height,
    end = batch.end_height,
    "ingest batch fetch complete"
  );
  Ok(
    rows
      .into_iter()
      .map(|(_hash_hex, raw)| FetchedBlock { raw })
      .collect(),
  )
}
