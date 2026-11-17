//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live InstantSend lock state backed by node ZMQ events.

pub(crate) mod state;
pub(crate) mod zmq;

pub use zmq::sync_current_loop;

use crate::rest::openapi::{TxLive, TxLiveTags};

/// Live overlay for a row that only reports lock state: `islock: true` when a
/// lock is known, and no tags otherwise. Lock state is never reported false.
pub(crate) fn lock_overlay(locked: bool) -> TxLive {
  TxLive {
    tags: locked.then_some(TxLiveTags {
      mempool: None,
      islock: Some(true),
    }),
  }
}
