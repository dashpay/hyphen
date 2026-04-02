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
  #[arg(long, default_value = "http://127.0.0.1:3080")]
  pub api_url: String,
}

impl Args {
  /// Parse console CLI arguments from `std::env::args`.
  pub fn parse_args() -> Self {
    Self::parse()
  }
}
