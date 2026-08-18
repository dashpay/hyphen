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

use std::collections::HashMap;

use async_trait::async_trait;

use super::pipeline::IngestStage;
use super::scratch::{HyDbStageError, Scratch};
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
      s_balance_delta.push(d.received - d.sent);
      s_received.push(d.received);
      s_tx_delta.push(d.txs.len() as i32);
      s_utxo_delta.push((d.created_utxos - d.spent_utxos) as i32);
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
    Ok(())
  }

  async fn disconnect(&self, tx: &mut PgTx<'_>, height: i32) -> Result<(), HyDbStageError> {
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
