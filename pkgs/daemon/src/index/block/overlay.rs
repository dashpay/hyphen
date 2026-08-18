//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block overlay helpers that enrich indexed routes with live ChainLock facts.

use std::sync::Arc;

use crate::live::state::ChainLockSnapshot;
use crate::rest::error::ApiError;
use crate::rest::openapi::{BlockLive, BlockLiveTags};
use crate::rest::{rfc3339_time, AppState, OverlayFeature};
use crate::utils::{self, HashEndian};

/// Read the latest ChainLock from in-RAM live state (an infallible cheap `Arc`
/// clone); `None` when the ChainLock overlay is disabled or nothing seen yet.
pub(crate) fn fetch_chainlock_snapshot(state: &AppState) -> Option<Arc<ChainLockSnapshot>> {
  if !state.overlay_enabled(OverlayFeature::Chainlock) {
    return None;
  }
  state.live_state.chainlock()
}

pub(crate) fn block_live_from_snapshot(
  snapshot: Option<&ChainLockSnapshot>,
  block_hash: &[u8],
) -> Result<BlockLive, ApiError> {
  let Some(snapshot) = snapshot else {
    return Ok(BlockLive::default());
  };
  let chainlock = Some(
    utils::view_endian(&snapshot.block_hash, HashEndian::View)
      == utils::view_endian(block_hash, HashEndian::Ser),
  );
  Ok(BlockLive {
    tags: Some(BlockLiveTags { chainlock }),
    core_height: snapshot.core_height,
    seen_at: Some(rfc3339_time(snapshot.seen_at)?),
  })
}

#[cfg(test)]
mod tests {
  use crate::live::state::ChainLockSnapshot;

  // `app_state()` assembles a `HyphenDb` facade whose async accessors need a
  // Tokio runtime, so this stays a `tokio::test` even though the ChainLock path
  // itself is now synchronous RAM.
  #[tokio::test]
  async fn block_live_marks_matching_chainlock_from_ram() -> Result<(), String> {
    let mut state = crate::tests::app_state();
    state.overlays.chainlock = true;
    state.live_state.set_chainlock(ChainLockSnapshot {
      block_hash: vec![0x22; 32],
      block_height: Some(42),
      core_height: Some(42),
      raw_chainlock: vec![0xaa],
      seen_at: 120,
    });

    let snapshot = super::fetch_chainlock_snapshot(&state);
    let live =
      super::block_live_from_snapshot(snapshot.as_deref(), &[0x22; 32]).map_err(|e| e.message)?;
    assert_eq!(
      live.tags,
      Some(crate::rest::openapi::BlockLiveTags {
        chainlock: Some(true),
      })
    );
    assert_eq!(live.core_height, Some(42));
    assert_eq!(live.seen_at.as_deref(), Some("1970-01-01T00:02:00Z"));
    Ok(())
  }
}
