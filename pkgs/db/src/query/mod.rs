//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Read-side domains. Each domain is a trait (plural = handle) with a Postgres
//! implementation here and an in-memory implementation supplied by the daemon
//! for its own tests. Row types (singular where they name one record) live
//! beside the trait that returns them.
//!
//! Method bodies for the Postgres backend are ported per domain in Phase 2;
//! until then they return [`crate::HyDbError::Unimplemented`].

pub mod addrs;
pub mod blocks;
pub mod metrics;
pub mod search;
pub mod status;
pub mod txs;
pub mod validators;

pub use addrs::{
  HyDbAddrIdx, HyDbQrAddrSummary, HyDbQrAddrTx, HyDbQrRichEntry, HyDbQrSupply, HyDbQrUtxo,
  HyDbTxDir,
};
pub use blocks::{
  HyDbBlockIdx, HyDbQrBlockDetail, HyDbQrBlockRef, HyDbQrBlockSummary, HyDbQrBlockTxRef,
};
pub use metrics::{HyDbMetricIdx, HyDbQrMetricPoint};
pub use search::{HyDbQrSearchHit, HyDbSearch, HyDbSearchKind};
pub use status::{
  HyDbQrChainTip, HyDbQrHealth, HyDbQrPoolStat, HyDbQrSizeDiag, HyDbQrStorageDiag, HyDbStatus,
};
pub use txs::{HyDbQrTxDetail, HyDbQrTxIn, HyDbQrTxOut, HyDbQrTxSummary, HyDbTxIdx};
pub use validators::{HyDbQrValidator, HyDbQrVlCounts, HyDbQrVlHistory, HyDbVlIdx};

use core::fmt;

use sqlx::PgPool;

/// The pools a read backend uses: latency-sensitive `query` for point lookups,
/// `heavy` for analytical scans (rich list, ranges), `admin` for diagnostics.
#[derive(Clone)]
pub(crate) struct PgRead {
  pub query: PgPool,
  pub heavy: PgPool,
  pub admin: PgPool,
}

impl fmt::Debug for PgRead {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("PgRead").finish()
  }
}
