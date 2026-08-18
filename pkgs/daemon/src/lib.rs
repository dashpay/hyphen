//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared daemon library surfaces.
//!
//! The daemon crate primarily builds the `hyphend` binary, but it also exposes
//! the optional IPC wire contract used by external consumers such as
//! `hyphen-tui`.

#![cfg_attr(
  feature = "daemon",
  expect(
    dead_code,
    reason = "the library target mirrors daemon-owned binary modules for tests, benches, and sibling consumers"
  )
)]

extern crate self as hyphend;

#[cfg(feature = "daemon")]
mod api_metrics;
#[cfg(test)]
mod architecture;
#[cfg(feature = "daemon")]
mod config;
#[cfg(feature = "daemon")]
pub(crate) mod consensus;
#[cfg(feature = "daemon")]
mod datadir;
#[cfg(feature = "daemon")]
pub mod error;
#[cfg(feature = "daemon")]
pub mod index;
#[cfg(feature = "daemon")]
mod ingest;
#[cfg(feature = "daemon")]
pub mod init;
#[cfg(feature = "daemon")]
mod ipc;
#[cfg(feature = "daemon")]
mod lifecycle;
#[cfg(feature = "daemon")]
pub mod live;
#[cfg(feature = "daemon")]
mod lockfile;
#[cfg(feature = "daemon")]
mod policy;
#[cfg(all(feature = "daemon", feature = "tests"))]
pub mod rest;
#[cfg(all(feature = "daemon", not(feature = "tests")))]
mod rest;
#[cfg(all(feature = "daemon", feature = "tests"))]
pub mod sources;
#[cfg(all(feature = "daemon", not(feature = "tests")))]
mod sources;
#[cfg(all(feature = "daemon", any(test, feature = "tests")))]
pub mod tests;
#[cfg(feature = "daemon")]
pub mod tx;
#[cfg(feature = "daemon")]
pub mod utils;
#[cfg(feature = "daemon")]
mod visor;
#[cfg(feature = "daemon")]
mod worker;

/// Public wire contract for external consumers (e.g. hyphen-tui).
/// Enabled via the `wire` Cargo feature.
#[cfg(feature = "wire")]
pub mod wire;
