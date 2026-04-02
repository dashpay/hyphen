//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live governance proposal state backed by node JSON-RPC.

pub(crate) mod rest;
pub(crate) mod rpc;
mod state;
pub(crate) mod zmq;

pub use rpc::refresh_loop;
pub use zmq::refresh_from_zmq_loop;
