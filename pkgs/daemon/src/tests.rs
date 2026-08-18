//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared test support exposed behind the `tests` feature gate.

use std::sync::Arc;

use crate::rest::{AppState, IndexCapabilities, LiveCapabilities, OverlayCapabilities};

/// Build one minimal REST app state for handler tests.
///
/// The storage engine defaults to an empty in-memory [`MemDb`] fake; individual
/// tests replace `state.hydb` with a seeded fake as needed.
///
/// [`MemDb`]: crate::rest::hydb_fake::MemDb
pub fn app_state() -> AppState {
  AppState {
    hydb: crate::rest::hydb_fake::MemDb::new().into_hydb(),
    p2pkh_version: 76,
    p2sh_version: 16,
    block_time_secs: crate::config::DEFAULT_BLOCK_TIME_SECS,
    indices: IndexCapabilities {
      address: false,
      validator: false,
      metrics: false,
    },
    live: LiveCapabilities {
      mempool: false,
      islock: false,
      network: false,
      governance: false,
      stream: false,
    },
    overlays: OverlayCapabilities {
      protx: false,
      chainlock: false,
    },
    recent_mempool_txs: 0,
    stream_hub: None,
    live_state: Arc::new(crate::live::state::LiveState::new()),
    stream_heartbeat_secs: 1,
    node_tip: Arc::new(std::sync::atomic::AtomicI64::new(-1)),
    ingest_progress_epoch: Arc::new(std::sync::atomic::AtomicU64::new(0)),
  }
}
