//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CLI argument parsing.

use std::path::PathBuf;

use clap::Parser;

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
  #[arg(long = "docs-only")]
  pub docs_only: bool,

  /// Print the storage layout recorded in the database next to the configured
  /// one, then exit. Diagnosing a layout refusal should not require a psql
  /// session: this connects with `[db.client]`, reads `init_params`, and
  /// shows every key's stored value, configured value, grade and what startup
  /// would do with it. Read-only — it never writes or migrates.
  #[arg(long = "show-layout")]
  pub show_layout: bool,

  /// Start serving without settling the vacuum debt a sync leaves behind.
  ///
  /// A sync writes its height partitions with autovacuum off, because an open
  /// partition is append-only and vacuuming it only competes with the ingest
  /// for the device. Startup normally pays that back before serving, which on
  /// a large store takes minutes. This skips it for the case where the node
  /// has to be up now; the cost is bloat and stale statistics until
  /// autovacuum catches up on its own, never incorrectness.
  #[arg(long = "skip-compacting")]
  pub skip_compacting: bool,
}

impl Args {
  /// Parse daemon CLI arguments from `std::env::args`.
  pub fn parse_args() -> Self {
    Self::parse()
  }
}
