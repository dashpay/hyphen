//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The chain-statistics stage: the cumulative `block_metrics` row, the singleton
//! `transactions_summary`/`supply_summary` totals, and the `undo_blocks`
//! pre-image that lets a disconnect restore those summaries verbatim. This stage
//! also owns the `undo_blocks` ledger, since the summary pre-image is the only
//! payload it carries.
//!
//! `block_metrics` stores `difficulty` only; the daemon derives `hashrate` at
//! read time from the chain's target block time (`difficulty * 2^32 /
//! block_time_secs`), so the value is never persisted here.

use async_trait::async_trait;

use super::pipeline::IngestStage;
use super::scratch::{HyDbStageError, Scratch};
use crate::records::PgTx;

/// The summaries + `block_metrics` stage.
pub(super) struct StatsStage;

#[async_trait]
impl IngestStage for StatsStage {
  const NAME: &'static str = "stats";

  async fn connect(&self, tx: &mut PgTx<'_>, scratch: &Scratch) -> Result<(), HyDbStageError> {
    let height = scratch.height;
    let block = scratch.block(Self::NAME)?;
    let metrics = scratch.metrics(Self::NAME)?;
    let deltas = scratch.address_deltas(Self::NAME)?;

    // --- block_metrics: fold this block onto the prior block's cumulative totals.
    let prior = sqlx::query!(
      r#"SELECT tx_count, utxos_spendable, utxos_unspendable, blockchain_size, circulating_supply
           FROM block_metrics WHERE height = $1"#,
      height - 1
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "connect: read prior metrics",
    ))?;
    // A missing prior row is only legitimate at height 0. Anywhere else it
    // means the cumulative chain is broken — silently restarting the totals
    // from zero would corrupt every metric from here on.
    let (prev_tx, prev_spendable, prev_unspendable, prev_size, prev_supply) = match prior {
      Some(p) => (
        p.tx_count,
        p.utxos_spendable,
        p.utxos_unspendable,
        p.blockchain_size,
        p.circulating_supply,
      ),
      None if height == 0 => (0, 0, 0, 0, 0),
      None => {
        return Err(HyDbStageError::Inconsistent {
          stage: Self::NAME,
          what: format!(
            "no block_metrics row at height {} to accumulate onto",
            height - 1
          ),
        });
      }
    };

    let tx_count = prev_tx + block.txs.len() as i64;
    let utxos_spendable = prev_spendable + metrics.spendable_created - metrics.inputs_spent;
    let utxos_unspendable = prev_unspendable + metrics.unspendable_created;
    let blockchain_size = prev_size + metrics.raw_len;
    // True supply delta is the subsidy: coinbase output value minus the fees
    // it recycles (counting full coinbase value would double-count every fee).
    let supply_delta = metrics.coinbase_value - metrics.fee_total;
    let circulating_supply = prev_supply + supply_delta;
    let credit_pool = block
      .header
      .coinbase
      .as_ref()
      .and_then(|cb| cb.cb_credit_pool);
    let difficulty = block.header.difficulty;

    sqlx::query!(
      r#"INSERT INTO block_metrics
           (height, tx_count, utxos_spendable, utxos_unspendable, blockchain_size,
            credit_pool_balance, circulating_supply, difficulty)
         VALUES ($1, $2, $3, $4, $5, $6, $7, $8)"#,
      height,
      tx_count,
      utxos_spendable,
      utxos_unspendable,
      blockchain_size,
      credit_pool,
      circulating_supply,
      difficulty,
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "connect: insert block_metrics",
    ))?;

    // --- singleton summaries: bump, capturing the prior totals for undo.
    // `total` moves by the subsidy (true issued supply); `included_supply`
    // moves by the addressed-output delta; `unspendable_value` absorbs value
    // parked in / released from addressless outputs. The three stay mutually
    // consistent: total = included_supply + unspendable_value.
    let tx_delta = block.txs.len() as i64;
    let included_delta: i64 = deltas.values().map(|d| d.received - d.sent).sum();

    let prior = sqlx::query!(
      r#"SELECT (SELECT total FROM transactions_summary WHERE singleton) AS "tx_total!",
                (SELECT total FROM supply_summary WHERE singleton) AS "supply_total!",
                (SELECT included_supply FROM supply_summary WHERE singleton) AS "supply_included!",
                (SELECT unspendable_value FROM supply_summary WHERE singleton) AS "supply_unspendable!""#
    )
    .fetch_one(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(Self::NAME, "connect: read summaries"))?;

    sqlx::query!(
      "UPDATE transactions_summary SET total = total + $1 WHERE singleton",
      tx_delta
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(Self::NAME, "connect: bump tx summary"))?;

    sqlx::query!(
      r#"UPDATE supply_summary
            SET total = total + $1,
                included_supply = included_supply + $2,
                unspendable_value = unspendable_value + $3
          WHERE singleton"#,
      supply_delta,
      included_delta,
      metrics.unspendable_delta,
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(Self::NAME, "connect: bump supply"))?;

    // --- undo_blocks: the summary pre-image plus block identity.
    sqlx::query!(
      r#"INSERT INTO undo_blocks
           (height, block_hash, prev_hash, summary_tx_total, summary_supply_total,
            summary_supply_included, summary_supply_unspendable)
         VALUES ($1, $2, $3, $4, $5, $6, $7)"#,
      height,
      &block.header.hash.0[..],
      &block.header.prev_hash.0[..],
      prior.tx_total,
      prior.supply_total,
      prior.supply_included,
      prior.supply_unspendable,
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(Self::NAME, "connect: write undo"))?;
    Ok(())
  }

  async fn disconnect(&self, tx: &mut PgTx<'_>, height: i32) -> Result<(), HyDbStageError> {
    let undo = sqlx::query!(
      r#"SELECT summary_tx_total, summary_supply_total, summary_supply_included,
                summary_supply_unspendable
           FROM undo_blocks WHERE height = $1"#,
      height
    )
    .fetch_optional(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(Self::NAME, "disconnect: read undo"))?
    .ok_or_else(|| HyDbStageError::Inconsistent {
      stage: Self::NAME,
      what: format!("no undo_blocks row captured for height {height}"),
    })?;

    // Restore the singleton summaries verbatim.
    sqlx::query!(
      "UPDATE transactions_summary SET total = $1 WHERE singleton",
      undo.summary_tx_total
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: restore tx summary",
    ))?;
    sqlx::query!(
      r#"UPDATE supply_summary
            SET total = $1, included_supply = $2, unspendable_value = $3
          WHERE singleton"#,
      undo.summary_supply_total,
      undo.summary_supply_included,
      undo.summary_supply_unspendable
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: restore supply",
    ))?;

    sqlx::query!("DELETE FROM block_metrics WHERE height = $1", height)
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(
        Self::NAME,
        "disconnect: delete block_metrics",
      ))?;

    sqlx::query!("DELETE FROM undo_blocks WHERE height = $1", height)
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(
        Self::NAME,
        "disconnect: delete undo_blocks",
      ))?;
    Ok(())
  }
}
