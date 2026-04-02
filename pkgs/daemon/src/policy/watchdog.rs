//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared watchdog and maintenance escalation tuning.

/// Poll supervised worker health at this cadence.
pub(crate) const WATCHDOG_TICK_SECS: u64 = 2;

/// Treat a worker as stale if it never leaves startup within this window.
pub(crate) const WATCHDOG_STARTING_STALE_SECS: u64 = 60;

/// Treat a worker as stale if it stops reporting activity for this window.
pub(crate) const WATCHDOG_ACTIVITY_STALE_SECS: u64 = 60;

/// Treat a worker as stalled if it stops reporting progress for this window.
pub(crate) const WATCHDOG_PROGRESS_STALE_SECS: u64 = 60;

/// Warn when one supervised operation exceeds this runtime.
pub(crate) const WATCHDOG_SLOW_OPERATION_MS: u64 = 5_000;

/// Fail one supervised operation when it exceeds this runtime budget.
pub(crate) const WATCHDOG_OPERATION_TIMEOUT_SECS: u64 = 60;

/// Escalate recovery after this many repeated failures at one height.
pub(crate) const WATCHDOG_RECOVERY_LIMIT: u32 = 3;

/// Wait this long for graceful quiescence before escalation.
pub(crate) const WATCHDOG_GRACEFUL_IDLE_TIMEOUT_SECS: u64 = 5;

/// Fence long-running work after this idle timeout.
pub(crate) const WATCHDOG_ESCALATED_IDLE_TIMEOUT_SECS: u64 = 30;
