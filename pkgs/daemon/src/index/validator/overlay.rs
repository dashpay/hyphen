//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Thin validator overlay backed by persisted live `protx` snapshots.
//!
//! Overlay modules enrich indexed responses with nullable live fields only.
//! They must not duplicate facts already exposed by the indexed payload.

use std::collections::HashMap;

use crate::live::state::LiveState;
use crate::utils::{self, HashEndian};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ValidatorOverlay {
  pub current_type: Option<String>,
  pub current_status: Option<String>,
  pub last_paid_height: Option<i32>,
  pub consecutive_payments: Option<i32>,
  pub pose_penalty: Option<i32>,
  pub pose_revived_height: Option<i32>,
  pub pose_ban_height: Option<i32>,
  pub revocation_reason: Option<i32>,
}

pub fn fetch_overlay_map(live_state: &LiveState) -> HashMap<String, ValidatorOverlay> {
  let Some(snapshot) = live_state.protx() else {
    return HashMap::new();
  };
  snapshot
    .values()
    .map(|entry| {
      (
        utils::view_hex(&entry.pro_tx_hash, HashEndian::Ser),
        ValidatorOverlay {
          current_type: entry.validator_type.clone(),
          current_status: entry.current_status.clone(),
          last_paid_height: entry.last_paid_height,
          consecutive_payments: entry.consecutive_payments,
          pose_penalty: entry.pose_penalty,
          pose_revived_height: entry.pose_revived_height,
          pose_ban_height: entry.pose_ban_height,
          revocation_reason: entry.revocation_reason,
        },
      )
    })
    .collect()
}

#[cfg(test)]
mod tests {
  use super::ValidatorOverlay;
  use crate::live::state::{LiveState, ProTxEntry, ProtxSnapshot};
  use crate::utils::{self, HashEndian};

  fn entry(pro_tx_hash: Vec<u8>) -> ProTxEntry {
    ProTxEntry {
      pro_tx_hash,
      validator_type: Some("Evo".to_owned()),
      current_status: Some("valid".to_owned()),
      last_paid_height: Some(42),
      consecutive_payments: None,
      pose_penalty: None,
      pose_revived_height: None,
      pose_ban_height: None,
      revocation_reason: None,
      updated_at: 100,
    }
  }

  #[test]
  fn fetch_overlay_map_keys_serialized_hashes_by_public_hex() {
    let live = LiveState::new();
    // pro_tx_hash is stored serialized-endian; the overlay map is keyed by the
    // public view hex, so a lookup by that hex must resolve the entry.
    let ser: Vec<u8> = [0x00u8, 0x11, 0x22, 0x33].repeat(8);
    live.set_protx(ProtxSnapshot::from_entries(vec![entry(ser.clone())]));

    let overlay = super::fetch_overlay_map(&live);
    let public_hex = utils::view_hex(&ser, HashEndian::Ser);
    assert_eq!(
      overlay.get(&public_hex),
      Some(&ValidatorOverlay {
        current_type: Some("Evo".to_owned()),
        current_status: Some("valid".to_owned()),
        last_paid_height: Some(42),
        consecutive_payments: None,
        pose_penalty: None,
        pose_revived_height: None,
        pose_ban_height: None,
        revocation_reason: None,
      })
    );
  }

  #[test]
  fn fetch_overlay_map_empty_without_snapshot() {
    let live = LiveState::new();
    assert!(super::fetch_overlay_map(&live).is_empty());
  }
}
