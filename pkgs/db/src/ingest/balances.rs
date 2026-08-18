//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The address-balance stage: folds the block's per-address deltas into
//! `address_stats`, capturing each row's pre-image (or its absence) in
//! `undo_address_stats` so a disconnect restores the exact prior state.
//! Batched: three statements per block (snapshot read, undo insert, upsert)
//! regardless of how many addresses the block touches.
//!
//! The same stage maintains `address_rich`, the rich-list side table. It
//! exists so `address_stats` carries no balance-ordered index and its updates
//! can be HOT; the ranked read is served from the side table instead. Both
//! directions decide membership from the balance they already hold — connect
//! from `prior + delta`, disconnect from the undo pre-image — so neither path
//! ever sums history. Keeping the two directions symmetric is the whole
//! correctness argument: a connect-only implementation would drift from
//! `address_stats` on the first reorg with nothing to detect it.
//!
//! The same two balances also drive `address_summary.positive_balance_count`,
//! the count of addresses with a positive balance. That count has no index to
//! ride since the balance index was removed, so it is accumulated here instead
//! of scanned at read time: connect adds one per address that crosses zero
//! upward and subtracts one per address that crosses it downward, and
//! disconnect applies the inverse, computed from the undo pre-image against
//! the still-current row before either is touched.

use std::collections::HashMap;

use async_trait::async_trait;

use super::pipeline::IngestStage;
use super::scratch::{HyDbStageError, Scratch};
use crate::layout::RICH_LIST_FLOOR;
use crate::records::PgTx;

/// The `address_stats` accumulation stage.
pub(super) struct BalancesStage;

struct PriorStats {
  balance: i64,
  total_received: i64,
  tx_count: i32,
  utxo_count: i32,
  first_seen_height: Option<i32>,
  last_seen_height: Option<i32>,
}

#[async_trait]
impl IngestStage for BalancesStage {
  const NAME: &'static str = "balances";

  async fn connect(&self, tx: &mut PgTx<'_>, scratch: &Scratch) -> Result<(), HyDbStageError> {
    let height = scratch.height;
    let deltas = scratch.address_deltas(Self::NAME)?;
    if deltas.is_empty() {
      return Ok(());
    }

    // Deterministic order keeps the statement plans stable and the undo rows
    // reproducible.
    let mut addrs: Vec<&String> = deltas.keys().collect();
    addrs.sort();
    let addr_vec: Vec<String> = addrs.iter().map(|a| (*a).clone()).collect();

    // One snapshot read for every touched address.
    let prior_rows = sqlx::query!(
      r#"SELECT address, balance, total_received, tx_count, utxo_count,
                first_seen_height, last_seen_height
           FROM address_stats WHERE address = ANY($1)"#,
      &addr_vec
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "connect: read address_stats",
    ))?;
    let mut prior: HashMap<String, PriorStats> = HashMap::with_capacity(prior_rows.len());
    for r in prior_rows {
      prior.insert(
        r.address,
        PriorStats {
          balance: r.balance,
          total_received: r.total_received,
          tx_count: r.tx_count,
          utxo_count: r.utxo_count,
          first_seen_height: r.first_seen_height,
          last_seen_height: r.last_seen_height,
        },
      );
    }

    // Column arrays for the undo capture and the upsert.
    let n = addr_vec.len();
    let mut u_existed: Vec<bool> = Vec::with_capacity(n);
    let mut u_balance: Vec<Option<i64>> = Vec::with_capacity(n);
    let mut u_received: Vec<Option<i64>> = Vec::with_capacity(n);
    let mut u_tx_count: Vec<Option<i32>> = Vec::with_capacity(n);
    let mut u_utxo_count: Vec<Option<i32>> = Vec::with_capacity(n);
    let mut u_first_seen: Vec<Option<i32>> = Vec::with_capacity(n);
    let mut u_last_seen: Vec<Option<i32>> = Vec::with_capacity(n);
    let mut s_balance_delta: Vec<i64> = Vec::with_capacity(n);
    let mut s_received: Vec<i64> = Vec::with_capacity(n);
    let mut s_tx_delta: Vec<i32> = Vec::with_capacity(n);
    let mut s_utxo_delta: Vec<i32> = Vec::with_capacity(n);
    // Rich-list membership, decided from the post-upsert balance computed
    // here. `was_member` is derived from the prior balance rather than probed
    // in `address_rich`: the two agree by construction, which is exactly what
    // makes a floor change an offline rebuild rather than a resync.
    let mut r_addr: Vec<String> = Vec::new();
    let mut r_balance: Vec<i64> = Vec::new();
    let mut r_drop: Vec<String> = Vec::new();
    // Net change in the number of positive-balance addresses. Same derivation
    // as `was_member` above and for the same reason: the prior balance read
    // here is the authority, so the counter cannot disagree with the rows it
    // counts.
    let mut positive_delta: i64 = 0;

