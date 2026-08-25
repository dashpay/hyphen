//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Test scaffolding, gated behind the `tests` feature so the daemon can reuse
//! it. This crate ships only what makes mocking *possible* — fixtures, a
//! trivial deriver, and the connection harness. The memory-backed fake domain
//! impls live in the daemon, next to their consumers, and are assembled with
//! [`crate::HyphenDb::from_parts`].

pub mod fixtures;
pub mod harness;
pub mod mock;
