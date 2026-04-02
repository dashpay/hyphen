//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Index-side supervision seams.
//!
//! This module is the ownership boundary for long-lived SQL/RocksDB-backed
//! index orchestration. Runtime migration happens incrementally; existing stage
//! and runner code remains below this seam until it is folded into a dedicated
//! supervisor.

pub mod overlay;

pub use overlay::OverlayPlan;
