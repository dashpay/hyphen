//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Canonical reverse journal helpers.

use std::collections::BTreeSet;

use crate::error::DaemonError;

#[derive(Debug, Clone, PartialEq, Eq, sqlx::FromRow)]
pub struct BlockUndoMetaRow {
  pub height: i32,
  pub block_hash: Vec<u8>,
  pub prev_hash: Vec<u8>,
  pub transactions_total_after_disconnect: i64,
  pub address_balances_total_after_disconnect: i64,
  pub address_supply_after_disconnect: i64,
  pub created_at: i64,
}

fn unique_addresses(addresses: &[String]) -> Vec<String> {
  addresses
    .iter()
    .cloned()
    .collect::<BTreeSet<_>>()
    .into_iter()
    .collect()
}

#[derive(sqlx::FromRow)]
struct AddressStatsUndoSourceRow {
  address: String,
  balance: i64,
  total_received: i64,
  tx_count: i32,
  utxo_count: i32,
  first_seen_height: Option<i32>,
  last_seen_height: Option<i32>,
}

pub async fn upsert_block_undo_meta(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
  block_hash: &[u8],
  prev_hash: &[u8],
  created_at: i64,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO block_undo_meta (
       height,
       block_hash,
       prev_hash,
       transactions_total_after_disconnect,
       address_balances_total_after_disconnect,
       address_supply_after_disconnect,
       created_at
     )
     SELECT
       $1,
       $2,
       $3,
       COALESCE(
         (SELECT total FROM transactions_summary WHERE singleton = TRUE),
         0
       ),
       COALESCE(
         (SELECT total FROM address_balances_summary WHERE singleton = TRUE),
         0
       ),
       COALESCE(
         (SELECT included_supply FROM address_balances_summary WHERE singleton = TRUE),
         0
       ),
       $4
     ON CONFLICT (height) DO UPDATE SET
       block_hash = EXCLUDED.block_hash,
       prev_hash = EXCLUDED.prev_hash,
       transactions_total_after_disconnect = EXCLUDED.transactions_total_after_disconnect,
       address_balances_total_after_disconnect = EXCLUDED.address_balances_total_after_disconnect,
       address_supply_after_disconnect = EXCLUDED.address_supply_after_disconnect,
       created_at = EXCLUDED.created_at",
  )
  .bind(height as i32)
  .bind(block_hash)
  .bind(prev_hash)
  .bind(created_at)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("upsert block_undo_meta at {height}: {e}")))?;
  Ok(())
}

pub async fn block_undo_meta(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<BlockUndoMetaRow, DaemonError> {
  sqlx::query_as::<_, BlockUndoMetaRow>(
    "SELECT height,
            block_hash,
            prev_hash,
            transactions_total_after_disconnect,
            address_balances_total_after_disconnect,
            address_supply_after_disconnect,
            created_at
       FROM block_undo_meta
      WHERE height = $1",
  )
  .bind(height)
  .fetch_one(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("read block_undo_meta at {height}: {e}")))
}

pub async fn capture_address_balances_undo(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
  addresses: &[String],
) -> Result<(), DaemonError> {
  let unique = unique_addresses(addresses);
  if unique.is_empty() {
    return Ok(());
  }
  sqlx::query(
    "INSERT INTO block_undo_address_balances (
       height,
       address,
       existed_before,
       prev_balance
     )
     SELECT
       $1,
       touched.address,
       b.address IS NOT NULL,
       b.balance
       FROM unnest($2::text[]) AS touched(address)
       LEFT JOIN address_balances AS b ON b.address = touched.address
     ON CONFLICT (height, address) DO NOTHING",
  )
  .bind(height as i32)
  .bind(&unique)
  .execute(&mut **tx)
  .await
  .map_err(|e| {
    DaemonError::Pg(format!(
      "capture block_undo_address_balances at {height}: {e}"
    ))
  })?;
  Ok(())
}

