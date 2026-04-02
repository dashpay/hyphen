//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared node transport clients and source traits.

pub mod rest;
pub mod rpc;
mod traits;
pub mod zmq;

pub use self::rpc::CoreRpcClient;
pub use self::zmq::{CoreZmqSource, SequenceEvent, ZmqEvent};
pub use traits::{RestSource, RpcSource, ZmqSource};
