//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared transport and API-adjacent runtime tuning.

/// Reuse TCP connections long enough to keep long-lived clients warm.
pub(crate) const API_TCP_KEEPALIVE_SECS: u64 = 60;

/// Bound one outbound HTTP request before surfacing a timeout.
pub(crate) const API_REQUEST_TIMEOUT_SECS: u64 = 10;

/// Bound one RPC snapshot fetch before treating the data source as stale.
pub(crate) const API_DATA_SOURCE_TIMEOUT_SECS: u64 = 5;

/// Suppress repeated live refresh warnings until this interval elapses.
pub(crate) const API_REFRESH_WARN_INTERVAL_SECS: u64 = 30;
