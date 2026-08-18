//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The metrics domain: cumulative per-height chain metrics for charts.

use async_trait::async_trait;

use super::PgRead;
use crate::error::{HyDbError, HyDbResult};
use crate::types::HyDbHeight;

/// A cumulative metrics point at a given height.
#[derive(Debug, Clone, Copy)]
pub struct HyDbQrMetricPoint {
  pub height: HyDbHeight,
  pub tx_count: i64,
  pub utxos_spendable: i64,
  pub utxos_unspendable: i64,
  pub blockchain_size: i64,
  pub credit_pool_balance: Option<i64>,
  pub circulating_supply: i64,
  pub difficulty: f64,
}

/// Query handle for chain metrics.
#[async_trait]
pub trait HyDbMetricIdx: Send + Sync {
  /// Metrics at the current tip.
  async fn latest(&self) -> HyDbResult<Option<HyDbQrMetricPoint>>;

  /// Metrics across a height range, oldest-first. `step` down-samples by
  /// returning roughly one point per `step` blocks (`step <= 1` returns all).
  async fn range(
    &self,
    from: HyDbHeight,
    to: HyDbHeight,
    step: u32,
  ) -> HyDbResult<Vec<HyDbQrMetricPoint>>;
}

#[derive(Debug)]
pub(crate) struct PgMetricIdx {
  pub(crate) pg: PgRead,
}

#[async_trait]
impl HyDbMetricIdx for PgMetricIdx {
  async fn latest(&self) -> HyDbResult<Option<HyDbQrMetricPoint>> {
    let row = sqlx::query!(
      r#"SELECT height, tx_count, utxos_spendable, utxos_unspendable, blockchain_size,
                credit_pool_balance, circulating_supply, difficulty
           FROM block_metrics ORDER BY height DESC LIMIT 1"#
    )
    .fetch_optional(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("latest metrics", e))?;

    Ok(row.map(|r| HyDbQrMetricPoint {
      height: HyDbHeight(r.height.max(0) as u32),
      tx_count: r.tx_count,
      utxos_spendable: r.utxos_spendable,
      utxos_unspendable: r.utxos_unspendable,
      blockchain_size: r.blockchain_size,
      credit_pool_balance: r.credit_pool_balance,
      circulating_supply: r.circulating_supply,
      difficulty: r.difficulty,
    }))
  }

  async fn range(
    &self,
    from: HyDbHeight,
    to: HyDbHeight,
    step: u32,
  ) -> HyDbResult<Vec<HyDbQrMetricPoint>> {
    // A step above i32::MAX would go negative through an unchecked cast, and
    // `height % negative` silently matches nothing — clamp instead. Heights
    // are bounded by i32, so any larger step degenerates to "endpoints only".
    let step = i32::try_from(step.max(1)).unwrap_or(i32::MAX);
    // Bound the result set: without a LIMIT a wide range with step 1 could
    // stream millions of rows through the heavy pool.
    const MAX_POINTS: i64 = 10_000;
    let rows = sqlx::query!(
      r#"SELECT height, tx_count, utxos_spendable, utxos_unspendable, blockchain_size,
                credit_pool_balance, circulating_supply, difficulty
           FROM block_metrics
          WHERE height >= $1 AND height <= $2
            AND ($3 <= 1 OR (height % $3) = 0)
          ORDER BY height ASC
          LIMIT $4"#,
      from.0 as i32,
      to.0 as i32,
      step,
      MAX_POINTS
    )
    .fetch_all(&self.pg.heavy)
    .await
    .map_err(|e| HyDbError::pg("metrics range", e))?;

    Ok(
      rows
        .into_iter()
        .map(|r| HyDbQrMetricPoint {
          height: HyDbHeight(r.height.max(0) as u32),
          tx_count: r.tx_count,
          utxos_spendable: r.utxos_spendable,
          utxos_unspendable: r.utxos_unspendable,
          blockchain_size: r.blockchain_size,
          credit_pool_balance: r.credit_pool_balance,
          circulating_supply: r.circulating_supply,
          difficulty: r.difficulty,
        })
        .collect(),
    )
  }
}
