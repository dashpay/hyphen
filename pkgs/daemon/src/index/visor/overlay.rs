//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Indexed overlay coordination.
//!
//! Indexed-domain overlays stay owned by `crate::index`, even when they consume
//! live-backed SQL snapshots. This module resolves the overlay runtime plan
//! that the rest of the daemon consumes instead of reconstructing overlay
//! policy ad hoc in `main.rs`.

use crate::init::Options;
use crate::rest::OverlayCapabilities;

/// Resolved overlay requirements for live-backed refresh inputs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayLiveRequirements {
  pub chainlock: bool,
  pub protx: bool,
}

/// Resolved overlay plan shared by the daemon runtime.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OverlayPlan {
  pub capabilities: OverlayCapabilities,
  pub live_requirements: OverlayLiveRequirements,
}

impl OverlayPlan {
  /// Resolve overlay policy from validated startup options.
  pub fn from_options(options: &Options) -> Self {
    let capabilities = OverlayCapabilities {
      protx: options.overlays.protx,
      chainlock: options.overlays.chainlock,
    };
    Self {
      capabilities,
      live_requirements: OverlayLiveRequirements {
        chainlock: options.live.chainlock || options.overlays.chainlock,
        protx: options.overlays.protx,
      },
    }
  }
}

#[cfg(test)]
mod tests {
  use super::OverlayPlan;
  use crate::init::options::{
    ApiOptions, IndexOptions, LiveOptions, Options, OverlayOptions, StartupMode,
  };

  fn test_options() -> Options {
    Options {
      startup_mode: StartupMode::Serve,
      indices: IndexOptions {
        address: false,
        validator: false,
        metrics: false,
      },
      live: LiveOptions {
        mempool: false,
        islock: false,
        chainlock: false,
        governance: false,
        network: false,
        stream: false,
      },
      overlays: OverlayOptions {
        protx: false,
        chainlock: false,
      },
      api: ApiOptions {
        enabled: false,
        bind: None,
        cors_origin: None,
      },
      recent_mempool_txs: 0,
      stream_client_buffer: 1,
      stream_heartbeat_secs: 1,
      chainlock_start_block: None,
      network_refresh_secs: 1,
      governance_refresh_secs: 1,
      chainlock_refresh_secs: 1,
      protx_refresh_secs: 1,
    }
  }

  #[test]
  fn overlay_plan_promotes_chainlock_overlay_to_live_requirement() {
    let mut options = test_options();
    options.live.chainlock = false;
    options.overlays.chainlock = true;

    let plan = OverlayPlan::from_options(&options);
    assert!(plan.capabilities.chainlock);
    assert!(plan.live_requirements.chainlock);
  }

  #[test]
  fn overlay_plan_tracks_protx_overlay_capability_and_requirement() {
    let mut options = test_options();
    options.overlays.protx = true;

    let plan = OverlayPlan::from_options(&options);
    assert!(plan.capabilities.protx);
    assert!(plan.live_requirements.protx);
  }
}
