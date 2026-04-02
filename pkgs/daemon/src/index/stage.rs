//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Canonical index stage metadata and ordering rules.

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) struct IndexStageFlags {
  pub address: bool,
  pub metrics: bool,
  pub validator: bool,
}

/// Canonical stages that make up the PostgreSQL indexing pipeline.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Stage {
  Block,
  Transaction,
  Script,
  Spent,
  Metrics,
  Validator,
}

const CONNECT_STAGE_ORDER: &[Stage] = &[
  Stage::Block,
  Stage::Transaction,
  Stage::Script,
  Stage::Spent,
  Stage::Metrics,
  Stage::Validator,
];

const DISCONNECT_STAGE_ORDER: &[Stage] = &[
  Stage::Spent,
  Stage::Script,
  Stage::Validator,
  Stage::Metrics,
  Stage::Transaction,
  Stage::Block,
];

/// Return enabled non-block stages in connect order for the current feature
/// flags.
pub(crate) fn enabled_stage_order(flags: IndexStageFlags) -> Vec<Stage> {
  CONNECT_STAGE_ORDER
    .iter()
    .copied()
    .filter(|stage| match *stage {
      Stage::Block => false,
      Stage::Transaction => true,
      Stage::Script | Stage::Spent => flags.address,
      Stage::Metrics => flags.metrics,
      Stage::Validator => flags.validator,
    })
    .collect()
}

/// Return the connect order restricted to the currently enabled stages.
pub(crate) fn connect_stage_order(enabled_stages: &[Stage]) -> Vec<Stage> {
  CONNECT_STAGE_ORDER
    .iter()
    .copied()
    .filter(|stage| stage.enabled_for(enabled_stages))
    .collect()
}

/// Return the connect order restricted to the exact provided stages.
pub(crate) fn exact_stage_order(stages: &[Stage]) -> Vec<Stage> {
  CONNECT_STAGE_ORDER
    .iter()
    .copied()
    .filter(|stage| stages.contains(stage))
    .collect()
}

/// Return the fixed disconnect order used during rewinds.
pub(crate) fn disconnect_stage_order() -> &'static [Stage] {
  DISCONNECT_STAGE_ORDER
}

impl Stage {
  /// Parse the persisted or wire name for a stage.
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

  /// Return the canonical lowercase stage name used in SQL and IPC.
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

  /// Return whether this stage is active for the current enabled stage list.
  pub(crate) fn enabled_for(self, enabled_stages: &[Stage]) -> bool {
    self == Self::Block || enabled_stages.contains(&self)
  }

  /// Return whether this stage is active for the current feature flags.
  pub(crate) fn enabled_with_flags(self, flags: IndexStageFlags) -> bool {
    match self {
      Self::Block | Self::Transaction => true,
      Self::Script | Self::Spent => flags.address,
      Self::Metrics => flags.metrics,
      Self::Validator => flags.validator,
    }
  }

  /// Return a stage-specific durable height query when one exists.
  ///
  /// Stages that derive their height from another table or from `sync_state`
  /// return `None`.
  pub(crate) fn durable_height_query(self) -> Option<&'static str> {
    match self {
      Self::Block => Some("SELECT COALESCE(MAX(height), -1) FROM blocks"),
      Self::Transaction => Some("SELECT COALESCE(MAX(block_height), -1) FROM transactions"),
      Self::Metrics => Some("SELECT COALESCE(MAX(height), -1) FROM block_metrics"),
      Self::Script | Self::Spent | Self::Validator => None,
    }
  }

  /// Return `(kind, display_name, gate_name)` metadata for console and REST
  /// consumers.
  pub(crate) fn meta(self) -> (&'static str, &'static str, Option<&'static str>) {
    match self {
      Self::Block => ("Basic", "Block Index", None),
      Self::Transaction => ("Basic", "Transaction Index", Some("block")),
      Self::Metrics => ("Feature", "Metrics", Some("block")),
      Self::Validator => ("Feature", "Validators", Some("block")),
      Self::Script => ("Feature", "Script", Some("transaction")),
      Self::Spent => ("Feature", "Spent / Address", Some("script")),
    }
  }

  /// Return the prerequisite stage that must be caught up before this stage can
  /// run.
  pub(crate) fn gate_stage(self) -> Option<Self> {
    match self {
      Self::Block => None,
      Self::Transaction | Self::Metrics | Self::Validator => Some(Self::Block),
      Self::Script => Some(Self::Transaction),
      Self::Spent => Some(Self::Script),
    }
  }

  /// Return the SQL tables owned by this stage for targeted resets.
  pub(crate) fn tables(self) -> &'static [&'static str] {
    match self {
      Self::Block => &["blocks", "orphan_blocks"],
      Self::Transaction => &[
        "transactions",
        "transactions_summary",
        "tx_inputs",
        "tx_outputs",
        "special_tx_payloads",
      ],
      Self::Metrics => &["block_metrics"],
      Self::Validator => &["validators", "validator_history", "validator_counts"],
      Self::Script => &["script_output_history"],
      Self::Spent => &[
        "address_balances",
        "address_balances_summary",
        "address_stats",
        "spent_output_history",
      ],
    }
  }

  /// Return the stages that must be reset when this stage is rebuilt.
  pub(crate) fn reset_stages(self) -> &'static [Stage] {
    match self {
      Self::Block => &[
        Stage::Spent,
        Stage::Script,
        Stage::Validator,
        Stage::Metrics,
        Stage::Transaction,
        Stage::Block,
      ],
      Self::Transaction => &[Stage::Spent, Stage::Script, Stage::Transaction],
      Self::Metrics => &[Stage::Metrics],
      Self::Validator => &[Stage::Validator],
      Self::Script => &[Stage::Spent, Stage::Script],
      Self::Spent => &[Stage::Spent],
    }
  }
}
