//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The data-source seam and the records it produces. Chain semantics live in
//! the daemon; storage semantics live in this crate. The crate never decodes a
//! block — it hands raw bytes to a caller-supplied [`HyDbDataSrc`] and receives
//! structured *records* back.
//!
//! Records are grouped by the thing they describe: [`block`] (the header and the
//! block wrapping it), [`tx`] (a transaction, its inputs/outputs, and its
//! special-tx payload blob), and [`sptx`] (the validator-registry transitions a
//! provider special transaction emits). Each record owns the `INSERT`/`UPDATE`
//! that writes the row(s) it possesses (its `insert`/`apply` method), so the SQL
//! sits beside the fields it mirrors and cannot drift from them. Cross-entity
//! ingest work stays in `ingest::pipeline`.

use sqlx::{Postgres, Transaction};

use crate::error::HyDbResult;

mod block;
mod sptx;
mod tx;

pub use block::{HyDbBlock, HyDbBlockId, HyDbCbFields, HyDbHeader};
pub use sptx::{HyDbVlEvent, HyDbVlRegister, HyDbVlRevoke, HyDbVlUpdRegistrar, HyDbVlUpdService};
pub(crate) use tx::SpentOutput;
pub use tx::{HyDbPayload, HyDbTx, HyDbTxIn, HyDbTxOut};

/// A write-side transaction handle, threaded through one block's connect.
pub(crate) type PgTx<'a> = Transaction<'a, Postgres>;

/// The chain data source: turns raw block bytes into structured records.
///
/// Implementations must be **pure and deterministic** with respect to the raw
/// bytes: no node calls, no I/O, no clock reads. The pipeline calls these both
/// on first ingest and again during reorg re-derivation from `ser_blocks`, and
/// relies on identical bytes producing identical results.
///
/// A block's *height* is not among the records: it is a chain-position property
/// the pipeline assigns as `parent.height + 1` (genesis is 0), never something
/// read from the bytes. So neither method takes or returns a height.
pub trait HyDbDataSrc: Send + Sync {
  /// Cheap identity: the block's own hash and its parent's hash. Used by the
  /// absorb stage to link a block into `ser_blocks` before anything is decoded.
  fn identify(&self, raw: &[u8]) -> HyDbResult<HyDbBlockId>;

  /// Full decode into structured records, for the derive stage.
  fn decode(&self, raw: &[u8]) -> HyDbResult<HyDbBlock>;
}
