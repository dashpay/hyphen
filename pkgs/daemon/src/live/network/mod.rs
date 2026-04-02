//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live network state backed by node JSON-RPC snapshots.

#[cfg(feature = "tests")]
pub mod rest;
#[cfg(not(feature = "tests"))]
pub(crate) mod rest;
pub(crate) mod rpc;

pub use rpc::refresh_loop;
