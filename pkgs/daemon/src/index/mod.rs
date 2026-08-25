//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Read API domains and their capability stages.
//!
//! Derivation and reorg handling now live entirely in the storage engine
//! (`hyphen-db`); this module only hosts the REST read handlers, their live
//! overlays, and the block decode helpers reused by ingest.

pub mod block;
mod block_ctx;
pub mod metrics;
pub mod spent;
mod stage;
pub mod tx;
pub mod validator;
pub mod visor;

pub(crate) use stage::IndexStageFlags;
pub use stage::Stage;
