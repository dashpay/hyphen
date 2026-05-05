//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! sqlx enum type mappings for PostgreSQL custom types.

use std::fmt;

/// PostgreSQL `tx_type` enum -- mirrors the DDL in `daemon_pg.sql`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, sqlx::Type, serde::Serialize)]
#[sqlx(type_name = "tx_type", rename_all = "snake_case")]
#[serde(rename_all = "snake_case")]
pub enum TxTypeDb {
  Normal,
  ProviderRegister,
  ProviderUpdateService,
  ProviderUpdateRegistrar,
  ProviderUpdateRevoke,
  Coinbase,
  QuorumCommitment,
  AssetLock,
  AssetUnlock,
  Mnhf,
}

impl fmt::Display for TxTypeDb {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    let s = match self {
      Self::Normal => "normal",
      Self::ProviderRegister => "provider_register",
      Self::ProviderUpdateService => "provider_update_service",
      Self::ProviderUpdateRegistrar => "provider_update_registrar",
      Self::ProviderUpdateRevoke => "provider_update_revoke",
      Self::Coinbase => "coinbase",
      Self::QuorumCommitment => "quorum_commitment",
      Self::AssetLock => "asset_lock",
      Self::AssetUnlock => "asset_unlock",
      Self::Mnhf => "mnhf",
    };
    f.write_str(s)
  }
}

impl From<dash_primitives::TxType> for TxTypeDb {
  fn from(t: dash_primitives::TxType) -> Self {
    use dash_primitives::TxType;
    match t {
      TxType::Spend | TxType::Unknown(_) => Self::Normal,
      TxType::ProviderRegister => Self::ProviderRegister,
      TxType::ProviderUpdateService => Self::ProviderUpdateService,
      TxType::ProviderUpdateRegistrar => Self::ProviderUpdateRegistrar,
      TxType::ProviderUpdateRevoke => Self::ProviderUpdateRevoke,
      TxType::CoinbaseCommitment => Self::Coinbase,
      TxType::QuorumCommitment => Self::QuorumCommitment,
      TxType::AssetLock => Self::AssetLock,
      TxType::AssetUnlock => Self::AssetUnlock,
      TxType::MnhfSignal => Self::Mnhf,
    }
  }
}
