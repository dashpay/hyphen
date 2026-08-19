//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared conversions between the daemon's REST layer and the `hyphen-db`
//! storage facade — hash/height newtype bridging, error mapping, and enum
//! label mapping, reused by every repointed chain-read handler family.

use crate::rest::error::ApiError;

/// Convert a byte slice into a storage-layer 32-byte hash.
pub(crate) fn hydb_hash(bytes: &[u8]) -> Result<hyphen_db::HyDbHash, ApiError> {
  hyphen_db::HyDbHash::from_slice(bytes).map_err(|_| ApiError::bad_request("invalid hash length"))
}

/// Convert a height into the storage-layer height newtype (negatives clamp to
/// 0).
pub(crate) fn hydb_height(height: i32) -> hyphen_db::HyDbHeight {
  hyphen_db::HyDbHeight::new(height.max(0) as u32)
}

/// Map a storage-engine error onto an API error, preserving its class.
///
/// The typed `HyDbError` is carried into `DaemonError::Storage` rather than
/// formatted away, so `from_daemon` can grade it: transient storage failures
/// become a retriable `503` instead of an indistinguishable `500`.
pub(crate) fn hydb_err(err: hyphen_db::HyDbError, ctx: &'static str) -> ApiError {
  ApiError::from_daemon(crate::error::DaemonError::storage(ctx, err), ctx)
}

/// Public string label for a stored transaction type — matches the `tx_type`
/// enum's snake_case names.
pub(crate) fn tx_type_str(tx_type: hyphen_db::HyDbTxType) -> &'static str {
  use hyphen_db::HyDbTxType::{
    AssetLock, AssetUnlock, Coinbase, Mnhf, Normal, ProviderRegister, ProviderUpdateRegistrar,
    ProviderUpdateRevoke, ProviderUpdateService, QuorumCommitment,
  };
  match tx_type {
    Normal => "normal",
    ProviderRegister => "provider_register",
    ProviderUpdateService => "provider_update_service",
    ProviderUpdateRegistrar => "provider_update_registrar",
    ProviderUpdateRevoke => "provider_update_revoke",
    Coinbase => "coinbase",
    QuorumCommitment => "quorum_commitment",
    AssetLock => "asset_lock",
    AssetUnlock => "asset_unlock",
    Mnhf => "mnhf",
  }
}
