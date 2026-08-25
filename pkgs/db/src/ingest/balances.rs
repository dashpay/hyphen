//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The address-balance stage: folds the block's per-address deltas into
//! `address_stats`, and records in `undo_address_stats` the only two things a
//! disconnect cannot work out for itself, so a disconnect restores the exact
//! prior state. Batched: three statements per block (snapshot read, undo
//! insert, upsert) regardless of how many addresses the block touches.
//!
//! # Reversal by inverse delta
//!
//! `balance`, `total_received`, `tx_count` and `utxo_count` are additive: the
//! upsert below adds a per-address delta to each, with no clamp, no
//! saturation and no condition on the addend. Subtracting the same delta
//! therefore recovers the prior value exactly, for every value that connect
//! could have produced — Postgres raises on integer overflow rather than
//! wrapping, so a delta that could not be un-added could not have been added
//! either. Those four columns keep no pre-image.
//!
//! Nor does the delta itself: it is a pure function of the base facts the
//! block wrote. `tx_outputs` at this height carries everything the block
//! created (value and count per address) and `spends` at this height, joined
//! back to `tx_outputs` by outpoint, carries everything it consumed; the
//! deduplicated union of the two is exactly the `address_txs` fan-out, hence
//! exactly the `tx_count` delta. Those rows are still in place when this stage
//! reverses: the base stage runs first on connect and therefore last on
//! disconnect. That ordering is load-bearing here and nowhere else in the
//! pipeline — see [`IngestStage`].
//!
//! `first_seen_height` is invertible for a different reason: the upsert sets
//! it on INSERT only, never in the ON CONFLICT branch, so a row that existed
//! before the block still holds its original value and a row that did not is
//! deleted whole. Restoring it would be a no-op.
//!
//! What is left is what `undo_address_stats` stores: `existed`, which decides
//! delete-versus-restore and cannot be derived from a block that only knows
//! it touched the address, and `last_seen_height`, which the upsert
//! overwrites unconditionally with the block height and which no arithmetic
//! can recover.
//!
//! The same stage maintains `address_rich`, the rich-list side table. It
//! exists so `address_stats` carries no balance-ordered index and its updates
//! can be HOT; the ranked read is served from the side table instead. Both
//! directions decide membership from the balance they already hold — connect
//! from `prior + delta`, disconnect from `current - delta` — so neither path
//! ever sums history. Keeping the two directions symmetric is the whole
//! correctness argument: a connect-only implementation would drift from
//! `address_stats` on the first reorg with nothing to detect it.
//!
//! The same two balances also drive `address_summary.positive_balance_count`,
//! the count of addresses with a positive balance. That count has no index to
//! ride since the balance index was removed, so it is accumulated here instead
//! of scanned at read time: connect adds one per address that crosses zero
//! upward and subtracts one per address that crosses it downward, and
//! disconnect applies the inverse from the same prior/current pair.
//!
//! Both projections used to read the prior balance out of the undo row. They
//! now read the *current* balance out of `address_stats` and subtract the
//! recomputed delta, which is the same number by construction — and the
//! disconnect path checks that construction on every block it reverses: the
//! recomputed address set must equal the undo row set, every touched address
//! must still have an `address_stats` row, and a row the block created must
//! hold exactly the balance the block gave it. A derivation that has drifted
//! from the connect path fails loudly instead of silently restoring the wrong
//! state.

use std::collections::HashMap;

use async_trait::async_trait;

use super::pipeline::IngestStage;
use super::scratch::{HyDbStageError, Scratch};
use crate::layout::rich_list_floor;
use crate::records::PgTx;

/// The `address_stats` accumulation stage.
pub(super) struct BalancesStage;

/// The two facts connect needs from an address's pre-block row: the balance
/// the rich list and the positive-balance counter branch on, and the
/// `last_seen_height` the upsert is about to overwrite.
struct PriorStats {
  balance: i64,
  last_seen_height: Option<i32>,
}

