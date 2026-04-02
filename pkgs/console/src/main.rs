//! Terminal UI for the Hyphen knowledge explorer daemon.

//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

mod app;
mod appview;
mod args;
mod error;
mod event;
mod ipc;
mod model;
mod pages;
mod rest;
mod sidebar;
mod status_bar;
pub mod theme;
mod widgets;

use args::Args;
use error::TuiError;

fn run() -> Result<(), TuiError> {
  tracing_subscriber::fmt::init();
  let args = Args::parse_args();
  let dir = match args.datadir {
    Some(p) => p,
    None => hyphend::wire::datadir::platform_datadir().ok_or(TuiError::NoPlatformDataDir)?,
  };
  tracing::info!(path = %dir.display(), "resolved datadir");
  app::run(&dir, &args.api_url)
}

fn main() {
  if let Err(e) = run() {
    eprintln!("error: {e}");
    std::process::exit(1);
  }
}
