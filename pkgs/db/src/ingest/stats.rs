//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The chain-statistics stage: the cumulative `block_metrics` row, the
//! singleton `transactions_summary`/`supply_summary` totals, and the
//! `undo_blocks` pre-image that lets a disconnect restore those summaries
//! verbatim. This stage also owns the `undo_blocks` ledger, since the summary
//! pre-image is the only payload it carries.
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

    // --- block_metrics: fold this block onto the prior block's cumulative
    // totals. The prior read is not a statement of its own: the accumulation
    // is `INSERT ... SELECT` over a `LEFT JOIN` onto height - 1, so one round
    // trip does the read and the write. The join stays outer, and the `WHERE`
    // keeps the missing-prior check exactly as strict as it was: a row is
    // produced only when the prior row exists or this is height 0, so
    // "nothing inserted" *is* "the cumulative chain is broken", surfaced
    // below rather than silently restarting the totals from zero.
    let tx_delta = block.txs.len() as i64;
    let utxo_spendable_delta = metrics.spendable_created - metrics.inputs_spent;
    // True supply delta is the subsidy: coinbase output value minus the fees
    // it recycles (counting full coinbase value would double-count every fee).
    let supply_delta = metrics.coinbase_value - metrics.fee_total;
    let credit_pool = block
      .header
      .coinbase
      .as_ref()
      .and_then(|cb| cb.cb_credit_pool);
    let difficulty = block.header.difficulty;

    let inserted = sqlx::query!(
      r#"INSERT INTO block_metrics
           (height, tx_count, utxos_spendable, utxos_unspendable, blockchain_size,
            credit_pool_balance, circulating_supply, difficulty)
         SELECT $1,
                COALESCE(p.tx_count, 0) + $2,
                COALESCE(p.utxos_spendable, 0) + $3,
                COALESCE(p.utxos_unspendable, 0) + $4,
                COALESCE(p.blockchain_size, 0) + $5,
                $6,
                COALESCE(p.circulating_supply, 0) + $7,
                $8
           FROM (VALUES (TRUE)) AS one(present)
           LEFT JOIN block_metrics p ON p.height = $1 - 1
          WHERE p.height IS NOT NULL OR $1 = 0"#,
      height,
      tx_delta,
      utxo_spendable_delta,
      metrics.unspendable_created,
      metrics.raw_len,
      credit_pool,
      supply_delta,
      difficulty,
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "connect: insert block_metrics",
    ))?
    .rows_affected();
    // A missing prior row is only legitimate at height 0. Anywhere else it
    // means the cumulative chain is broken.
    if inserted != 1 {
      return Err(HyDbStageError::Inconsistent {
        stage: Self::NAME,
        what: format!(
          "no block_metrics row at height {} to accumulate onto",
          height - 1
        ),
      });
    }

    // --- singleton summaries + undo pre-image: one statement -------------
    // `transactions_summary.total` moves by the block's transaction count;
    // `supply_summary.total` by the subsidy (true issued supply),
    // `included_supply` by the addressed-output delta, `unspendable_value` by
    // value parked in / released from addressless outputs. The three supply
    // columns stay mutually consistent: total = included_supply +
    // unspendable_value.
    //
    // Reversibility forbids deferring any of this — `undo_blocks` must hold
    // the pre-image of both singletons at every height — so what is reduced is
    // the number of statements, not the number of facts. Four statements (read
    // the summaries, bump each of the two, write the undo row) become one:
    // each `UPDATE` returns its own pre-image by subtracting the delta it just
    // added, and the `INSERT` reads those. That also removes three sequential
    // scans of `supply_summary` per block, because the old read asked for its
    // three columns as three scalar subqueries — three scans of the singleton,
    // not one.
    let included_delta: i64 = deltas.values().map(|d| d.received - d.sent).sum();
    let block_hash = &block.header.hash.0[..];
    let prev_hash = &block.header.prev_hash.0[..];
    let moved =
      tx_delta != 0 || supply_delta != 0 || included_delta != 0 || metrics.unspendable_delta != 0;

    // A block that moves nothing at all writes no `UPDATE`: the undo row still
    // has to record the pre-image, so it reads the two singletons in place
    // instead of rewriting them with the values they already hold. Unreachable
    // on any chain whose blocks carry a coinbase (a coinbase alone moves
    // `tx_delta` and the subsidy), which is why the moving arm is the one that
    // is optimised; the point of the guard is that a no-op block leaves no
    // dead tuple behind to vacuum.
    let written = if moved {
      sqlx::query!(
        r#"WITH t AS (
             UPDATE transactions_summary SET total = total + $4 WHERE singleton
             RETURNING total - $4 AS pre_total
           ), s AS (
             UPDATE supply_summary
                SET total = total + $5,
                    included_supply = included_supply + $6,
                    unspendable_value = unspendable_value + $7
              WHERE singleton
             RETURNING total - $5 AS pre_total,
                       included_supply - $6 AS pre_included,
                       unspendable_value - $7 AS pre_unspendable
           )
           INSERT INTO undo_blocks
             (height, block_hash, prev_hash, summary_tx_total, summary_supply_total,
              summary_supply_included, summary_supply_unspendable)
           SELECT $1, $2, $3, t.pre_total, s.pre_total, s.pre_included, s.pre_unspendable
             FROM t, s"#,
        height,
        block_hash,
        prev_hash,
        tx_delta,
        supply_delta,
        included_delta,
        metrics.unspendable_delta,
      )
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(
        Self::NAME,
        "connect: bump summaries and write undo",
      ))?
      .rows_affected()
    } else {
      sqlx::query!(
        r#"INSERT INTO undo_blocks
             (height, block_hash, prev_hash, summary_tx_total, summary_supply_total,
              summary_supply_included, summary_supply_unspendable)
           SELECT $1, $2, $3, t.total, s.total, s.included_supply, s.unspendable_value
             FROM transactions_summary t, supply_summary s
            WHERE t.singleton AND s.singleton"#,
        height,
        block_hash,
        prev_hash,
      )
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(
        Self::NAME,
        "connect: write undo without a summary move",
      ))?
      .rows_affected()
    };
    // Both arms read the singletons through the same statement that writes the
    // undo row, so "no undo row" is the only way a missing singleton can show
    // up. It has to be an error and not a silent skip: a height with no
    // pre-image is a height that cannot be disconnected.
    if written != 1 {
      return Err(HyDbStageError::Inconsistent {
        stage: Self::NAME,
        what: format!(
          "summary pre-image for height {height} captured {written} undo rows, expected 1 —            a singleton summary row is missing"
        ),
      });
    }
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