/// A block's recomputed effect on one address, as the additive deltas the
/// connect-side upsert applied. Derived from the block's own base facts on
/// the disconnect path; never stored.
struct AddrEffect {
  balance_delta: i64,
  received: i64,
  tx_delta: i32,
  utxo_delta: i32,
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

    // One snapshot read for every touched address. Only two columns: the rest
    // of the row is either recomputable or unchanged, so neither the undo
    // capture nor the projections below have any use for it.
    let prior_rows = sqlx::query!(
      r#"SELECT address, balance, last_seen_height
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
          last_seen_height: r.last_seen_height,
        },
      );
    }

    // Column arrays for the undo capture and the upsert.
    let n = addr_vec.len();
    let mut u_existed: Vec<bool> = Vec::with_capacity(n);
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
      if new_balance >= rich_list_floor() {
        r_addr.push(addr.clone());
        r_balance.push(new_balance);
      } else if prior_balance >= rich_list_floor() {
        r_drop.push(addr.clone());
      }
      positive_delta += i64::from(new_balance > 0) - i64::from(prior_balance > 0);
    }

    // The undo capture: two columns wide, because the other five are either
    // recomputable by inverse delta or untouched by the upsert. See the module
    // header for the column-by-column argument.
    sqlx::query!(
      r#"INSERT INTO undo_address_stats (height, address, existed, last_seen_height)
         SELECT $1, t.address, t.existed, t.last_seen_height
           FROM UNNEST($2::text[], $3::bool[], $4::int4[])
             AS t(address, existed, last_seen_height)"#,
      height,
      &addr_vec,
      &u_existed,
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
    let inconsistent = |what: String| HyDbStageError::Inconsistent {
      stage: Self::NAME,
      what,
    };

    // The two facts the block could not have worked out for itself: which
    // `address_stats` rows it created, and what `last_seen_height` held before
    // it overwrote them. This row set is also the authoritative list of the
    // addresses the block touched.
    let undo_rows = sqlx::query!(
      r#"SELECT address, existed, last_seen_height
           FROM undo_address_stats WHERE height = $1 ORDER BY address"#,
      height
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: read address undo",
    ))?;
    if undo_rows.is_empty() {
      // Either the block touched no address, or its undo rows were pruned
      // below the retention window — and the second case cannot be reversed
      // at all. The pipeline's reorg bound is supposed to make it
      // unreachable; check rather than trust, because the alternative is a
      // disconnect that silently leaves every balance the block moved where
      // the block put it.
      let orphaned: bool = sqlx::query_scalar!(
        r#"SELECT EXISTS (
             SELECT 1 FROM tx_outputs
              WHERE block_height = $1 AND address IS NOT NULL
             UNION ALL
             SELECT 1 FROM spends s JOIN tx_outputs o ON o.outpoint = s.outpoint
              WHERE s.spend_height = $1 AND o.address IS NOT NULL
           ) AS "exists!""#,
        height
      )
      .fetch_one(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(
        Self::NAME,
        "disconnect: check for orphaned address effects",
      ))?;
      if orphaned {
        return Err(inconsistent(format!(
          "block at height {height} moved address balances but has no undo rows;            it is below the undo retention window and cannot be reversed"
        )));
      }
      return Ok(());
    }

    // Everything else, recomputed from the block's own base facts. The base
    // stage disconnects after this one, so `tx_outputs` and `spends` at this
    // height are still in place; the three legs below are the same three the
    // deriver folded into its per-address deltas on the way in — outputs
    // paying an address, prevouts consumed from one, and the deduplicated
    // union of the two that is the `address_txs` fan-out.
    let effect_rows = sqlx::query!(
      r#"WITH created AS (
           SELECT address, SUM(value)::int8 AS recv, COUNT(*)::int4 AS n_out
             FROM tx_outputs
            WHERE block_height = $1 AND address IS NOT NULL
            GROUP BY address
         ), consumed AS (
           SELECT o.address, SUM(o.value)::int8 AS sent, COUNT(*)::int4 AS n_in
             FROM spends s JOIN tx_outputs o ON o.outpoint = s.outpoint
            WHERE s.spend_height = $1 AND o.address IS NOT NULL
            GROUP BY o.address
         ), touched AS (
           SELECT address, txid FROM tx_outputs
            WHERE block_height = $1 AND address IS NOT NULL
           UNION
           SELECT o.address, s.spending_txid
             FROM spends s JOIN tx_outputs o ON o.outpoint = s.outpoint
            WHERE s.spend_height = $1 AND o.address IS NOT NULL
         ), tx_counts AS (
           SELECT address, COUNT(*)::int4 AS n_tx FROM touched GROUP BY address
         )
         SELECT t.address AS "address!",
                (COALESCE(c.recv, 0) - COALESCE(x.sent, 0))::int8 AS "balance_delta!",
                COALESCE(c.recv, 0)::int8                         AS "received!",
                t.n_tx                                            AS "tx_delta!",
                (COALESCE(c.n_out, 0) - COALESCE(x.n_in, 0))::int4 AS "utxo_delta!"
           FROM tx_counts t
           LEFT JOIN created  c ON c.address = t.address
           LEFT JOIN consumed x ON x.address = t.address"#,
      height
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: recompute address deltas",
    ))?;
    let mut effects: HashMap<String, AddrEffect> = HashMap::with_capacity(effect_rows.len());
    for r in effect_rows {
      effects.insert(
        r.address,
        AddrEffect {
          balance_delta: r.balance_delta,
          received: r.received,
          tx_delta: r.tx_delta,
          utxo_delta: r.utxo_delta,
        },
      );
    }
    // The recomputed address set must be the set the connect captured. Both
    // are derived from the same block by different routes, so a divergence is
    // a broken derivation and must not be reversed on top of.
    if effects.len() != undo_rows.len() {
      return Err(inconsistent(format!(
        "recomputed {} touched addresses at height {height} but the undo capture holds {}",
        effects.len(),
        undo_rows.len()
      )));
    }

    // The state the block left behind, for the two projections that need a
    // before/after pair. Every touched address has a row: the connect upsert
    // either inserted or updated one for each.
    let addr_vec: Vec<String> = undo_rows.iter().map(|r| r.address.clone()).collect();
    let current_rows = sqlx::query!(
      "SELECT address, balance FROM address_stats WHERE address = ANY($1)",
      &addr_vec
    )
    .fetch_all(&mut **tx)
    .await
    .map_err(HyDbStageError::sql(
      Self::NAME,
      "disconnect: read address_stats",
    ))?;
    let current: HashMap<String, i64> = current_rows
      .into_iter()
      .map(|r| (r.address, r.balance))
      .collect();

    // ---- Reverse each address, in the mirror of the connect loop ----------
    let n = addr_vec.len();
    let mut d_addr: Vec<String> = Vec::new(); // rows the block created
    let mut e_addr: Vec<String> = Vec::with_capacity(n); // rows it updated
    let mut e_balance: Vec<i64> = Vec::with_capacity(n);
    let mut e_received: Vec<i64> = Vec::with_capacity(n);
    let mut e_tx: Vec<i32> = Vec::with_capacity(n);
    let mut e_utxo: Vec<i32> = Vec::with_capacity(n);
    let mut e_last_seen: Vec<Option<i32>> = Vec::with_capacity(n);
    let mut r_addr: Vec<String> = Vec::new();
    let mut r_balance: Vec<i64> = Vec::new();
    let mut r_drop: Vec<String> = Vec::new();
    let mut positive_delta: i64 = 0;

    for u in &undo_rows {
      let Some(eff) = effects.get(&u.address) else {
        return Err(inconsistent(format!(
          "no recomputed delta for address {} at height {height}",
          u.address
        )));
      };
      let Some(&cur_balance) = current.get(&u.address) else {
        return Err(inconsistent(format!(
          "address {} has no address_stats row to reverse at height {height}",
          u.address
        )));
      };
      let Some(prior_balance) = cur_balance.checked_sub(eff.balance_delta) else {
        return Err(inconsistent(format!(
          "reversing address {} at height {height} overflows: {cur_balance} - {}",
          u.address, eff.balance_delta
        )));
      };

      if u.existed {
        e_addr.push(u.address.clone());
        e_balance.push(eff.balance_delta);
        e_received.push(eff.received);
        e_tx.push(eff.tx_delta);
        e_utxo.push(eff.utxo_delta);
        e_last_seen.push(u.last_seen_height);
      } else {
        // The block created this row, so the whole of its balance is this
        // block's delta. This is the one place the recomputation can be
        // checked against a value the connect path actually wrote, so check
        // it rather than assume it.
        if prior_balance != 0 {
          return Err(inconsistent(format!(
            "address {} was created at height {height} with balance {cur_balance} but the \
             recomputed delta is {}",
            u.address, eff.balance_delta
          )));
        }
        d_addr.push(u.address.clone());
      }

      // Rich list and positive-balance counter, from the same prior/current
      // pair the connect path branched on — with the roles swapped. A row the
      // block created ends up absent, which is neither rich nor positive.
      let restored = if u.existed { prior_balance } else { 0 };
      if u.existed && restored >= rich_list_floor() {
        r_addr.push(u.address.clone());
        r_balance.push(restored);
      } else if cur_balance >= rich_list_floor() {
        r_drop.push(u.address.clone());
      }
      positive_delta += i64::from(restored > 0) - i64::from(cur_balance > 0);
    }

    // Positive-balance counter. Order-independent now: it is computed from the
    // pair above rather than read back out of the tables the statements below
    // are about to rewrite.
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
        "disconnect: unwind address_summary",
      ))?;
    }

    // Delete the address_stats rows this block created (no prior state).
    if !d_addr.is_empty() {
      sqlx::query!("DELETE FROM address_stats WHERE address = ANY($1)", &d_addr)
        .execute(&mut **tx)
        .await
        .map_err(HyDbStageError::sql(
          Self::NAME,
          "disconnect: delete new address_stats",
        ))?;
    }

    // Un-add the block's delta from every row it updated, and put back the one
    // column arithmetic cannot recover. `first_seen_height` is deliberately
    // absent: the upsert never touched it.
    if !e_addr.is_empty() {
      sqlx::query!(
        r#"UPDATE address_stats AS a SET
             balance = a.balance - t.balance_delta,
             total_received = a.total_received - t.received,
             tx_count = a.tx_count - t.tx_delta,
             utxo_count = a.utxo_count - t.utxo_delta,
             last_seen_height = t.last_seen_height
           FROM UNNEST($1::text[], $2::int8[], $3::int8[], $4::int4[], $5::int4[], $6::int4[])
             AS t(address, balance_delta, received, tx_delta, utxo_delta, last_seen_height)
          WHERE a.address = t.address"#,
        &e_addr,
        &e_balance,
        &e_received,
        &e_tx,
        &e_utxo,
        &e_last_seen as _,
      )
      .execute(&mut **tx)
      .await
      .map_err(HyDbStageError::sql(
        Self::NAME,
        "disconnect: reverse address_stats",
      ))?;
    }

    // Rich-list side table, against the balances just restored.
    if !r_drop.is_empty() {
      sqlx::query!("DELETE FROM address_rich WHERE address = ANY($1)", &r_drop)
        .execute(&mut **tx)
        .await
        .map_err(HyDbStageError::sql(
          Self::NAME,
          "disconnect: prune address_rich",
        ))?;
    }
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
        "disconnect: restore address_rich",
      ))?;
    }

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
