//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Dash block and transaction primitives.

#![cfg_attr(not(feature = "std"), no_std)]

#[cfg(feature = "alloc")]
extern crate alloc;

/// Re-exports of `Vec` and `String`.
#[cfg(feature = "alloc")]
pub(crate) mod prelude {
  #[cfg(not(feature = "std"))]
  #[allow(unused_imports)]
  pub use alloc::{string::String, vec, vec::Vec};

  #[cfg(feature = "std")]
  #[allow(unused_imports)]
  pub use std::{string::String, vec, vec::Vec};
}

#[cfg(feature = "alloc")]
pub mod block;
pub mod cursor;
pub mod decode;
pub mod encode;
pub mod error;
#[cfg(feature = "alloc")]
pub mod gov;
pub mod hash;
#[cfg(feature = "alloc")]
pub mod hex;
#[cfg(feature = "alloc")]
pub mod script;
#[cfg(feature = "alloc")]
pub mod tx;
pub mod types;
#[cfg(feature = "alloc")]
pub mod util;
#[cfg(feature = "wasm")]
pub mod wasm_api;
