//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live mempool state backed by JSON-RPC snapshots and ZMQ deltas.

pub mod query;
#[cfg(feature = "tests")]
pub mod rest;
#[cfg(not(feature = "tests"))]
pub(crate) mod rest;
pub(crate) mod rpc;
mod state;
pub(crate) mod zmq;

pub(crate) use rpc::{refresh_summary_loop, MempoolRefresh};
pub use zmq::sync_recent_loop;