    for addr in &addr_vec {
      let p = prior.get(addr);
      u_existed.push(p.is_some());
      u_balance.push(p.map(|p| p.balance));
      u_received.push(p.map(|p| p.total_received));
      u_tx_count.push(p.map(|p| p.tx_count));
      u_utxo_count.push(p.map(|p| p.utxo_count));
      u_first_seen.push(p.and_then(|p| p.first_seen_height));
      u_last_seen.push(p.and_then(|p| p.last_seen_height));

      let Some(d) = deltas.get(addr) else {
        return Err(HyDbStageError::Inconsistent {
          stage: Self::NAME,
          what: format!("delta vanished for address {addr}"),
        });
      };
      let delta = d.received - d.sent;
      s_balance_delta.push(delta);
      s_received.push(d.received);
      s_tx_delta.push(d.txs.len() as i32);
      s_utxo_delta.push((d.created_utxos - d.spent_utxos) as i32);

      let prior_balance = p.map_or(0, |p| p.balance);
      let new_balance = prior_balance.saturating_add(delta);
      if new_balance >= RICH_LIST_FLOOR {
        r_addr.push(addr.clone());
        r_balance.push(new_balance);
      } else if prior_balance >= RICH_LIST_FLOOR {
        r_drop.push(addr.clone());
      }
      positive_delta += i64::from(new_balance > 0) - i64::from(prior_balance > 0);
    }

