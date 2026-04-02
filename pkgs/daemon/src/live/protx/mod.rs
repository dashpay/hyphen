//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live validator `protx` snapshot state backed by node JSON-RPC.

#[cfg(feature = "tests")]
pub mod rpc;
#[cfg(not(feature = "tests"))]
pub(crate) mod rpc;

pub use rpc::refresh_loop;
