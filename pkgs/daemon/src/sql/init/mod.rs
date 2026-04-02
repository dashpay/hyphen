//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Schema bootstrap, validation, and reset helpers.

mod apply;
mod reset;
mod validate;

const SCHEMA: &str = include_str!(concat!(
  env!("CARGO_MANIFEST_DIR"),
  "/../../schema/daemon_pg.sql"
));

pub use apply::bootstrap;
pub use reset::{reset, targeted_reset};
pub use validate::validate;
