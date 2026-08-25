//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! CLI argument parsing for the TUI console.

use std::path::PathBuf;

use clap::Parser;

#[derive(Debug, Parser)]
#[command(name = "hyphen-tui", about = "Hyphen daemon TUI console")]
pub struct Args {
  /// Override the data directory path.
  #[arg(short, long)]
  pub datadir: Option<PathBuf>,

  /// REST API URL for the browse panel.
  ///
  /// Normally unnecessary: the console takes the endpoint from the daemon's
  /// own reported configuration, which is the only source that stays right
  /// when an operator moves the bind. Set this only to browse a daemon other
  /// than the one whose datadir this console is reading.
  #[arg(long)]
  pub api_url: Option<String>,
}

impl Args {
  /// Parse console CLI arguments from `std::env::args`.
  pub fn parse_args() -> Self {
    Self::parse()
  }
}
