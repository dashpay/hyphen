//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Read-capability stage metadata.
//!
//! The storage engine (`hyphen-db`) advances all derivation atomically per
//! block, so there is no longer a multi-stage pipeline — these stages are just
//! the API's capability gates (which read routes require which optional index).

/// Which optional indexes are enabled for this daemon.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IndexStageFlags {
  pub address: bool,
  pub metrics: bool,
  pub validator: bool,
}

/// Read-capability stages, one per API domain that can be gated behind an
/// optional index.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
  Block,
  Transaction,
  Script,
  Spent,
  Metrics,
  Validator,
}

impl Stage {
  /// Parse the wire/name form for a stage.
  pub(crate) fn from_name(stage: &str) -> Option<Self> {
    match stage {
      "block" => Some(Self::Block),
      "transaction" => Some(Self::Transaction),
      "script" => Some(Self::Script),
      "spent" => Some(Self::Spent),
      "metrics" => Some(Self::Metrics),
      "validator" => Some(Self::Validator),
      _ => None,
    }
  }

  /// The canonical lowercase stage name.
  pub(crate) fn name(self) -> &'static str {
    match self {
      Self::Block => "block",
      Self::Transaction => "transaction",
      Self::Script => "script",
      Self::Spent => "spent",
      Self::Metrics => "metrics",
      Self::Validator => "validator",
    }
  }

  /// Whether this stage's route is served for the current feature flags.
  pub(crate) fn enabled_with_flags(self, flags: IndexStageFlags) -> bool {
    match self {
      Self::Block | Self::Transaction => true,
      Self::Script | Self::Spent => flags.address,
      Self::Metrics => flags.metrics,
      Self::Validator => flags.validator,
    }
  }
}
