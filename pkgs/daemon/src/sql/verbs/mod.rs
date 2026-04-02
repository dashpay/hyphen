//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! IPC verb SQL contract modules.
//!
//! Verb-owned SQL belongs here when rewind or reindex needs control-plane
//! persistence work that is not part of the indexed or live data planes.

pub mod reindex;
pub mod rewind;
