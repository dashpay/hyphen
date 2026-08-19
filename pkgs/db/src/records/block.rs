//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block records: the decoded block, its identity/parent link, and the header
//! (with its coinbase special-tx fields) that owns the `blocks` row write.

use super::tx::HyDbTx;
use super::PgTx;
use crate::error::{HyDbError, HyDbResult};
use crate::types::HyDbHash;

/// Everything the storage layer needs to know about one block.
#[derive(Debug, Clone)]
pub struct HyDbBlock {
  pub header: HyDbHeader,
  pub txs: Vec<HyDbTx>,
}

/// A block's identity and parent link — everything absorb needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HyDbBlockId {
  pub hash: HyDbHash,
  pub prev_hash: HyDbHash,
}

/// Coinbase special-transaction fields carried on the header row.
#[derive(Debug, Clone)]
pub struct HyDbCbFields {
  pub cb_height: Option<i32>,
  pub cb_merkle_root: Option<Vec<u8>>,
  pub cb_merkle_flags: Option<i32>,
  pub cb_best_cl_height: Option<i32>,
  pub cb_best_cl_sig: Option<Vec<u8>>,
  pub cb_credit_pool: Option<i64>,
}

/// Decoded header facts plus the per-block inputs the accumulate stage needs to
/// roll cumulative metrics (supply, difficulty, hashrate) forward. Height is
/// deliberately absent — see [`HyDbDataSrc`](super::HyDbDataSrc).
#[derive(Debug, Clone)]
pub struct HyDbHeader {
  pub hash: HyDbHash,
  pub prev_hash: HyDbHash,
  pub merkle_root: HyDbHash,
  pub time: u32,
  pub bits: u32,
  /// Floating-point difficulty (`genesis_target / target`), computed by the
  /// data source from `bits`. Supplied as a fact rather than derived here so
  /// the storage crate does no consensus math — the daemon owns
  /// `bits`→difficulty.
  pub difficulty: f64,
  pub nonce: u32,
  pub version: i32,
  pub size: u32,
  pub tx_count: u32,
  pub chainwork: Option<String>,
  /// Dash coinbase (cbTx) fields, present from DIP0003 onward.
  pub coinbase: Option<HyDbCbFields>,
}

impl HyDbHeader {
  /// Insert the canonical `blocks` row for this header at `height`.
  ///
  /// `chainwork` is the pipeline-accumulated cumulative work (the data source
  /// cannot derive it from a single block in isolation, so the field on the
  /// record itself is only a fallback). `time` is stored as `BIGINT` and
  /// `bits` through a checked conversion — consensus `u32` fields never pass
  /// through a silently truncating `as i32`.
  pub(crate) async fn insert(
    &self,
    tx: &mut PgTx<'_>,
    height: i32,
    chainwork: Option<&str>,
  ) -> HyDbResult<()> {
    let cb = self.coinbase.as_ref();
    let bits: i32 = self
      .bits
      .try_into()
      .map_err(|_| HyDbError::Derive(format!("header bits {:#x} exceeds i32", self.bits)))?;
    sqlx::query!(
      r#"INSERT INTO blocks
           (height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count,
            chainwork, cb_height, cb_merkle_root, cb_merkle_flags, cb_best_cl_height,
            cb_best_cl_sig, cb_credit_pool)
         VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)"#,
      height,
      &self.hash.0[..],
      &self.prev_hash.0[..],
      &self.merkle_root.0[..],
      i64::from(self.time),
      bits,
      i64::from(self.nonce),
      self.version,
      self.size as i32,
      self.tx_count as i32,
      chainwork.or(self.chainwork.as_deref()),
      cb.and_then(|c| c.cb_height),
      cb.and_then(|c| c.cb_merkle_root.clone()),
      cb.and_then(|c| c.cb_merkle_flags),
      cb.and_then(|c| c.cb_best_cl_height),
      cb.and_then(|c| c.cb_best_cl_sig.clone()),
      cb.and_then(|c| c.cb_credit_pool),
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| HyDbError::pg("insert block header", e))?;
    Ok(())
  }
}
