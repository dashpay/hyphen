//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live ChainLock state backed by node JSON-RPC snapshots.

pub(crate) mod rpc;

pub use rpc::refresh_loop;