pub async fn capture_address_stats_undo(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
  addresses: &[String],
) -> Result<(), DaemonError> {
  let unique = unique_addresses(addresses);
  if unique.is_empty() {
    return Ok(());
  }
  let existing = sqlx::query_as::<_, AddressStatsUndoSourceRow>(
    "SELECT address,
            balance,
            total_received,
            tx_count,
            utxo_count,
            first_seen_height,
            last_seen_height
       FROM address_stats
      WHERE address = ANY($1::text[])",
  )
  .bind(&unique)
  .fetch_all(&mut **tx)
  .await
  .map_err(|e| {
    DaemonError::Pg(format!(
      "read current address_stats before undo capture at {height}: {e}"
    ))
  })?;
  let existing_by_address = existing
    .into_iter()
    .map(|row| (row.address.clone(), row))
    .collect::<std::collections::HashMap<_, _>>();

  const ADDRESS_STATS_UNDO_WRITE_CHUNK_SIZE: usize = 2_000;
  for chunk in unique.chunks(ADDRESS_STATS_UNDO_WRITE_CHUNK_SIZE) {
    let mut undo_addresses = Vec::with_capacity(chunk.len());
    let mut existed_before = Vec::with_capacity(chunk.len());
    let mut prev_balances = Vec::with_capacity(chunk.len());
    let mut prev_total_received = Vec::with_capacity(chunk.len());
    let mut prev_tx_counts = Vec::with_capacity(chunk.len());
    let mut prev_utxo_counts = Vec::with_capacity(chunk.len());
    let mut prev_first_seen_heights = Vec::with_capacity(chunk.len());
    let mut prev_last_seen_heights = Vec::with_capacity(chunk.len());

    for address in chunk {
      undo_addresses.push(address.clone());
      if let Some(row) = existing_by_address.get(address) {
        existed_before.push(true);
        prev_balances.push(Some(row.balance));
        prev_total_received.push(Some(row.total_received));
        prev_tx_counts.push(Some(row.tx_count));
        prev_utxo_counts.push(Some(row.utxo_count));
        prev_first_seen_heights.push(row.first_seen_height);
        prev_last_seen_heights.push(row.last_seen_height);
      } else {
        existed_before.push(false);
        prev_balances.push(None);
        prev_total_received.push(None);
        prev_tx_counts.push(None);
        prev_utxo_counts.push(None);
        prev_first_seen_heights.push(None);
        prev_last_seen_heights.push(None);
      }
    }

    sqlx::query(
      "INSERT INTO block_undo_address_stats (
         height,
         address,
         existed_before,
         prev_balance,
         prev_total_received,
         prev_tx_count,
         prev_utxo_count,
         prev_first_seen_height,
         prev_last_seen_height
       )
       SELECT * FROM unnest(
         $1::int[],
         $2::text[],
         $3::boolean[],
         $4::bigint[],
         $5::bigint[],
         $6::int[],
         $7::int[],
         $8::int[],
         $9::int[]
       )
       ON CONFLICT (height, address) DO NOTHING",
    )
    .bind(vec![height as i32; chunk.len()])
    .bind(&undo_addresses)
    .bind(&existed_before)
    .bind(&prev_balances)
    .bind(&prev_total_received)
    .bind(&prev_tx_counts)
    .bind(&prev_utxo_counts)
    .bind(&prev_first_seen_heights)
    .bind(&prev_last_seen_heights)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("capture block_undo_address_stats at {height}: {e}")))?;
  }
  Ok(())
}

