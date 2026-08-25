//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live validator `protx` refresh backed by node JSON-RPC.

use std::collections::HashSet;
use std::sync::Arc;

use crate::error::{DaemonError, RestErrorKind};
use crate::lifecycle::Lifecycle;
use crate::live::state::{LiveState, ProTxEntry, ProtxSnapshot};
use crate::live::warn;
use crate::policy::api::API_REFRESH_WARN_INTERVAL_SECS;
use crate::sources::rpc::ProTxListEntry;
use crate::sources::RpcSource;
use crate::utils::{HashEndian, HashLookup};

/// Refresh the in-RAM validator `protx` snapshot once from node JSON-RPC.
///
/// # Errors
///
/// Returns `DaemonError::RestSource` when the RPC requests fail or a
/// pro_tx_hash cannot be decoded.
pub async fn refresh_once(live_state: &LiveState, rpc: &dyn RpcSource) -> Result<(), DaemonError> {
  let now = now_epoch_secs();
  let registered = rpc.get_protx_list_registered().await?;
  let valid = rpc.get_protx_list_valid().await?;
  let valid_hashes: HashSet<&str> = valid
    .iter()
    .map(|entry| entry.pro_tx_hash.as_str())
    .collect();
  let entries: Vec<ProTxEntry> = registered
    .into_iter()
    .map(|entry| snapshot_row(entry, &valid_hashes, now))
    .collect::<Result<Vec<_>, _>>()?;

  live_state.set_protx(ProtxSnapshot::from_entries(entries));
  Ok(())
}

/// Periodically refresh validator `protx` snapshot state until shutdown.
pub async fn refresh_loop(
  live_state: Arc<LiveState>,
  rpc: Arc<dyn RpcSource>,
  lifecycle: Arc<Lifecycle>,
  refresh_secs: u64,
) {
  while lifecycle.running() {
    if let Err(err) = refresh_once(&live_state, rpc.as_ref()).await {
      warn::warn_every(
        "live_protx_refresh_failed",
        std::time::Duration::from_secs(API_REFRESH_WARN_INTERVAL_SECS),
        |suppressed| {
          tracing::warn!(suppressed, "validator protx refresh failed: {err}");
        },
      );
    }
    lifecycle
      .sleep(std::time::Duration::from_secs(refresh_secs))
      .await;
  }
}

fn snapshot_row(
  entry: ProTxListEntry,
  valid_hashes: &HashSet<&str>,
  updated_at: i64,
) -> Result<ProTxEntry, DaemonError> {
  let pro_tx_lookup =
    HashLookup::from_hex(&entry.pro_tx_hash).map_err(|e| DaemonError::RestSource {
      message: format!("decode validator protx hash {}: {e}", entry.pro_tx_hash),
      kind: RestErrorKind::Other,
    })?;
  Ok(ProTxEntry {
    pro_tx_hash: pro_tx_lookup.bytes(HashEndian::Ser).to_vec(),
    validator_type: entry.validator_type.clone(),
    current_status: Some(current_status(&entry, valid_hashes)),
    last_paid_height: entry
      .state
      .as_ref()
      .and_then(|state| state.last_paid_height),
    consecutive_payments: entry
      .state
      .as_ref()
      .and_then(|state| state.consecutive_payments),
    pose_penalty: entry.state.as_ref().and_then(|state| state.pose_penalty),
    pose_revived_height: entry
      .state
      .as_ref()
      .and_then(|state| state.pose_revived_height),
    pose_ban_height: entry.state.as_ref().and_then(|state| state.pose_ban_height),
    revocation_reason: entry
      .state
      .as_ref()
      .and_then(|state| state.revocation_reason),
    updated_at,
  })
}

fn current_status(entry: &ProTxListEntry, valid_hashes: &HashSet<&str>) -> String {
  if valid_hashes.contains(entry.pro_tx_hash.as_str()) {
    return "valid".to_owned();
  }
  if entry
    .state
    .as_ref()
    .and_then(|state| state.pose_ban_height)
    .is_some_and(|height| height >= 0)
  {
    return "pose_banned".to_owned();
  }
  if entry
    .state
    .as_ref()
    .and_then(|state| state.revocation_reason)
    .is_some_and(|reason| reason != 0)
  {
    return "revoked".to_owned();
  }
  "registered".to_owned()
}

fn now_epoch_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs() as i64
}

#[cfg(test)]
mod tests {
  use super::{current_status, snapshot_row};
  use crate::sources::rpc::{ProTxListEntry, ProTxState};
  use std::collections::HashSet;

  #[test]
  fn current_status_prefers_valid_membership() {
    let entry = ProTxListEntry {
      pro_tx_hash: "11".repeat(32),
      validator_type: None,
      state: Some(ProTxState {
        last_paid_height: None,
        consecutive_payments: None,
        pose_penalty: None,
        pose_revived_height: None,
        pose_ban_height: Some(10),
        revocation_reason: Some(1),
        service: None,
      }),
    };
    let valid = HashSet::from([entry.pro_tx_hash.as_str()]);

    assert_eq!(current_status(&entry, &valid), "valid");
  }

  #[test]
  fn snapshot_row_keeps_rpc_state_fields() {
    let entry = ProTxListEntry {
      pro_tx_hash: "11".repeat(32),
      validator_type: Some("Evo".to_owned()),
      state: Some(ProTxState {
        last_paid_height: Some(42),
        consecutive_payments: Some(2),
        pose_penalty: Some(1),
        pose_revived_height: Some(8),
        pose_ban_height: None,
        revocation_reason: None,
        service: None,
      }),
    };

    let row = snapshot_row(entry, &HashSet::new(), 77).expect("snapshot row should decode");
    assert_eq!(row.pro_tx_hash, vec![0x11; 32]);
    assert_eq!(row.validator_type.as_deref(), Some("Evo"));
    assert_eq!(row.current_status.as_deref(), Some("registered"));
    assert_eq!(row.last_paid_height, Some(42));
    assert_eq!(row.consecutive_payments, Some(2));
    assert_eq!(row.pose_penalty, Some(1));
    assert_eq!(row.pose_revived_height, Some(8));
    assert_eq!(row.updated_at, 77);
  }

  #[test]
  fn snapshot_row_reorders_view_hash_to_serialized_bytes() {
    let entry = ProTxListEntry {
      pro_tx_hash: "00112233".repeat(8),
      validator_type: None,
      state: None,
    };

    let row = snapshot_row(entry, &HashSet::new(), 77).expect("snapshot row should decode");
    assert_eq!(
      row.pro_tx_hash,
      crate::utils::decode_hex_to(&"00112233".repeat(8), crate::utils::HashEndian::Ser)
        .expect("hex should decode"),
    );
  }
}
