//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Indexed SQL contract modules.
//!
//! Each file in this tree owns one indexed domain and exposes typed read and
//! write operations for that domain. Callers outside `crate::sql` consume these
//! operations; they do not build indexed SQL directly.

pub mod block;
pub mod metrics;
pub mod script;
pub mod spent;
pub mod tx;
pub mod validator;
