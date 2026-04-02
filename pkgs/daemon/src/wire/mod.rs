//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Console-facing wire contract and transport helpers.

#[cfg(feature = "wire")]
/// Delay between IPC client reconnect attempts.
pub const IPC_CLIENT_RETRY_DELAY_MS: u64 = 100;
/// Maximum accepted wire message size in bytes.
pub const IPC_MAX_MESSAGE_BYTES: usize = 64 * 1024;

pub mod codec;
pub mod msg;

#[cfg(feature = "wire")]
pub mod client;
#[cfg(feature = "wire")]
pub mod datadir;
