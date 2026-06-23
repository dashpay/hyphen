//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CLI argument parsing.

use std::path::PathBuf;

use clap::{Parser, ValueEnum};

/// One or all secondary RocksDB stores.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum StoreTarget {
  All,
  Address,
  Prevout,
}

/// One or all PostgreSQL index stages.
#[derive(Clone, Copy, Debug, Eq, PartialEq, ValueEnum)]
pub enum IndexTarget {
  All,
  Block,
  Transaction,
  Script,
  Spent,
  Metrics,
  Validator,
}

impl IndexTarget {
  pub fn as_stage_name(self) -> Option<&'static str> {
    match self {
      Self::All => None,
      Self::Block => Some("block"),
      Self::Transaction => Some("transaction"),
      Self::Script => Some("script"),
      Self::Spent => Some("spent"),
      Self::Metrics => Some("metrics"),
      Self::Validator => Some("validator"),
    }
  }
}

const MAINTENANCE_FLAGS: &[&str] = &[
  "rebuild_index",
  "rebuild_store",
  "verify_index",
  "verify_store",
  "compact_store",
  "rewind_all",
];

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

  /// Serve only the Swagger UI and OpenAPI document, then exit on shutdown.
  #[arg(long = "docs-only", conflicts_with_all = MAINTENANCE_FLAGS)]
  pub docs_only: bool,

  /// Rebuild one or all PostgreSQL index stages from RocksDB, then exit.
  #[arg(
    long = "rebuild-index",
    value_enum,
    conflicts_with_all = ["rebuild_store", "verify_index", "verify_store", "compact_store", "rewind_all"],
  )]
  pub rebuild_index: Option<IndexTarget>,

  /// Rebuild one or all secondary RocksDB stores from the raw block store, then exit.
  #[arg(
    long = "rebuild-store",
    value_enum,
    conflicts_with_all = ["rebuild_index", "verify_index", "verify_store", "compact_store", "rewind_all"],
  )]
  pub rebuild_store: Option<StoreTarget>,

  /// Verify one or all PostgreSQL index stages against RocksDB, then exit.
  #[arg(
    long = "verify-index",
    value_enum,
    conflicts_with_all = ["rebuild_index", "rebuild_store", "verify_store", "compact_store", "rewind_all"],
  )]
  pub verify_index: Option<IndexTarget>,

  /// Deep-verify one or all secondary RocksDB stores against the raw block store, then exit.
  #[arg(
    long = "verify-store",
    value_enum,
    conflicts_with_all = ["rebuild_index", "rebuild_store", "verify_index", "compact_store", "rewind_all"],
  )]
  pub verify_store: Option<StoreTarget>,

  /// Compact one or all secondary RocksDB stores, then exit.
  #[arg(
    long = "compact-store",
    value_enum,
    conflicts_with_all = ["rebuild_index", "rebuild_store", "verify_index", "verify_store", "rewind_all"],
  )]
  pub compact_store: Option<StoreTarget>,

  /// Rewind all databases to the lowest consistent height (or to an explicit
  /// target height), then exit.
  #[arg(
    long = "rewind-all",
    num_args = 0..=1,
    value_name = "HEIGHT",
    conflicts_with_all = ["rebuild_index", "rebuild_store", "verify_index", "verify_store", "compact_store"],
  )]
  pub rewind_all: Option<Option<u32>>,
}

impl Args {
  /// Parse daemon CLI arguments from `std::env::args`.
  pub fn parse_args() -> Self {
    Self::parse()
  }
}