#[expect(clippy::too_many_arguments)]
pub async fn insert_block_undo_outputs(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
  prev_txids: &[Vec<u8>],
  prev_vouts: &[i32],
  prev_spent_txids: &[Option<Vec<u8>>],
  prev_spent_vins: &[Option<i32>],
  prev_spent_heights: &[Option<i32>],
) -> Result<(), DaemonError> {
  if prev_txids.is_empty() {
    return Ok(());
  }
  sqlx::query(
    "INSERT INTO block_undo_outputs (
       height,
       prev_txid,
       prev_vout,
       prev_spent_txid,
       prev_spent_vin,
       prev_spent_height
     )
     SELECT * FROM unnest(
       $1::int[],
       $2::bytea[],
       $3::int[],
       $4::bytea[],
       $5::int[],
       $6::int[]
     )
     ON CONFLICT (height, prev_txid, prev_vout) DO NOTHING",
  )
  .bind(vec![height as i32; prev_txids.len()])
  .bind(prev_txids)
  .bind(prev_vouts)
  .bind(prev_spent_txids)
  .bind(prev_spent_vins)
  .bind(prev_spent_heights)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("insert block_undo_outputs at {height}: {e}")))?;
  Ok(())
}

pub async fn restore_tx_outputs_from_undo(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  sqlx::query(
    "UPDATE tx_outputs AS o
        SET spent_txid = u.prev_spent_txid,
            spent_vin = u.prev_spent_vin,
            spent_height = u.prev_spent_height
       FROM block_undo_outputs AS u
      WHERE u.height = $1
        AND o.txid = u.prev_txid
        AND o.vout_index = u.prev_vout",
  )
  .bind(height)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("restore tx_outputs from undo at {height}: {e}")))?;
  Ok(())
}

pub async fn restore_address_balances_from_undo(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  sqlx::query(
    "DELETE FROM address_balances
      WHERE address IN (
        SELECT address
          FROM block_undo_address_balances
         WHERE height = $1
      )",
  )
  .bind(height)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("clear touched address_balances at {height}: {e}")))?;
  sqlx::query(
    "INSERT INTO address_balances (address, balance)
     SELECT address, prev_balance
       FROM block_undo_address_balances
      WHERE height = $1
        AND existed_before
      ORDER BY address",
  )
  .bind(height)
  .execute(&mut **tx)
  .await
  .map_err(|e| {
    DaemonError::Pg(format!(
      "restore address_balances from undo at {height}: {e}"
    ))
  })?;
  Ok(())
}

pub async fn restore_address_stats_from_undo(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  sqlx::query(
    "DELETE FROM address_stats
      WHERE address IN (
        SELECT address
          FROM block_undo_address_stats
         WHERE height = $1
      )",
  )
  .bind(height)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("clear touched address_stats at {height}: {e}")))?;
  sqlx::query(
    "INSERT INTO address_stats (
       address,
       balance,
       total_received,
       tx_count,
       utxo_count,
       first_seen_height,
       last_seen_height
     )
     SELECT address,
            prev_balance,
            prev_total_received,
            prev_tx_count,
            prev_utxo_count,
            prev_first_seen_height,
            prev_last_seen_height
       FROM block_undo_address_stats
      WHERE height = $1
        AND existed_before
      ORDER BY address",
  )
  .bind(height)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("restore address_stats from undo at {height}: {e}")))?;
  Ok(())
}

pub async fn restore_transactions_summary_from_undo(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO transactions_summary (singleton, total)
     SELECT TRUE, transactions_total_after_disconnect
       FROM block_undo_meta
      WHERE height = $1
     ON CONFLICT (singleton) DO UPDATE SET
       total = EXCLUDED.total",
  )
  .bind(height)
  .execute(&mut **tx)
  .await
  .map_err(|e| {
    DaemonError::Pg(format!(
      "restore transactions_summary from undo at {height}: {e}"
    ))
  })?;
  Ok(())
}

pub async fn restore_address_balances_summary_from_undo(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO address_balances_summary (singleton, total, included_supply)
     SELECT TRUE,
            address_balances_total_after_disconnect,
            address_supply_after_disconnect
       FROM block_undo_meta
      WHERE height = $1
     ON CONFLICT (singleton) DO UPDATE SET
       total = EXCLUDED.total,
       included_supply = EXCLUDED.included_supply",
  )
  .bind(height)
  .execute(&mut **tx)
  .await
  .map_err(|e| {
    DaemonError::Pg(format!(
      "restore address_balances_summary from undo at {height}: {e}"
    ))
  })?;
  Ok(())
}
