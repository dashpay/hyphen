//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Benchmark entrypoint for daemon-owned benchmark definitions.

mod index;
mod ingest;
mod live;
mod spent;
mod write;

fn main() {
  if let Err(error) = hyphend::bench::run_with(|| divan::Divan::default().main()) {
    match error {
      hyphend::error::DaemonError::Config(message) => eprintln!("{message}"),
      other => eprintln!("{other}"),
    }
    std::process::exit(1);
  }
}
