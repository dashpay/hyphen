//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CLI argument parsing.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum StoreTargetArg {
  All,
  AddressHistory,
  PrevoutLookup,
}

/// One index stage accepted by the CLI reindex entrypoint.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum ReindexTarget {
  Block,
  Transaction,
  Script,
  Spent,
  Metrics,
  Validator,
}

impl ReindexTarget {
  pub fn as_stage_name(self) -> &'static str {
    match self {
      Self::Block => "block",
      Self::Transaction => "transaction",
      Self::Script => "script",
      Self::Spent => "spent",
      Self::Metrics => "metrics",
      Self::Validator => "validator",
    }
  }
}

/// Hyphen block explorer daemon.
#[derive(Debug, Parser)]
#[command(name = "hyphend", version)]
pub struct Args {
  /// Data directory path.
  /// Defaults to the platform-appropriate location.
  #[arg(short = 'd', long = "datadir")]
  pub datadir: Option<PathBuf>,

  /// Console log verbosity (0=error .. 3=debug).
  #[arg(
    long = "loglevel",
    default_value = "2",
    value_parser = clap::value_parser!(u8).range(0..=3),
  )]
  pub loglevel: u8,

  /// Also emit logs to stdout.
  #[arg(long = "logconsole")]
  pub logconsole: bool,

  /// Reindex one PostgreSQL stage and any dependent downstream stages, then exit.
  #[arg(long = "reindex", conflicts_with = "reindex_all", value_enum)]
  pub reindex: Option<ReindexTarget>,

  /// Reset and rebuild the entire PostgreSQL schema, then exit.
  #[arg(long = "reindex-all", conflicts_with = "reindex")]
  pub reindex_all: bool,

  /// Rebuild one or all secondary RocksDB stores from the raw block store, then exit.
  #[arg(
    long = "rebuild-store",
    value_enum,
    conflicts_with_all = ["reindex", "reindex_all", "verify_store"]
  )]
  pub rebuild_store: Option<StoreTargetArg>,

  /// Deep-verify one or all secondary RocksDB stores against the raw block store, then exit.
  #[arg(
    long = "verify-store",
    value_enum,
    conflicts_with_all = ["reindex", "reindex_all", "rebuild_store"]
  )]
  pub verify_store: Option<StoreTargetArg>,
}

impl Args {
  /// Parse daemon CLI arguments from `std::env::args`.
  pub fn parse_args() -> Self {
    Self::parse()
  }
}
