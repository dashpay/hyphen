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
}

impl Args {
  /// Parse daemon CLI arguments from `std::env::args`.
  pub fn parse_args() -> Self {
    Self::parse()
  }
}