    sqlx::query!(
      r#"INSERT INTO undo_address_stats
           (height, address, existed, balance, total_received, tx_count, utxo_count,
            first_seen_height, last_seen_height)
         SELECT $1, t.address, t.existed, t.balance, t.total_received, t.tx_count,
                t.utxo_count, t.first_seen_height, t.last_seen_height
           FROM UNNEST($2::text[], $3::bool[], $4::int8[], $5::int8[], $6::int4[],
                       $7::int4[], $8::int4[], $9::int4[])
             AS t(address, existed, balance, total_received, tx_count, utxo_count,
                  first_seen_height, last_seen_height)"#,
      height,
      &addr_vec,
      &u_existed,
      &u_balance as _,
      &u_received as _,
      &u_tx_count as _,
      &u_utxo_count as _,
      &u_first_seen as _,
      &u_last_seen as _,
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "connect: capture address undo",
    ))?;

    sqlx::query!(
      r#"INSERT INTO address_stats
           (address, balance, total_received, tx_count, utxo_count,
            first_seen_height, last_seen_height)
         SELECT t.address, t.balance_delta, t.received, t.tx_delta, t.utxo_delta, $1, $1
           FROM UNNEST($2::text[], $3::int8[], $4::int8[], $5::int4[], $6::int4[])
             AS t(address, balance_delta, received, tx_delta, utxo_delta)
         ON CONFLICT (address) DO UPDATE SET
           balance = address_stats.balance + EXCLUDED.balance,
           total_received = address_stats.total_received + EXCLUDED.total_received,
           tx_count = address_stats.tx_count + EXCLUDED.tx_count,
           utxo_count = address_stats.utxo_count + EXCLUDED.utxo_count,
           last_seen_height = EXCLUDED.last_seen_height"#,
      height,
      &addr_vec,
      &s_balance_delta,
      &s_received,
      &s_tx_delta,
      &s_utxo_delta,
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "connect: upsert address_stats",
    ))?;

    // Positive-balance counter. Skipped when the block moved no address across
    // zero, which keeps the singleton row out of most block transactions.
    if positive_delta != 0 {
      sqlx::query!(
        r#"UPDATE address_summary
              SET positive_balance_count = positive_balance_count + $1
            WHERE singleton"#,
        positive_delta,
      )
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(
        Self::NAME,
        "connect: bump address_summary",
      ))?;
    }

    // Rich-list side table. Both statements are skipped when the block moved
    // no address across (or within) the floor, which is the common case.
    if !r_addr.is_empty() {
      sqlx::query!(
        r#"INSERT INTO address_rich (address, balance)
           SELECT t.address, t.balance
             FROM UNNEST($1::text[], $2::int8[]) AS t(address, balance)
           ON CONFLICT (address) DO UPDATE SET balance = EXCLUDED.balance"#,
        &r_addr,
        &r_balance,
      )
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(
        Self::NAME,
        "connect: upsert address_rich",
      ))?;
    }
    if !r_drop.is_empty() {
      sqlx::query!("DELETE FROM address_rich WHERE address = ANY($1)", &r_drop)
        .execute(&mut **tx)
        .await
        .map_err(HyDbStageError::sql(
          Self::NAME,
          "connect: prune address_rich",
        ))?;
    }
    Ok(())
  }

  async fn disconnect(&self, tx: &mut PgTx<'_>, height: i32) -> Result<(), HyDbStageError> {
    // Positive-balance counter, mirrored from the pre-image. This has to run
    // first: it compares each undo row against the address_stats row as the
    // block left it, and the two statements below overwrite exactly those
    // rows. A missing `address_stats` row counts as non-positive on both
    // sides, so the arithmetic is the exact inverse of the connect path's.
    sqlx::query!(
      r#"UPDATE address_summary AS s
            SET positive_balance_count = s.positive_balance_count + d.delta
           FROM (SELECT COALESCE(SUM(
                          (CASE WHEN u.existed AND u.balance > 0 THEN 1 ELSE 0 END)
                        - (CASE WHEN a.balance > 0 THEN 1 ELSE 0 END)), 0) AS delta
                   FROM undo_address_stats AS u
                   LEFT JOIN address_stats AS a ON a.address = u.address
                  WHERE u.height = $1) AS d
          WHERE s.singleton AND d.delta <> 0"#,
      height
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: unwind address_summary",
    ))?;

    // Delete the address_stats rows this block created (no pre-image to restore).
    sqlx::query!(
      r#"DELETE FROM address_stats WHERE address IN
           (SELECT address FROM undo_address_stats WHERE height = $1 AND NOT existed)"#,
      height
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: delete new address_stats",
    ))?;

    // Restore the pre-image of every address_stats row this block updated, in
    // one set-based UPDATE. The `existed` filter guarantees non-null columns.
    sqlx::query!(
      r#"UPDATE address_stats AS a SET
           balance = u.balance, total_received = u.total_received, tx_count = u.tx_count,
           utxo_count = u.utxo_count, first_seen_height = u.first_seen_height,
           last_seen_height = u.last_seen_height
         FROM undo_address_stats AS u
        WHERE u.height = $1 AND u.existed AND a.address = u.address"#,
      height
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: restore address_stats",
    ))?;

    // Mirror the connect-side membership check against the restored balances,
    // while the undo rows are still present. `NOT existed` means the block
    // registered the address, so the post-disconnect state is "no row" and the
    // side-table entry (if any) must go with it.
    sqlx::query!(
      r#"DELETE FROM address_rich AS r
           USING undo_address_stats AS u
          WHERE u.height = $1 AND r.address = u.address
            AND (NOT u.existed OR u.balance < $2)"#,
      height,
      RICH_LIST_FLOOR,
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: prune address_rich",
    ))?;

    sqlx::query!(
      r#"INSERT INTO address_rich (address, balance)
         SELECT u.address, u.balance
           FROM undo_address_stats AS u
          WHERE u.height = $1 AND u.existed AND u.balance >= $2
         ON CONFLICT (address) DO UPDATE SET balance = EXCLUDED.balance"#,
      height,
      RICH_LIST_FLOOR,
    )
    .execute(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: restore address_rich",
    ))?;

    sqlx::query!("DELETE FROM undo_address_stats WHERE height = $1", height)
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(
        Self::NAME,
        "disconnect: delete address undo",
      ))?;
    Ok(())
  }
}
