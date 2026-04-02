//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block overlay helpers that enrich indexed routes with live ChainLock facts.

use crate::rest::error::ApiError;
use crate::rest::openapi::{BlockLive, BlockLiveTags};
use crate::rest::{rfc3339_time, AppState, OverlayFeature};
use crate::utils::{self, HashEndian};

pub(crate) async fn fetch_chainlock_snapshot(
  state: &AppState,
) -> Result<Option<crate::sql::live::chainlock::ChainLockSnapshotRow>, ApiError> {
  if !state.overlay_enabled(OverlayFeature::Chainlock) {
    return Ok(None);
  }
  crate::sql::live::chainlock::snapshot(&state.pool)
    .await
    .map_err(|e| ApiError::internal(format!("read chainlock snapshot: {e}")))
}

pub(crate) fn block_live_from_snapshot(
  snapshot: Option<&crate::sql::live::chainlock::ChainLockSnapshotRow>,
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
  #[tokio::test]
  async fn block_live_marks_matching_chainlock_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut tx = pool.begin().await?;
    crate::sql::live::chainlock::upsert_snapshot(
      &mut tx,
      &crate::sql::live::chainlock::ChainLockSnapshotRow {
        block_hash: vec![0x22; 32],
        block_height: Some(42),
        core_height: Some(42),
        raw_chainlock: vec![0xaa],
        seen_at: 120,
      },
    )
    .await?;
    tx.commit().await?;

    let mut state = crate::tests::app_state();
    state.address_pool = pool.clone();
    state.pool = pool;
    state.overlays.chainlock = true;

    let snapshot = super::fetch_chainlock_snapshot(&state)
      .await
      .map_err(|e| e.message)?;
    let live =
      super::block_live_from_snapshot(snapshot.as_ref(), &[0x22; 32]).map_err(|e| e.message)?;
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
