//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Spent index SQL operations.

use sqlx::PgPool;

use crate::error::DaemonError;
use crate::rest::openapi::AddressTxDirection;
use crate::sql::pg_err;

// ── Connect phases ─────────────────────────────────────────────────────────

/// Max rows per unnest statement to stay within PG parameter limits.
const PREVOUT_LOOKUP_CHUNK_SIZE: usize = 2_000;
const PREVOUT_STAGE_CHUNK_SIZE: usize = 2_000;
const SPENT_HISTORY_WRITE_CHUNK_SIZE: usize = 2_000;
const ADDRESS_HISTORY_WRITE_CHUNK_SIZE: usize = 2_000;

#[derive(Debug, Clone)]
pub struct AddressSummaryRow {
  pub balance: i64,
  pub total_received: i64,
  pub total_sent: i64,
  pub tx_count: i32,
  pub utxo_count: i32,
  pub first_seen_height: Option<i32>,
  pub last_seen_height: Option<i32>,
}

#[derive(Debug, Clone)]
pub struct AddressTxRow {
  pub txid: Vec<u8>,
  pub block_height: i32,
  pub block_pos: i32,
  pub block_time: i64,
  pub version: i32,
  pub tx_type: crate::sql::types::TxTypeDb,
  pub size: i32,
  pub value_out: i64,
  pub net_value: i64,
  pub direction: AddressTxDirection,
  pub fee: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct AddressUtxoRow {
  pub txid: Vec<u8>,
  pub vout_index: i32,
  pub value: i64,
  pub block_height: i32,
  pub script_pubkey: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct BalancesRow {
  pub address: String,
  pub balance: i64,
  pub total_received: i64,
  pub total_sent: i64,
  pub tx_count: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BalancesSummaryRow {
  pub total: i64,
  pub included_supply: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct BalancesSummaryDeltaDiag {
  current_total: i64,
  current_included_supply: i64,
  total_delta: i64,
  included_supply_delta: i64,
  exact_total: i64,
  exact_included_supply: i64,
  staged_rows: i64,
  distinct_addresses: i64,
}

#[derive(Debug, Clone)]
pub struct AddressSearchRow {
  pub address: String,
}

#[derive(Default)]
struct AddressStatsDelta {
  balance: i64,
  total_received: i64,
  utxo_count: i32,
}

#[derive(Default)]
struct AddressBalanceDelta {
  balance: i64,
}

/// Ensure all spent-history partitions covering the given height exist.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn ensure_history_partitions(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  for sql in crate::sql::partition::history_partition_sqls(height) {
    sqlx::query(&sql)
      .execute(&mut **tx)
      .await
      .map_err(|e| pg_err("ensure spent history partitions", e))?;
  }
  Ok(())
}

/// Read the current address summary row.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn address_summary(
  pool: &PgPool,
  address: &str,
) -> Result<Option<AddressSummaryRow>, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query(
    "SELECT balance,
            total_received,
            (total_received - balance)::bigint AS total_sent,
            tx_count,
            utxo_count,
            first_seen_height,
            last_seen_height
       FROM address_stats
      WHERE address = $1",
  )
  .bind(address)
  .fetch_optional(pool)
  .await
  .map_err(|e| pg_err("read address summary", e))?;

  Ok(row.map(|row| AddressSummaryRow {
    balance: row.get("balance"),
    total_received: row.get("total_received"),
    total_sent: row.get("total_sent"),
    tx_count: row.get("tx_count"),
    utxo_count: row.get("utxo_count"),
    first_seen_height: row.get("first_seen_height"),
    last_seen_height: row.get("last_seen_height"),
  }))
}

/// Check whether one address exists anywhere in indexed chain facts.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn address_exists(pool: &PgPool, address: &str) -> Result<bool, DaemonError> {
  let exists = sqlx::query_scalar::<_, bool>(
    "SELECT EXISTS(
       SELECT 1
         FROM address_stats
        WHERE address = $1
       LIMIT 1
     )",
  )
  .bind(address)
  .fetch_one(pool)
  .await
  .map_err(|e| pg_err("read address existence", e))?;
  Ok(exists)
}

/// Read one block timestamp by height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn block_time_by_height(pool: &PgPool, height: i32) -> Result<Option<i64>, DaemonError> {
  sqlx::query_scalar("SELECT time::bigint FROM blocks WHERE height = $1")
    .bind(height)
    .fetch_optional(pool)
    .await
    .map_err(|e| pg_err("read block time by height", e))
}

/// Read paginated transaction summaries for one address.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn address_history_txs(
  pool: &PgPool,
  address: &str,
  limit: i64,
  before: Option<(i32, i32, &[u8])>,
) -> Result<Vec<AddressTxRow>, DaemonError> {
  use sqlx::Row;

  let rows = match before {
    Some((before_height, before_pos, before_txid)) => {
      sqlx::query(
        "WITH page AS (
           SELECT txid, block_height, block_pos
             FROM address_txs
            WHERE address = $1
              AND (block_height, block_pos, txid) < ($2, $3, $4)
            ORDER BY block_height DESC, block_pos DESC, txid DESC
            LIMIT $5
         )
         SELECT t.txid, p.block_height, p.block_pos, b.time::bigint AS block_time,
                t.version, t.tx_type, t.size, t.value_out,
                (COALESCE(received.value, 0) - COALESCE(sent.value, 0))::bigint AS net_value,
                CASE
                  WHEN (COALESCE(received.value, 0) - COALESCE(sent.value, 0)) > 0 THEN 'in'
                  ELSE 'out'
                END AS direction,
                t.fee
           FROM page AS p
           JOIN transactions AS t
             ON t.block_height = p.block_height
            AND t.txid = p.txid
           JOIN blocks AS b ON b.height = p.block_height
           LEFT JOIN LATERAL (
             SELECT COALESCE(SUM(o.value), 0)::bigint AS value
               FROM tx_outputs AS o
              WHERE o.block_height = p.block_height
                AND o.txid = p.txid
                AND o.address = $1
           ) AS received ON TRUE
           LEFT JOIN LATERAL (
             SELECT COALESCE(SUM(i.prev_value), 0)::bigint AS value
               FROM tx_inputs AS i
              WHERE i.txid = p.txid
                AND i.prev_address = $1
                AND i.prev_value IS NOT NULL
           ) AS sent ON TRUE
          ORDER BY p.block_height DESC, p.block_pos DESC, p.txid DESC",
      )
      .bind(address)
      .bind(before_height)
      .bind(before_pos)
      .bind(before_txid)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    None => {
      sqlx::query(
        "WITH page AS (
           SELECT txid, block_height, block_pos
             FROM address_txs
            WHERE address = $1
            ORDER BY block_height DESC, block_pos DESC, txid DESC
            LIMIT $2
         )
         SELECT t.txid, p.block_height, p.block_pos, b.time::bigint AS block_time,
                t.version, t.tx_type, t.size, t.value_out,
                (COALESCE(received.value, 0) - COALESCE(sent.value, 0))::bigint AS net_value,
                CASE
                  WHEN (COALESCE(received.value, 0) - COALESCE(sent.value, 0)) > 0 THEN 'in'
                  ELSE 'out'
                END AS direction,
                t.fee
           FROM page AS p
           JOIN transactions AS t
             ON t.block_height = p.block_height
            AND t.txid = p.txid
           JOIN blocks AS b ON b.height = p.block_height
           LEFT JOIN LATERAL (
             SELECT COALESCE(SUM(o.value), 0)::bigint AS value
               FROM tx_outputs AS o
              WHERE o.block_height = p.block_height
                AND o.txid = p.txid
                AND o.address = $1
           ) AS received ON TRUE
           LEFT JOIN LATERAL (
             SELECT COALESCE(SUM(i.prev_value), 0)::bigint AS value
               FROM tx_inputs AS i
              WHERE i.txid = p.txid
                AND i.prev_address = $1
                AND i.prev_value IS NOT NULL
           ) AS sent ON TRUE
          ORDER BY p.block_height DESC, p.block_pos DESC, p.txid DESC",
      )
      .bind(address)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
  }
  .map_err(|e| pg_err("read address txs", e))?;

  Ok(
    rows
      .into_iter()
      .map(|row| AddressTxRow {
        txid: row.get("txid"),
        block_height: row.get("block_height"),
        block_pos: row.get("block_pos"),
        block_time: row.get("block_time"),
        version: row.get("version"),
        tx_type: row.get("tx_type"),
        size: row.get("size"),
        value_out: row.get("value_out"),
        net_value: row.get("net_value"),
        direction: match row.get::<&str, _>("direction") {
          "in" => AddressTxDirection::In,
          _ => AddressTxDirection::Out,
        },
        fee: row.get("fee"),
      })
      .collect(),
  )
}

/// Hydrate one preselected address-history page from indexed PostgreSQL facts.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn hydrate_address_history_page(
  pool: &PgPool,
  address: &str,
  page: &[(Vec<u8>, i32, i32)],
) -> Result<Vec<AddressTxRow>, DaemonError> {
  use sqlx::Row;

  if page.is_empty() {
    return Ok(Vec::new());
  }

  let txids = page
    .iter()
    .map(|(txid, _, _)| txid.clone())
    .collect::<Vec<_>>();
  let block_heights = page
    .iter()
    .map(|(_, block_height, _)| *block_height)
    .collect::<Vec<_>>();
  let block_positions = page
    .iter()
    .map(|(_, _, block_pos)| *block_pos)
    .collect::<Vec<_>>();

  let rows = sqlx::query(
    "WITH page AS (
       SELECT txid, block_height, block_pos, ord
         FROM unnest($1::bytea[], $2::int[], $3::int[]) WITH ORDINALITY
           AS v(txid, block_height, block_pos, ord)
     )
     SELECT t.txid, p.block_height, p.block_pos, b.time::bigint AS block_time,
            t.version, t.tx_type, t.size, t.value_out,
            (COALESCE(received.value, 0) - COALESCE(sent.value, 0))::bigint AS net_value,
            CASE
              WHEN (COALESCE(received.value, 0) - COALESCE(sent.value, 0)) > 0 THEN 'in'
              ELSE 'out'
            END AS direction,
            t.fee
       FROM page AS p
       JOIN transactions AS t
         ON t.block_height = p.block_height
        AND t.txid = p.txid
       JOIN blocks AS b ON b.height = p.block_height
       LEFT JOIN LATERAL (
         SELECT COALESCE(SUM(o.value), 0)::bigint AS value
           FROM tx_outputs AS o
          WHERE o.block_height = p.block_height
            AND o.txid = p.txid
            AND o.address = $4
       ) AS received ON TRUE
       LEFT JOIN LATERAL (
         SELECT COALESCE(SUM(i.prev_value), 0)::bigint AS value
           FROM tx_inputs AS i
          WHERE i.txid = p.txid
            AND i.prev_address = $4
            AND i.prev_value IS NOT NULL
       ) AS sent ON TRUE
      ORDER BY p.ord",
  )
  .bind(&txids)
  .bind(&block_heights)
  .bind(&block_positions)
  .bind(address)
  .fetch_all(pool)
  .await
  .map_err(|e| pg_err("hydrate address txs", e))?;

  if rows.len() != page.len() {
    tracing::warn!(
      requested_rows = page.len(),
      hydrated_rows = rows.len(),
      address,
      "address history hydration dropped rows"
    );
    return Err(DaemonError::Index(format!(
      "address history hydration mismatch for {address}: requested {}, hydrated {}",
      page.len(),
      rows.len()
    )));
  }

  Ok(
    rows
      .into_iter()
      .map(|row| AddressTxRow {
        txid: row.get("txid"),
        block_height: row.get("block_height"),
        block_pos: row.get("block_pos"),
        block_time: row.get("block_time"),
        version: row.get("version"),
        tx_type: row.get("tx_type"),
        size: row.get("size"),
        value_out: row.get("value_out"),
        net_value: row.get("net_value"),
        direction: match row.get::<&str, _>("direction") {
          "in" => AddressTxDirection::In,
          _ => AddressTxDirection::Out,
        },
        fee: row.get("fee"),
      })
      .collect(),
  )
}

/// Read paginated live UTXOs for one address.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn address_utxos(
  pool: &PgPool,
  address: &str,
  limit: i64,
  before: Option<(i32, &[u8], i32)>,
) -> Result<Vec<AddressUtxoRow>, DaemonError> {
  use sqlx::Row;

  let rows = match before {
    Some((before_height, before_txid, before_vout_index)) => {
      sqlx::query(
        "SELECT txid, vout_index, value, block_height, script_pubkey
         FROM tx_outputs
         WHERE address = $1
           AND spent_txid IS NULL
           AND (
             block_height < $2
             OR (block_height = $2 AND txid < $3)
             OR (block_height = $2 AND txid = $3 AND vout_index < $4)
           )
         ORDER BY block_height DESC, txid DESC, vout_index DESC
         LIMIT $5",
      )
      .bind(address)
      .bind(before_height)
      .bind(before_txid)
      .bind(before_vout_index)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    None => {
      sqlx::query(
        "SELECT txid, vout_index, value, block_height, script_pubkey
         FROM tx_outputs
         WHERE address = $1 AND spent_txid IS NULL
         ORDER BY block_height DESC, txid DESC, vout_index DESC
         LIMIT $2",
      )
      .bind(address)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
  }
  .map_err(|e| pg_err("read address utxos", e))?;

  Ok(
    rows
      .into_iter()
      .map(|row| AddressUtxoRow {
        txid: row.get("txid"),
        vout_index: row.get("vout_index"),
        value: row.get("value"),
        block_height: row.get("block_height"),
        script_pubkey: row.get("script_pubkey"),
      })
      .collect(),
  )
}

/// Read one page of balances entries ordered by balance descending.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn balances_page(
  pool: &PgPool,
  limit: i64,
  after: Option<(i64, &str)>,
) -> Result<Vec<BalancesRow>, DaemonError> {
  use sqlx::Row;

  let rows = match after {
    Some((after_balance, after_address)) => {
      sqlx::query(
        "WITH page_keys AS (
           SELECT address, balance, total_received, tx_count
             FROM (
               SELECT address, balance, total_received, tx_count
                 FROM address_stats
                WHERE balance = $1
                  AND address > $2
                ORDER BY address ASC
                LIMIT $3
             ) AS same_balance
           UNION ALL
           SELECT address, balance, total_received, tx_count
             FROM (
               SELECT address, balance, total_received, tx_count
                 FROM address_stats
                WHERE balance > 0
                  AND balance < $1
                ORDER BY balance DESC, address ASC
                LIMIT $3
             ) AS lower_balances
         ), limited AS (
           SELECT address, balance, total_received, tx_count
             FROM page_keys
            ORDER BY balance DESC, address ASC
            LIMIT $3
         )
         SELECT limited.address,
                limited.balance,
                limited.total_received,
                (limited.total_received - limited.balance)::bigint AS total_sent,
                limited.tx_count
           FROM limited
          ORDER BY limited.balance DESC, limited.address ASC",
      )
      .bind(after_balance)
      .bind(after_address)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    None => {
      sqlx::query(
        "WITH page AS (
           SELECT address, balance, total_received, tx_count
             FROM address_stats
            WHERE balance > 0
            ORDER BY balance DESC, address ASC
            LIMIT $1
         )
         SELECT page.address,
                page.balance,
                page.total_received,
                (page.total_received - page.balance)::bigint AS total_sent,
                page.tx_count
           FROM page
          ORDER BY page.balance DESC, page.address ASC",
      )
      .bind(limit)
      .fetch_all(pool)
      .await
    }
  }
  .map_err(|e| pg_err("read balances", e))?;

  Ok(
    rows
      .into_iter()
      .map(|row| BalancesRow {
        address: row.get("address"),
        balance: row.get("balance"),
        total_received: row.get("total_received"),
        total_sent: row.get("total_sent"),
        tx_count: row.get("tx_count"),
      })
      .collect(),
  )
}

/// Read exact balances summary totals from the current address snapshot.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn balances_summary(pool: &PgPool) -> Result<BalancesSummaryRow, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query(
    "SELECT total, included_supply
       FROM address_balances_summary
      LIMIT 1",
  )
  .fetch_optional(pool)
  .await;

  let row = match row {
    Ok(Some(row)) => row,
    Ok(None) => sqlx::query(
      "SELECT COUNT(*) AS total,
                COALESCE(SUM(balance), 0)::bigint AS included_supply
           FROM address_stats
          WHERE balance > 0",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| pg_err("read balances summary fallback", e))?,
    Err(sqlx::Error::Database(error)) if error.code().as_deref() == Some("42P01") => sqlx::query(
      "SELECT COUNT(*) AS total,
                COALESCE(SUM(balance), 0)::bigint AS included_supply
           FROM address_stats
          WHERE balance > 0",
    )
    .fetch_one(pool)
    .await
    .map_err(|e| pg_err("read balances summary fallback", e))?,
    Err(e) => {
      return Err(pg_err("read balances summary", e));
    }
  };

  Ok(BalancesSummaryRow {
    total: row.get("total"),
    included_supply: row.get("included_supply"),
  })
}

/// Read address suggestions by Base58 prefix.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn search_addresses_by_prefix(
  pool: &PgPool,
  prefix: &str,
  limit: i64,
) -> Result<Vec<AddressSearchRow>, DaemonError> {
  use sqlx::Row;

  let pattern = format!("{prefix}%");
  let rows = sqlx::query(
    "SELECT address
       FROM address_stats
      WHERE address LIKE $1
      ORDER BY last_seen_height DESC NULLS LAST, tx_count DESC, address ASC
      LIMIT $2",
  )
  .bind(pattern)
  .bind(limit)
  .fetch_all(pool)
  .await
  .map_err(|e| pg_err("search address prefix", e))?;

  Ok(
    rows
      .into_iter()
      .map(|row| AddressSearchRow {
        address: row.get("address"),
      })
      .collect(),
  )
}

#[derive(Debug, Clone)]
pub struct PrevoutLookupRow {
  pub prev_txid: Vec<u8>,
  pub prev_vout: i32,
  pub prev_height: i32,
  pub prev_spent_txid: Option<Vec<u8>>,
  pub prev_spent_vin: Option<i32>,
  pub prev_spent_height: Option<i32>,
  pub prev_addr: Option<String>,
  pub prev_value: i64,
}

/// Read current output state for prevouts before applying spent updates.
pub async fn lookup_prev_outputs(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  prev_txids: &[Vec<u8>],
  prev_vouts: &[i32],
) -> Result<Vec<PrevoutLookupRow>, DaemonError> {
  let n = prev_txids.len();
  if n == 0 {
    return Ok(Vec::new());
  }

  let mut all_rows = Vec::with_capacity(n);

  for start in (0..n).step_by(PREVOUT_LOOKUP_CHUNK_SIZE) {
    let end = (start + PREVOUT_LOOKUP_CHUNK_SIZE).min(n);
    let rows = sqlx::query_as::<
      _,
      (
        Vec<u8>,
        i32,
        i32,
        Option<Vec<u8>>,
        Option<i32>,
        Option<i32>,
        Option<String>,
        i64,
      ),
    >(
      "SELECT o.txid, o.vout_index, o.block_height, o.spent_txid, o.spent_vin, o.spent_height,
              o.address, o.value
         FROM tx_outputs AS o
         JOIN unnest($1::bytea[], $2::int[]) AS v(ptxid, pvout)
           ON o.txid = v.ptxid AND o.vout_index = v.pvout",
    )
    .bind(&prev_txids[start..end])
    .bind(&prev_vouts[start..end])
    .fetch_all(&mut **tx)
    .await
    .map_err(|e| pg_err("lookup prev outputs", e))?;
    all_rows.extend(rows.into_iter().map(|row| PrevoutLookupRow {
      prev_txid: row.0,
      prev_vout: row.1,
      prev_height: row.2,
      prev_spent_txid: row.3,
      prev_spent_vin: row.4,
      prev_spent_height: row.5,
      prev_addr: row.6,
      prev_value: row.7,
    }));
  }

  Ok(all_rows)
}

pub async fn prepare_prevout_stage(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), DaemonError> {
  if !temp_table_exists(tx, "tmp_prevout_spend_delta").await? {
    sqlx::query(
      "CREATE TEMP TABLE tmp_prevout_spend_delta (
         stage_id bigint GENERATED ALWAYS AS IDENTITY,
         prev_txid bytea NOT NULL,
         prev_vout int NOT NULL,
         prev_height int NOT NULL,
         spending_txid bytea NOT NULL,
         spending_vin int NOT NULL,
         spending_height int NOT NULL,
         prev_spent_txid bytea,
         prev_spent_vin int,
         prev_spent_height int,
         prev_address text,
         prev_value bigint NOT NULL
       ) ON COMMIT PRESERVE ROWS",
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("create prevout staging", e))?;
  }

  sqlx::query("TRUNCATE tmp_prevout_spend_delta RESTART IDENTITY")
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("truncate prevout staging", e))?;
  Ok(())
}

#[cfg(test)]
mod tests {
  use sqlx::PgPool;

  use crate::error::DaemonError;
  use crate::rest::openapi::AddressTxDirection;

  use super::{
    address_exists, address_history_txs, address_summary, address_utxos,
    apply_address_balances_chunk, apply_address_balances_summary_chunk, apply_address_stats_chunk,
    apply_address_stats_summary_chunk, apply_staged_address_balances_chunk,
    apply_staged_address_balances_summary_chunk, balances_page, balances_summary,
    block_time_by_height, mark_outputs_spent_chunk, prepare_address_balances_staging,
    prepare_prevout_stage, refresh_address_balances_summary, refresh_address_stats_for_addresses,
    reverse_address_balances_at_height, reverse_address_history_at_height,
    reverse_address_stats_at_height, stage_address_balances_delta, stage_prevout_snapshot_rows,
    staged_address_balances_count,
  };

  /// Build one stable test hash for seeded spent-route rows.
  fn synthetic_hash(domain: u8, id: i32) -> Vec<u8> {
    let mut value = [0_u8; 32];
    value[0] = domain;
    value[1..5].copy_from_slice(&id.to_be_bytes());
    for (index, byte) in value[5..].iter_mut().enumerate() {
      *byte = domain.wrapping_add(index as u8).wrapping_mul(13);
    }
    value.to_vec()
  }

  /// Seed one canonical block row so spent-route reads can resolve heights and times.
  async fn seed_block(pool: &PgPool, height: i32) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO blocks (
         height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count, chainwork
       ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)",
    )
    .bind(height)
    .bind(synthetic_hash(b'B', height))
    .bind(if height == 0 {
      vec![0_u8; 32]
    } else {
      synthetic_hash(b'B', height - 1)
    })
    .bind(synthetic_hash(b'M', height))
    .bind(height)
    .bind(486_604_799_i32)
    .bind(i64::from(height))
    .bind(1_i32)
    .bind(1024_i32)
    .bind(1_i32)
    .bind(format!("{height:064x}"))
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one canonical transaction row at a height already present in `blocks`.
  async fn seed_transaction(
    pool: &PgPool,
    height: i32,
    txid: &[u8],
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
       ) VALUES ($1, $2, $3, $4, 'normal', $5, $6, $7, $8, $9, $10)",
    )
    .bind(txid)
    .bind(height)
    .bind(0_i32)
    .bind(3_i32)
    .bind(0_i32)
    .bind(200_i32)
    .bind(1_000_i64)
    .bind(1_i32)
    .bind(1_i32)
    .bind(Some(10_i64))
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one address history series using real transaction and address-history tables.
  async fn seed_address_history(
    pool: &PgPool,
    address: &str,
    count: i32,
  ) -> Result<(), Box<dyn std::error::Error>> {
    for height in 0..count {
      let txid = synthetic_hash(b'T', height);
      sqlx::query(
        "INSERT INTO transactions (
           txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
         ) VALUES ($1, $2, $3, $4, 'normal', $5, $6, $7, $8, $9, $10)",
      )
      .bind(&txid)
      .bind(height)
      .bind(0_i32)
      .bind(3_i32)
      .bind(0_i32)
      .bind(200_i32)
      .bind(1_000_i64)
      .bind(1_i32)
      .bind(1_i32)
      .bind(Some(10_i64))
      .execute(pool)
      .await?;

      sqlx::query(
        "INSERT INTO address_txs (address, txid, block_height, block_pos)
         VALUES ($1, $2, $3, $4)",
      )
      .bind(address)
      .bind(&txid)
      .bind(height)
      .bind(0_i32)
      .execute(pool)
      .await?;
    }
    Ok(())
  }

  /// Seed one descending balances corpus across address stats and ranking tables.
  async fn seed_balances(pool: &PgPool, rows: i32) -> Result<(), Box<dyn std::error::Error>> {
    for index in 0..rows {
      let address = format!("XspentBalance{index:08}");
      let balance = i64::from(rows - index) * 1_000;
      let total_received = balance + 500;
      let tx_count = 1 + (index % 3);
      sqlx::query(
        "INSERT INTO address_stats (
           address, balance, total_received, tx_count, first_seen_height, last_seen_height
         ) VALUES ($1, $2, $3, $4, $5, $6)",
      )
      .bind(&address)
      .bind(balance)
      .bind(total_received)
      .bind(tx_count)
      .bind(0_i32)
      .bind(0_i32)
      .execute(pool)
      .await?;
    }
    Ok(())
  }

  /// Seed one single ranked balance row with matching summary payload fields.
  async fn seed_balance_row(
    pool: &PgPool,
    address: &str,
    balance: i64,
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO address_stats (
         address, balance, total_received, tx_count, first_seen_height, last_seen_height
       ) VALUES ($1, $2, $3, $4, $5, $6)",
    )
    .bind(address)
    .bind(balance)
    .bind(balance + 500)
    .bind(1_i32)
    .bind(0_i32)
    .bind(0_i32)
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one spendable tx output row for address UTXO route tests.
  async fn seed_utxo(
    pool: &PgPool,
    address: &str,
    block_height: i32,
    txid: Vec<u8>,
    vout_index: i32,
    value: i64,
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO tx_outputs (
         txid, vout_index, block_height, value, script_pubkey, script_type, address, spent_txid, spent_vin_index
       ) VALUES ($1, $2, $3, $4, $5, 'p2pkh', $6, NULL, NULL)",
    )
    .bind(txid)
    .bind(vout_index)
    .bind(block_height)
    .bind(value)
    .bind(vec![0x51_u8])
    .bind(address)
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Read one typed address-stats row after an apply-path mutation test.
  async fn fetch_address_stats_row(
    pool: &PgPool,
    address: &str,
  ) -> Result<(i64, i64, i32, i32, Option<i32>, Option<i32>), Box<dyn std::error::Error>> {
    let row = sqlx::query_as::<_, (i64, i64, i32, i32, Option<i32>, Option<i32>)>(
      "SELECT balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
         FROM address_stats
        WHERE address = $1",
    )
    .bind(address)
    .fetch_one(pool)
    .await?;
    Ok(row)
  }

  async fn fetch_optional_address_stats_row(
    pool: &PgPool,
    address: &str,
  ) -> Result<Option<(i64, i64, i32, i32, Option<i32>, Option<i32>)>, Box<dyn std::error::Error>>
  {
    let row = sqlx::query_as::<_, (i64, i64, i32, i32, Option<i32>, Option<i32>)>(
      "SELECT balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
         FROM address_stats
        WHERE address = $1",
    )
    .bind(address)
    .fetch_optional(pool)
    .await?;
    Ok(row)
  }

  async fn fetch_address_history_rows(
    pool: &PgPool,
    address: &str,
  ) -> Result<Vec<(i32, Vec<u8>)>, Box<dyn std::error::Error>> {
    let rows = sqlx::query_as::<_, (i32, Vec<u8>)>(
      "SELECT block_height, txid
         FROM address_txs
        WHERE address = $1
        ORDER BY block_height ASC, block_pos ASC, txid ASC",
    )
    .bind(address)
    .fetch_all(pool)
    .await?;
    Ok(rows)
  }

  async fn seed_address_history_tx(
    pool: &PgPool,
    address: &str,
    height: i32,
    block_pos: i32,
    txid: Vec<u8>,
    value_out: i64,
    fee: i64,
    received_value: i64,
    sent_value: i64,
  ) -> Result<(), Box<dyn std::error::Error>> {
    seed_block(pool, height).await?;
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
       ) VALUES ($1, $2, $3, 3, 'normal', 0, 200, $4, 1, 1, $5)",
    )
    .bind(&txid)
    .bind(height)
    .bind(block_pos)
    .bind(value_out)
    .bind(Some(fee))
    .execute(pool)
    .await?;
    sqlx::query(
      "INSERT INTO address_txs (address, txid, block_height, block_pos)
       VALUES ($1, $2, $3, $4)",
    )
    .bind(address)
    .bind(&txid)
    .bind(height)
    .bind(block_pos)
    .execute(pool)
    .await?;
    if received_value > 0 {
      sqlx::query(
        "INSERT INTO tx_outputs (
           txid, vout_index, block_height, value, script_pubkey, script_type, address
         ) VALUES ($1, 0, $2, $3, $4, 'p2pkh', $5)",
      )
      .bind(&txid)
      .bind(height)
      .bind(received_value)
      .bind(vec![0x51_u8])
      .bind(address)
      .execute(pool)
      .await?;
    }
    if sent_value > 0 {
      sqlx::query(
        "INSERT INTO tx_inputs (
           txid, vin_index, prev_txid, prev_vout, script_sig, sequence, prev_address, prev_value
         ) VALUES ($1, 0, NULL, NULL, $2, 0, $3, $4)",
      )
      .bind(&txid)
      .bind(vec![0x51_u8])
      .bind(address)
      .bind(sent_value)
      .execute(pool)
      .await?;
    }
    Ok(())
  }

  async fn seed_reverse_address_stats_parity_fixture(
    pool: &PgPool,
  ) -> Result<[String; 2], Box<dyn std::error::Error>> {
    for height in 7..=11 {
      seed_block(pool, height).await?;
    }

    let addresses = [
      "XreverseParityA1111111111111111111111".to_owned(),
      "XreverseParityB1111111111111111111111".to_owned(),
    ];
    let tx_rows = [
      (synthetic_hash(b'P', 7), addresses[0].clone(), 7_i32, 40_i64),
      (synthetic_hash(b'P', 9), addresses[0].clone(), 9_i32, 60_i64),
      (
        synthetic_hash(b'P', 11),
        addresses[0].clone(),
        11_i32,
        80_i64,
      ),
      (synthetic_hash(b'Z', 8), addresses[1].clone(), 8_i32, 30_i64),
      (
        synthetic_hash(b'Z', 11),
        addresses[1].clone(),
        11_i32,
        20_i64,
      ),
    ];

    for (txid, address, block_height, value) in &tx_rows {
      sqlx::query(
        "INSERT INTO transactions (
           txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
         ) VALUES ($1, $2, 0, 3, 'normal', 0, 200, 1_000, 1, 1, 10)",
      )
      .bind(txid)
      .bind(block_height)
      .execute(pool)
      .await?;
      sqlx::query(
        "INSERT INTO address_txs (address, txid, block_height, block_pos)
         VALUES ($1, $2, $3, 0)",
      )
      .bind(address)
      .bind(txid)
      .bind(block_height)
      .execute(pool)
      .await?;
      seed_utxo(pool, address, *block_height, txid.clone(), 0, *value).await?;
    }

    let mut seed_tx = pool.begin().await?;
    refresh_address_stats_for_addresses(&mut seed_tx, &addresses).await?;
    seed_tx.commit().await?;

    Ok(addresses)
  }

  /// Read one typed address-balance scalar after a staged apply-path mutation test.
  async fn fetch_address_balance(
    pool: &PgPool,
    address: &str,
  ) -> Result<i64, Box<dyn std::error::Error>> {
    let balance = sqlx::query_scalar::<_, i64>(
      "SELECT balance
         FROM address_balances
        WHERE address = $1",
    )
    .bind(address)
    .fetch_one(pool)
    .await?;
    Ok(balance)
  }

  /// Update one seeded address-stats row to create a mixed upsert scenario.
  async fn set_address_stats_fields(
    pool: &PgPool,
    address: &str,
    total_received: i64,
    tx_count: i32,
    utxo_count: i32,
    first_seen_height: i32,
    last_seen_height: i32,
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "UPDATE address_stats
          SET total_received = $2,
              tx_count = $3,
              utxo_count = $4,
              first_seen_height = $5,
              last_seen_height = $6
        WHERE address = $1",
    )
    .bind(address)
    .bind(total_received)
    .bind(tx_count)
    .bind(utxo_count)
    .bind(first_seen_height)
    .bind(last_seen_height)
    .execute(pool)
    .await?;
    Ok(())
  }

  #[tokio::test]
  async fn address_history_later_page_reads_older_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    for height in 0..5 {
      seed_block(&pool, height).await?;
    }
    seed_address_history(&pool, "XspentBench1111111111111111111111111", 5).await?;

    let cursor_txid = synthetic_hash(b'T', 3);
    let rows = address_history_txs(
      &pool,
      "XspentBench1111111111111111111111111",
      10,
      Some((3, 0, cursor_txid.as_slice())),
    )
    .await?;

    assert_eq!(rows.len(), 3);
    assert_eq!(rows[0].block_height, 2);
    assert_eq!(rows[1].block_height, 1);
    assert_eq!(rows[2].block_height, 0);

    Ok(())
  }

  #[tokio::test]
  async fn balances_page_reads_first_and_later_pages_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;
    seed_balances(&pool, 6).await?;

    let first_page = balances_page(&pool, 3, None).await?;
    assert_eq!(first_page.len(), 3);
    assert_eq!(first_page[0].balance, 6_000);
    assert_eq!(first_page[1].balance, 5_000);
    assert_eq!(first_page[2].balance, 4_000);
    assert_eq!(first_page[0].total_sent, 500);

    let later_page = balances_page(&pool, 3, Some((4_000, first_page[2].address.as_str()))).await?;
    assert_eq!(later_page.len(), 3);
    assert_eq!(later_page[0].balance, 3_000);
    assert_eq!(later_page[1].balance, 2_000);
    assert_eq!(later_page[2].balance, 1_000);

    Ok(())
  }

  #[tokio::test]
  async fn balances_page_keeps_same_balance_tail_ahead_of_lower_balances_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    seed_balance_row(&pool, "XsameA", 5_000).await?;
    seed_balance_row(&pool, "XsameB", 5_000).await?;
    seed_balance_row(&pool, "Xlower", 4_000).await?;

    let first_page = balances_page(&pool, 1, None).await?;
    assert_eq!(first_page.len(), 1);
    assert_eq!(first_page[0].address, "XsameA");

    let later_page = balances_page(&pool, 2, Some((5_000, "XsameA"))).await?;
    assert_eq!(later_page.len(), 2);
    assert_eq!(later_page[0].address, "XsameB");
    assert_eq!(later_page[1].address, "Xlower");

    Ok(())
  }

  #[tokio::test]
  async fn address_summary_reads_seeded_totals_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    seed_balance_row(&pool, "Xsummary", 5_000).await?;

    let summary = address_summary(&pool, "Xsummary").await?;
    assert!(summary.is_some());
    let summary = summary.expect("seeded address summary should exist");
    assert_eq!(summary.balance, 5_000);
    assert_eq!(summary.total_received, 5_500);
    assert_eq!(summary.total_sent, 500);
    assert_eq!(summary.tx_count, 1);
    assert_eq!(summary.first_seen_height, Some(0));
    assert_eq!(summary.last_seen_height, Some(0));

    Ok(())
  }

  #[tokio::test]
  async fn address_exists_tracks_seeded_address_stats_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    assert!(!address_exists(&pool, "Xmissing").await?);
    seed_balance_row(&pool, "Xexists", 1_000).await?;
    assert!(address_exists(&pool, "Xexists").await?);

    Ok(())
  }

  #[tokio::test]
  async fn block_time_by_height_reads_seeded_time_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    seed_block(&pool, 42).await?;
    assert_eq!(block_time_by_height(&pool, 42).await?, Some(42));
    assert_eq!(block_time_by_height(&pool, 43).await?, None);

    Ok(())
  }

  #[tokio::test]
  async fn address_utxos_pages_newest_unspent_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    seed_utxo(&pool, "Xutxo", 10, synthetic_hash(b'U', 3), 0, 3_000).await?;
    seed_utxo(&pool, "Xutxo", 10, synthetic_hash(b'U', 2), 1, 2_000).await?;
    seed_utxo(&pool, "Xutxo", 9, synthetic_hash(b'U', 1), 0, 1_000).await?;

    let first_page = address_utxos(&pool, "Xutxo", 2, None).await?;
    assert_eq!(first_page.len(), 2);
    assert_eq!(first_page[0].block_height, 10);
    assert_eq!(first_page[0].value, 3_000);
    assert_eq!(first_page[1].vout_index, 1);

    let later_page = address_utxos(
      &pool,
      "Xutxo",
      2,
      Some((
        first_page[1].block_height,
        first_page[1].txid.as_slice(),
        first_page[1].vout_index,
      )),
    )
    .await?;
    assert_eq!(later_page.len(), 1);
    assert_eq!(later_page[0].block_height, 9);
    assert_eq!(later_page[0].value, 1_000);

    Ok(())
  }

  #[tokio::test]
  async fn balances_summary_reads_positive_supply_snapshot_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    seed_balance_row(&pool, "XbalanceA", 5_000).await?;
    seed_balance_row(&pool, "XbalanceB", 3_000).await?;

    let summary = balances_summary(&pool).await?;
    assert_eq!(summary.total, 2);
    assert_eq!(summary.included_supply, 8_000);

    Ok(())
  }

  #[tokio::test]
  async fn apply_address_stats_chunk_upserts_mixed_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    seed_balance_row(&pool, "Xexisting", 5_000).await?;
    set_address_stats_fields(&pool, "Xexisting", 7_000, 3, 7, 10, 20).await?;

    let mut tx = pool.begin().await?;
    apply_address_stats_chunk(
      &mut tx,
      &["Xexisting".to_owned(), "Xnew".to_owned()],
      &[100, 300],
      &[200, 600],
      &[1, 2],
      &[1, 2],
      &[Some(5), Some(30)],
      &[Some(25), Some(30)],
    )
    .await?;
    tx.commit().await?;

    let existing = fetch_address_stats_row(&pool, "Xexisting").await?;
    assert_eq!(existing.0, 5_100);
    assert_eq!(existing.1, 7_200);
    assert_eq!(existing.2, 4);
    assert_eq!(existing.3, 8);
    assert_eq!(existing.4, Some(5));
    assert_eq!(existing.5, Some(25));

    let new_row = fetch_address_stats_row(&pool, "Xnew").await?;
    assert_eq!(new_row.0, 300);
    assert_eq!(new_row.1, 600);
    assert_eq!(new_row.2, 2);
    assert_eq!(new_row.3, 2);
    assert_eq!(new_row.4, Some(30));
    assert_eq!(new_row.5, Some(30));

    Ok(())
  }

  #[tokio::test]
  async fn apply_staged_address_balances_chunk_upserts_mixed_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut seed_tx = pool.begin().await?;
    sqlx::query(
      "INSERT INTO address_balances (address, balance)
       VALUES ('Xexisting', 5000)",
    )
    .execute(&mut *seed_tx)
    .await?;
    seed_tx.commit().await?;

    let mut tx = pool.begin().await?;
    prepare_address_balances_staging(&mut tx).await?;
    stage_address_balances_delta(
      &mut tx,
      &["Xexisting".to_owned(), "Xnew".to_owned()],
      &[100, 300],
    )
    .await?;
    let staged = staged_address_balances_count(&mut tx).await?;
    apply_staged_address_balances_summary_chunk(&mut tx, 0, staged).await?;
    apply_staged_address_balances_chunk(&mut tx, 0, staged).await?;
    tx.commit().await?;

    let existing = fetch_address_balance(&pool, "Xexisting").await?;
    assert_eq!(existing, 5_100);

    let new_balance = fetch_address_balance(&pool, "Xnew").await?;
    assert_eq!(new_balance, 300);

    let summary = balances_summary(&pool).await?;
    assert_eq!(summary.total, 2);
    assert_eq!(summary.included_supply, 5_400);

    Ok(())
  }

  #[tokio::test]
  async fn apply_address_balances_chunk_upserts_mixed_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut seed_tx = pool.begin().await?;
    sqlx::query(
      "INSERT INTO address_balances (address, balance)
       VALUES ('Xexisting', 5000)",
    )
    .execute(&mut *seed_tx)
    .await?;
    refresh_address_balances_summary(&mut seed_tx).await?;
    seed_tx.commit().await?;

    let mut tx = pool.begin().await?;
    apply_address_balances_summary_chunk(
      &mut tx,
      &["Xexisting".to_owned(), "Xnew".to_owned()],
      &[100, 300],
    )
    .await?;
    apply_address_balances_chunk(
      &mut tx,
      &["Xexisting".to_owned(), "Xnew".to_owned()],
      &[100, 300],
    )
    .await?;
    tx.commit().await?;

    let existing = fetch_address_balance(&pool, "Xexisting").await?;
    assert_eq!(existing, 5_100);

    let new_balance = fetch_address_balance(&pool, "Xnew").await?;
    assert_eq!(new_balance, 300);

    let summary = balances_summary(&pool).await?;
    assert_eq!(summary.total, 2);
    assert_eq!(summary.included_supply, 5_400);

    Ok(())
  }

  #[tokio::test]
  async fn apply_staged_address_balances_summary_chunk_handles_zero_crossings_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut seed_tx = pool.begin().await?;
    sqlx::query(
      "INSERT INTO address_balances (address, balance)
       VALUES ('Xdrop', 50), ('Xgrow', 10)",
    )
    .execute(&mut *seed_tx)
    .await?;
    refresh_address_balances_summary(&mut seed_tx).await?;
    seed_tx.commit().await?;

    let mut tx = pool.begin().await?;
    prepare_address_balances_staging(&mut tx).await?;
    stage_address_balances_delta(
      &mut tx,
      &["Xdrop".to_owned(), "Xgrow".to_owned(), "Xnew".to_owned()],
      &[-50, 20, 30],
    )
    .await?;
    let staged = staged_address_balances_count(&mut tx).await?;
    apply_staged_address_balances_summary_chunk(&mut tx, 0, staged).await?;
    apply_staged_address_balances_chunk(&mut tx, 0, staged).await?;
    tx.commit().await?;

    let summary = balances_summary(&pool).await?;
    assert_eq!(summary.total, 2);
    assert_eq!(summary.included_supply, 60);
    Ok(())
  }

  #[tokio::test]
  async fn apply_address_balances_summary_chunk_handles_zero_crossings_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut seed_tx = pool.begin().await?;
    sqlx::query(
      "INSERT INTO address_balances (address, balance)
       VALUES ('Xdrop', 50), ('Xgrow', 10)",
    )
    .execute(&mut *seed_tx)
    .await?;
    refresh_address_balances_summary(&mut seed_tx).await?;
    seed_tx.commit().await?;

    let mut tx = pool.begin().await?;
    apply_address_balances_summary_chunk(
      &mut tx,
      &["Xdrop".to_owned(), "Xgrow".to_owned(), "Xnew".to_owned()],
      &[-50, 20, 30],
    )
    .await?;
    apply_address_balances_chunk(
      &mut tx,
      &["Xdrop".to_owned(), "Xgrow".to_owned(), "Xnew".to_owned()],
      &[-50, 20, 30],
    )
    .await?;
    tx.commit().await?;

    let summary = balances_summary(&pool).await?;
    assert_eq!(summary.total, 2);
    assert_eq!(summary.included_supply, 60);
    Ok(())
  }

  #[tokio::test]
  async fn apply_address_stats_summary_chunk_handles_zero_crossings_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut seed_tx = pool.begin().await?;
    sqlx::query(
      "INSERT INTO address_stats (
         address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
       ) VALUES
         ('Xdrop', 50, 50, 1, 1, 1, 1),
         ('Xgrow', 10, 10, 1, 1, 1, 1)",
    )
    .execute(&mut *seed_tx)
    .await?;
    refresh_address_balances_summary(&mut seed_tx).await?;
    seed_tx.commit().await?;

    let mut tx = pool.begin().await?;
    apply_address_stats_summary_chunk(
      &mut tx,
      &["Xdrop".to_owned(), "Xgrow".to_owned(), "Xnew".to_owned()],
      &[-50, 20, 30],
    )
    .await?;
    apply_address_stats_chunk(
      &mut tx,
      &["Xdrop".to_owned(), "Xgrow".to_owned(), "Xnew".to_owned()],
      &[-50, 20, 30],
      &[0, 0, 30],
      &[0, 0, 1],
      &[-1, 0, 1],
      &[None, None, Some(2)],
      &[Some(2), Some(2), Some(2)],
    )
    .await?;
    tx.commit().await?;

    let summary = balances_summary(&pool).await?;
    assert_eq!(summary.total, 2);
    assert_eq!(summary.included_supply, 60);
    Ok(())
  }

  #[tokio::test]
  async fn apply_staged_address_balances_summary_chunk_allows_negative_delta_when_final_total_stays_positive(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut seed_tx = pool.begin().await?;
    sqlx::query(
      "INSERT INTO address_balances (address, balance)
       VALUES ('Xdrop', 50), ('Xstay', 10)",
    )
    .execute(&mut *seed_tx)
    .await?;
    refresh_address_balances_summary(&mut seed_tx).await?;
    seed_tx.commit().await?;

    let mut tx = pool.begin().await?;
    prepare_address_balances_staging(&mut tx).await?;
    stage_address_balances_delta(&mut tx, &["Xdrop".to_owned()], &[-50]).await?;
    let staged = staged_address_balances_count(&mut tx).await?;
    apply_staged_address_balances_summary_chunk(&mut tx, 0, staged).await?;
    apply_staged_address_balances_chunk(&mut tx, 0, staged).await?;
    tx.commit().await?;

    let summary = balances_summary(&pool).await?;
    assert_eq!(summary.total, 1);
    assert_eq!(summary.included_supply, 10);
    Ok(())
  }

  #[tokio::test]
  async fn apply_staged_address_balances_chunk_aggregates_duplicate_addresses_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut seed_tx = pool.begin().await?;
    sqlx::query(
      "INSERT INTO address_balances (address, balance)
       VALUES ('Xdup', 10)",
    )
    .execute(&mut *seed_tx)
    .await?;
    refresh_address_balances_summary(&mut seed_tx).await?;
    seed_tx.commit().await?;

    let mut tx = pool.begin().await?;
    prepare_address_balances_staging(&mut tx).await?;
    stage_address_balances_delta(
      &mut tx,
      &["Xdup".to_owned(), "Xdup".to_owned(), "Xnew".to_owned()],
      &[5, -3, 7],
    )
    .await?;
    let staged = staged_address_balances_count(&mut tx).await?;
    apply_staged_address_balances_summary_chunk(&mut tx, 0, staged).await?;
    apply_staged_address_balances_chunk(&mut tx, 0, staged).await?;
    tx.commit().await?;

    let dup_balance = fetch_address_balance(&pool, "Xdup").await?;
    assert_eq!(dup_balance, 12);
    let new_balance = fetch_address_balance(&pool, "Xnew").await?;
    assert_eq!(new_balance, 7);

    let summary = balances_summary(&pool).await?;
    assert_eq!(summary.total, 2);
    assert_eq!(summary.included_supply, 19);

    Ok(())
  }

  #[tokio::test]
  async fn restore_spent_at_height_allows_blocks_without_prevout_undo_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut tx = pool.begin().await?;
    crate::sql::undo::upsert_block_undo_meta(&mut tx, 7, &[7_u8; 32], &[6_u8; 32], 0).await?;
    super::restore_spent_at_height(&mut tx, 7).await?;
    tx.commit().await?;

    Ok(())
  }

  #[tokio::test]
  async fn restore_spent_at_height_rejects_missing_prevout_undo_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let txid = vec![7_u8; 32];
    let prev_txid = vec![6_u8; 32];
    let mut tx = pool.begin().await?;
    crate::sql::undo::upsert_block_undo_meta(&mut tx, 7, &txid, &prev_txid, 0).await?;
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
       ) VALUES ($1, 7, 0, 3, 'normal', 0, 100, 0, 1, 0, 0)",
    )
    .bind(&txid)
    .execute(&mut *tx)
    .await?;
    sqlx::query(
      "INSERT INTO tx_inputs (
         txid, vin_index, prev_txid, prev_vout, script_sig, sequence
       ) VALUES ($1, 0, $2, 0, $3, 0)",
    )
    .bind(&txid)
    .bind(&prev_txid)
    .bind(vec![0x51_u8])
    .execute(&mut *tx)
    .await?;

    let error = super::restore_spent_at_height(&mut tx, 7)
      .await
      .expect_err("missing prevout undo rows should fail");
    assert!(
      error
        .to_string()
        .contains("block_undo_outputs mismatch at 7: expected 1, found 0"),
      "{error}"
    );

    tx.rollback().await?;
    Ok(())
  }

  #[tokio::test]
  async fn apply_staged_address_balances_summary_chunk_repairs_stale_singleton_before_retry(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut seed_tx = pool.begin().await?;
    sqlx::query(
      "INSERT INTO address_balances (address, balance)
       VALUES ('Xstay', 20), ('Xother', 15)",
    )
    .execute(&mut *seed_tx)
    .await?;
    refresh_address_balances_summary(&mut seed_tx).await?;
    seed_tx.commit().await?;

    sqlx::query(
      "INSERT INTO address_balances_summary (singleton, total, included_supply)
       VALUES (TRUE, -214, 1563655803660)
       ON CONFLICT (singleton) DO UPDATE SET
         total = EXCLUDED.total,
         included_supply = EXCLUDED.included_supply",
    )
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    prepare_address_balances_staging(&mut tx).await?;
    stage_address_balances_delta(&mut tx, &["Xnew".to_owned()], &[30]).await?;
    let staged = staged_address_balances_count(&mut tx).await?;
    apply_staged_address_balances_summary_chunk(&mut tx, 0, staged).await?;
    apply_staged_address_balances_chunk(&mut tx, 0, staged).await?;
    tx.commit().await?;

    let summary = balances_summary(&pool).await?;
    assert_eq!(summary.total, 3);
    assert_eq!(summary.included_supply, 65);
    Ok(())
  }

  #[tokio::test]
  async fn apply_address_stats_chunk_aggregates_duplicate_addresses_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut tx = pool.begin().await?;
    apply_address_stats_chunk(
      &mut tx,
      &["Xdup".to_owned(), "Xdup".to_owned()],
      &[100, 200],
      &[400, 600],
      &[1, 2],
      &[1, 1],
      &[Some(20), Some(10)],
      &[Some(20), Some(30)],
    )
    .await?;
    tx.commit().await?;

    let row = fetch_address_stats_row(&pool, "Xdup").await?;
    assert_eq!(row.0, 300);
    assert_eq!(row.1, 1_000);
    assert_eq!(row.2, 3);
    assert_eq!(row.3, 2);
    assert_eq!(row.4, Some(10));
    assert_eq!(row.5, Some(30));

    Ok(())
  }

  #[tokio::test]
  async fn apply_address_stats_chunk_matches_exact_rebuild_for_touched_addresses_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    for height in 9..=11 {
      seed_block(&pool, height).await?;
    }
    let txid_a9 = synthetic_hash(b'S', 9);
    let txid_a10 = synthetic_hash(b'S', 10);
    let txid_a11 = synthetic_hash(b'S', 11);
    let txid_b11 = synthetic_hash(b'V', 11);
    for (txid, height, value) in [(&txid_a9, 9_i32, 40_i64), (&txid_a10, 10_i32, 60_i64)] {
      sqlx::query(
        "INSERT INTO transactions (
           txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
         ) VALUES ($1, $2, 0, 3, 'normal', 0, 200, 1_000, 1, 1, 10)",
      )
      .bind(txid)
      .bind(height)
      .execute(&pool)
      .await?;
      sqlx::query(
        "INSERT INTO address_txs (address, txid, block_height, block_pos)
         VALUES ($1, $2, $3, 0)",
      )
      .bind("XapplyRepair111111111111111111111111")
      .bind(txid)
      .bind(height)
      .execute(&pool)
      .await?;
      seed_utxo(
        &pool,
        "XapplyRepair111111111111111111111111",
        height,
        txid.clone(),
        0,
        value,
      )
      .await?;
    }

    let mut seed_tx = pool.begin().await?;
    refresh_address_stats_for_addresses(
      &mut seed_tx,
      &["XapplyRepair111111111111111111111111".to_owned()],
    )
    .await?;
    seed_tx.commit().await?;

    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
       ) VALUES
         ($1, 11, 0, 3, 'normal', 0, 200, 1_000, 1, 1, 10),
         ($2, 11, 1, 3, 'normal', 0, 200, 1_000, 1, 1, 10)",
    )
    .bind(&txid_a11)
    .bind(&txid_b11)
    .execute(&pool)
    .await?;
    sqlx::query(
      "INSERT INTO address_txs (address, txid, block_height, block_pos)
       VALUES
         ($1, $2, 11, 0),
         ($3, $4, 11, 1)",
    )
    .bind("XapplyRepair111111111111111111111111")
    .bind(&txid_a11)
    .bind("XapplyRepairNew111111111111111111111")
    .bind(&txid_b11)
    .execute(&pool)
    .await?;
    seed_utxo(
      &pool,
      "XapplyRepair111111111111111111111111",
      11,
      txid_a11.clone(),
      0,
      80,
    )
    .await?;
    seed_utxo(
      &pool,
      "XapplyRepairNew111111111111111111111",
      11,
      txid_b11.clone(),
      0,
      20,
    )
    .await?;

    let mut tx = pool.begin().await?;
    apply_address_stats_chunk(
      &mut tx,
      &[
        "XapplyRepair111111111111111111111111".to_owned(),
        "XapplyRepairNew111111111111111111111".to_owned(),
      ],
      &[80, 20],
      &[80, 20],
      &[1, 1],
      &[1, 1],
      &[Some(11), Some(11)],
      &[Some(11), Some(11)],
    )
    .await?;
    tx.commit().await?;

    let incremental_rows = (
      fetch_address_stats_row(&pool, "XapplyRepair111111111111111111111111").await?,
      fetch_address_stats_row(&pool, "XapplyRepairNew111111111111111111111").await?,
    );

    let mut exact_tx = pool.begin().await?;
    refresh_address_stats_for_addresses(
      &mut exact_tx,
      &[
        "XapplyRepair111111111111111111111111".to_owned(),
        "XapplyRepairNew111111111111111111111".to_owned(),
      ],
    )
    .await?;
    exact_tx.commit().await?;
    let exact_rows = (
      fetch_address_stats_row(&pool, "XapplyRepair111111111111111111111111").await?,
      fetch_address_stats_row(&pool, "XapplyRepairNew111111111111111111111").await?,
    );

    assert_eq!(incremental_rows, exact_rows);
    assert_eq!(incremental_rows.0, (180, 180, 3, 3, Some(9), Some(11)));
    assert_eq!(incremental_rows.1, (20, 20, 1, 1, Some(11), Some(11)));

    Ok(())
  }

  #[tokio::test]
  async fn apply_address_stats_chunk_deletes_fully_zeroed_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    sqlx::query(
      "INSERT INTO address_stats (
         address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
       ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind("XapplyZero1111111111111111111111111")
    .bind(50_i64)
    .bind(50_i64)
    .bind(1_i32)
    .bind(1_i32)
    .bind(Some(9_i32))
    .bind(Some(9_i32))
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    apply_address_stats_chunk(
      &mut tx,
      &["XapplyZero1111111111111111111111111".to_owned()],
      &[-50],
      &[-50],
      &[-1],
      &[-1],
      &[None],
      &[None],
    )
    .await?;
    tx.commit().await?;

    let row =
      fetch_optional_address_stats_row(&pool, "XapplyZero1111111111111111111111111").await?;
    assert_eq!(row, None);

    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_stats_updates_tip_disconnect_incrementally_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    for height in 9..=11 {
      seed_block(&pool, height).await?;
    }

    let txid_a9 = synthetic_hash(b'T', 9);
    let txid_a10 = synthetic_hash(b'T', 10);
    let txid_a11 = synthetic_hash(b'T', 11);
    let txid_b11 = synthetic_hash(b'U', 11);

    for (txid, block_height, block_pos) in [
      (&txid_a9, 9_i32, 0_i32),
      (&txid_a10, 10_i32, 0_i32),
      (&txid_a11, 11_i32, 0_i32),
      (&txid_b11, 11_i32, 1_i32),
    ] {
      sqlx::query(
        "INSERT INTO transactions (
           txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
         ) VALUES ($1, $2, $3, 3, 'normal', 0, 200, 1_000, 1, 1, 10)",
      )
      .bind(txid)
      .bind(block_height)
      .bind(block_pos)
      .execute(&pool)
      .await?;
    }

    for (address, txid, block_height, block_pos) in [
      (
        "XreverseA111111111111111111111111111",
        &txid_a9,
        9_i32,
        0_i32,
      ),
      (
        "XreverseA111111111111111111111111111",
        &txid_a10,
        10_i32,
        0_i32,
      ),
      (
        "XreverseA111111111111111111111111111",
        &txid_a11,
        11_i32,
        0_i32,
      ),
      (
        "XreverseB111111111111111111111111111",
        &txid_b11,
        11_i32,
        1_i32,
      ),
    ] {
      sqlx::query(
        "INSERT INTO address_txs (address, txid, block_height, block_pos)
         VALUES ($1, $2, $3, $4)",
      )
      .bind(address)
      .bind(txid)
      .bind(block_height)
      .bind(block_pos)
      .execute(&pool)
      .await?;
    }

    for (address, balance, total_received, tx_count, utxo_count, first_seen, last_seen) in [
      (
        "XreverseA111111111111111111111111111",
        150_i64,
        150_i64,
        3_i32,
        3_i32,
        Some(9_i32),
        Some(11_i32),
      ),
      (
        "XreverseB111111111111111111111111111",
        20_i64,
        20_i64,
        1_i32,
        1_i32,
        Some(11_i32),
        Some(11_i32),
      ),
    ] {
      sqlx::query(
        "INSERT INTO address_stats (
           address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
         ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
      )
      .bind(address)
      .bind(balance)
      .bind(total_received)
      .bind(tx_count)
      .bind(utxo_count)
      .bind(first_seen)
      .bind(last_seen)
      .execute(&pool)
      .await?;
    }

    seed_utxo(
      &pool,
      "XreverseA111111111111111111111111111",
      11,
      txid_a11.clone(),
      0,
      50,
    )
    .await?;
    seed_utxo(
      &pool,
      "XreverseB111111111111111111111111111",
      11,
      txid_b11.clone(),
      0,
      20,
    )
    .await?;

    let mut tx = pool.begin().await?;
    reverse_address_history_at_height(&mut tx, 11).await?;
    reverse_address_stats_at_height(&mut tx, 11).await?;
    tx.commit().await?;

    let surviving = fetch_address_stats_row(&pool, "XreverseA111111111111111111111111111").await?;
    assert_eq!(surviving, (100, 100, 2, 2, Some(9), Some(10)));

    let removed = sqlx::query_scalar::<_, i64>(
      "SELECT COUNT(*)
         FROM address_stats
        WHERE address = $1",
    )
    .bind("XreverseB111111111111111111111111111")
    .fetch_one(&pool)
    .await?;
    assert_eq!(removed, 0);

    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_stats_repairs_stale_rows_before_retry_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    for height in 9..=11 {
      seed_block(&pool, height).await?;
    }

    let txid_a9 = synthetic_hash(b'R', 9);
    let txid_a10 = synthetic_hash(b'R', 10);
    let txid_a11 = synthetic_hash(b'R', 11);

    for (txid, block_height) in [(&txid_a9, 9_i32), (&txid_a10, 10_i32), (&txid_a11, 11_i32)] {
      sqlx::query(
        "INSERT INTO transactions (
           txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
         ) VALUES ($1, $2, 0, 3, 'normal', 0, 200, 1_000, 1, 1, 10)",
      )
      .bind(txid)
      .bind(block_height)
      .execute(&pool)
      .await?;
    }

    for (txid, block_height) in [(&txid_a9, 9_i32), (&txid_a10, 10_i32), (&txid_a11, 11_i32)] {
      sqlx::query(
        "INSERT INTO address_txs (address, txid, block_height, block_pos)
         VALUES ($1, $2, $3, 0)",
      )
      .bind("XreverseRepair11111111111111111111111")
      .bind(txid)
      .bind(block_height)
      .execute(&pool)
      .await?;
    }

    for (txid, block_height, value) in [
      (&txid_a9, 9_i32, 40_i64),
      (&txid_a10, 10_i32, 60_i64),
      (&txid_a11, 11_i32, 50_i64),
    ] {
      seed_utxo(
        &pool,
        "XreverseRepair11111111111111111111111",
        block_height,
        txid.clone(),
        0,
        value,
      )
      .await?;
    }

    sqlx::query(
      "INSERT INTO address_stats (
         address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
       ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind("XreverseRepair11111111111111111111111")
    .bind(999_i64)
    .bind(999_i64)
    .bind(50_i32)
    .bind(50_i32)
    .bind(Some(11_i32))
    .bind(Some(99_i32))
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    reverse_address_history_at_height(&mut tx, 11).await?;
    reverse_address_stats_at_height(&mut tx, 11).await?;
    tx.commit().await?;

    let repaired = fetch_address_stats_row(&pool, "XreverseRepair11111111111111111111111").await?;
    assert_eq!(repaired, (100, 100, 2, 2, Some(9), Some(10)));

    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_stats_self_heal_rollback_leaves_committed_state_unchanged_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    for height in 9..=11 {
      seed_block(&pool, height).await?;
    }

    let txid_a10 = synthetic_hash(b'Q', 10);
    let txid_a11 = synthetic_hash(b'Q', 11);
    for (txid, block_height) in [(&txid_a10, 10_i32), (&txid_a11, 11_i32)] {
      sqlx::query(
        "INSERT INTO transactions (
           txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
         ) VALUES ($1, $2, 0, 3, 'normal', 0, 200, 1_000, 1, 1, 10)",
      )
      .bind(txid)
      .bind(block_height)
      .execute(&pool)
      .await?;
      sqlx::query(
        "INSERT INTO address_txs (address, txid, block_height, block_pos)
         VALUES ($1, $2, $3, 0)",
      )
      .bind("XreverseRollback111111111111111111111")
      .bind(txid)
      .bind(block_height)
      .execute(&pool)
      .await?;
    }

    for (txid, block_height, value) in [(&txid_a10, 10_i32, 60_i64), (&txid_a11, 11_i32, 50_i64)] {
      seed_utxo(
        &pool,
        "XreverseRollback111111111111111111111",
        block_height,
        txid.clone(),
        0,
        value,
      )
      .await?;
    }

    sqlx::query(
      "INSERT INTO address_stats (
         address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
       ) VALUES ($1, $2, $3, $4, $5, $6, $7)",
    )
    .bind("XreverseRollback111111111111111111111")
    .bind(999_i64)
    .bind(999_i64)
    .bind(50_i32)
    .bind(50_i32)
    .bind(Some(11_i32))
    .bind(Some(99_i32))
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    reverse_address_history_at_height(&mut tx, 11).await?;
    reverse_address_stats_at_height(&mut tx, 11).await?;
    tx.rollback().await?;

    let persisted = fetch_address_stats_row(&pool, "XreverseRollback111111111111111111111").await?;
    assert_eq!(persisted, (999, 999, 50, 50, Some(11), Some(99)));
    let history_count = sqlx::query_scalar::<_, i64>(
      "SELECT COUNT(*)
         FROM address_txs
        WHERE address = $1",
    )
    .bind("XreverseRollback111111111111111111111")
    .fetch_one(&pool)
    .await?;
    assert_eq!(history_count, 2);

    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_stats_matches_exact_rebuild_for_touched_addresses_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;
    let addresses = seed_reverse_address_stats_parity_fixture(&pool).await?;

    let mut exact_tx = pool.begin().await?;
    reverse_address_history_at_height(&mut exact_tx, 11).await?;
    refresh_address_stats_for_addresses(&mut exact_tx, &addresses).await?;
    exact_tx.commit().await?;
    let exact_rows = (
      fetch_optional_address_stats_row(&pool, &addresses[0]).await?,
      fetch_optional_address_stats_row(&pool, &addresses[1]).await?,
    );

    crate::sql::bootstrap::reset(&pool).await?;
    let addresses = seed_reverse_address_stats_parity_fixture(&pool).await?;

    let mut fast_tx = pool.begin().await?;
    reverse_address_history_at_height(&mut fast_tx, 11).await?;
    reverse_address_stats_at_height(&mut fast_tx, 11).await?;
    fast_tx.commit().await?;

    let fast_rows = (
      fetch_optional_address_stats_row(&pool, &addresses[0]).await?,
      fetch_optional_address_stats_row(&pool, &addresses[1]).await?,
    );

    assert_eq!(fast_rows, exact_rows);
    Ok(())
  }

  #[tokio::test]
  async fn address_history_txs_returns_address_relative_fields_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let address = "XaddressHistoryRelative111111111111111";
    let incoming_txid = synthetic_hash(b'A', 1);
    let self_txid = synthetic_hash(b'A', 2);
    let outgoing_txid = synthetic_hash(b'A', 3);

    seed_address_history_tx(&pool, address, 1, 0, incoming_txid.clone(), 90, 3, 70, 0).await?;
    seed_address_history_tx(&pool, address, 2, 0, self_txid.clone(), 40, 4, 40, 40).await?;
    seed_address_history_tx(&pool, address, 3, 0, outgoing_txid.clone(), 55, 5, 0, 55).await?;

    let rows = address_history_txs(&pool, address, 10, None).await?;
    assert_eq!(rows.len(), 3);

    assert_eq!(rows[0].txid, outgoing_txid);
    assert_eq!(rows[0].block_time, 3);
    assert_eq!(rows[0].net_value, -55);
    assert_eq!(rows[0].direction, AddressTxDirection::Out);

    assert_eq!(rows[1].txid, self_txid);
    assert_eq!(rows[1].block_time, 2);
    assert_eq!(rows[1].net_value, 0);
    assert_eq!(rows[1].direction, AddressTxDirection::Out);

    assert_eq!(rows[2].txid, incoming_txid);
    assert_eq!(rows[2].block_time, 1);
    assert_eq!(rows[2].net_value, 70);
    assert_eq!(rows[2].direction, AddressTxDirection::In);
    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_history_matches_exact_pre_block_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;
    let addresses = seed_reverse_address_stats_parity_fixture(&pool).await?;

    let exact_before = (
      fetch_address_history_rows(&pool, &addresses[0]).await?,
      fetch_address_history_rows(&pool, &addresses[1]).await?,
    );

    let mut exact_tx = pool.begin().await?;
    sqlx::query(
      "DELETE FROM address_txs
        WHERE block_height = 11",
    )
    .execute(&mut *exact_tx)
    .await?;
    exact_tx.commit().await?;
    let exact_after = (
      fetch_address_history_rows(&pool, &addresses[0]).await?,
      fetch_address_history_rows(&pool, &addresses[1]).await?,
    );

    crate::sql::bootstrap::reset(&pool).await?;
    let addresses = seed_reverse_address_stats_parity_fixture(&pool).await?;
    assert_eq!(
      (
        fetch_address_history_rows(&pool, &addresses[0]).await?,
        fetch_address_history_rows(&pool, &addresses[1]).await?,
      ),
      exact_before
    );

    let mut tx = pool.begin().await?;
    reverse_address_history_at_height(&mut tx, 11).await?;
    tx.commit().await?;

    let reversed_after = (
      fetch_address_history_rows(&pool, &addresses[0]).await?,
      fetch_address_history_rows(&pool, &addresses[1]).await?,
    );

    assert_eq!(reversed_after, exact_after);
    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_stats_counts_one_removed_tx_for_self_touch_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let address = "XreverseSelfTouch11111111111111111111";
    let txid10 = synthetic_hash(b'S', 10);
    let txid11a = synthetic_hash(b'S', 110);
    let txid11b = synthetic_hash(b'S', 111);

    seed_address_history_tx(&pool, address, 10, 0, txid10, 90, 3, 90, 0).await?;
    seed_address_history_tx(&pool, address, 11, 0, txid11a, 40, 2, 40, 40).await?;
    seed_address_history_tx(&pool, address, 11, 1, txid11b, 30, 2, 0, 30).await?;

    let mut seed_tx = pool.begin().await?;
    refresh_address_stats_for_addresses(&mut seed_tx, &[address.to_owned()]).await?;
    seed_tx.commit().await?;

    let before = fetch_address_stats_row(&pool, address).await?;
    assert_eq!(before.2, 3);
    assert_eq!(before.4, Some(10));
    assert_eq!(before.5, Some(11));

    let mut tx = pool.begin().await?;
    reverse_address_history_at_height(&mut tx, 11).await?;
    reverse_address_stats_at_height(&mut tx, 11).await?;
    tx.commit().await?;

    let after = fetch_address_stats_row(&pool, address).await?;
    assert_eq!(
      after.2, 1,
      "self-touch tx should only remove one history count"
    );
    assert_eq!(after.4, Some(10));
    assert_eq!(after.5, Some(10));
    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_stats_recomputes_last_seen_from_remaining_history_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let address = "XreverseLastSeen111111111111111111111";
    let txid9 = synthetic_hash(b'L', 9);
    let txid10a = synthetic_hash(b'L', 100);
    let txid10b = synthetic_hash(b'L', 101);
    let txid11 = synthetic_hash(b'L', 11);

    seed_address_history_tx(&pool, address, 9, 0, txid9, 20, 1, 20, 0).await?;
    seed_address_history_tx(&pool, address, 10, 0, txid10a, 30, 1, 30, 0).await?;
    seed_address_history_tx(&pool, address, 10, 1, txid10b, 40, 1, 40, 0).await?;
    seed_address_history_tx(&pool, address, 11, 0, txid11, 50, 1, 50, 0).await?;

    let mut seed_tx = pool.begin().await?;
    refresh_address_stats_for_addresses(&mut seed_tx, &[address.to_owned()]).await?;
    seed_tx.commit().await?;

    sqlx::query(
      "UPDATE address_stats
          SET last_seen_height = 99
        WHERE address = $1",
    )
    .bind(address)
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    reverse_address_history_at_height(&mut tx, 11).await?;
    reverse_address_stats_at_height(&mut tx, 11).await?;
    tx.commit().await?;

    let after = fetch_address_stats_row(&pool, address).await?;
    assert_eq!(after.2, 3);
    assert_eq!(after.4, Some(9));
    assert_eq!(
      after.5,
      Some(10),
      "last_seen should come from remaining history"
    );
    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_stats_repairs_mixed_corruption_cases_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    for height in 9..=11 {
      seed_block(&pool, height).await?;
    }

    let cases = [
      (
        "XreverseMixedMissing111111111111111111",
        b'M',
        [9_i32, 10, 11],
      ),
      (
        "XreverseMixedFirst1111111111111111111",
        b'F',
        [9_i32, 10, 11],
      ),
      (
        "XreverseMixedLast11111111111111111111",
        b'L',
        [10_i32, 11, 11],
      ),
      (
        "XreverseMixedHealthy111111111111111111",
        b'H',
        [9_i32, 10, 11],
      ),
    ];

    for (address, domain, heights) in &cases {
      for (index, height) in heights.iter().enumerate() {
        let txid = synthetic_hash(*domain, *height * 10 + index as i32);
        sqlx::query(
          "INSERT INTO transactions (
             txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
           ) VALUES ($1, $2, 0, 3, 'normal', 0, 200, 1_000, 1, 1, 10)",
        )
        .bind(&txid)
        .bind(height)
        .execute(&pool)
        .await?;
        sqlx::query(
          "INSERT INTO address_txs (address, txid, block_height, block_pos)
           VALUES ($1, $2, $3, 0)
          ON CONFLICT DO NOTHING",
        )
        .bind(address)
        .bind(&txid)
        .bind(height)
        .execute(&pool)
        .await?;
        seed_utxo(&pool, address, *height, txid, 0, 10).await?;
      }
    }

    let touched = cases
      .iter()
      .map(|(address, _, _)| (*address).to_owned())
      .collect::<Vec<_>>();
    let mut seed_tx = pool.begin().await?;
    refresh_address_stats_for_addresses(&mut seed_tx, &touched).await?;
    seed_tx.commit().await?;

    sqlx::query("DELETE FROM address_stats WHERE address = $1")
      .bind(cases[0].0)
      .execute(&pool)
      .await?;
    sqlx::query(
      "UPDATE address_stats
          SET first_seen_height = 11
        WHERE address = $1",
    )
    .bind(cases[1].0)
    .execute(&pool)
    .await?;
    sqlx::query(
      "UPDATE address_stats
          SET last_seen_height = 99
        WHERE address = $1",
    )
    .bind(cases[2].0)
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    reverse_address_history_at_height(&mut tx, 11).await?;
    reverse_address_stats_at_height(&mut tx, 11).await?;
    tx.commit().await?;

    for address in touched {
      let row = fetch_address_stats_row(&pool, &address).await?;
      assert_eq!(row.2, 2);
      assert_eq!(row.3, 2);
      assert_eq!(row.5, Some(10));
    }

    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_stats_returns_after_one_repair_attempt_when_canonical_rows_are_still_missing(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;
    seed_block(&pool, 11).await?;

    let txid = synthetic_hash(b'W', 11);
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
       ) VALUES ($1, 11, 0, 3, 'normal', 0, 200, 1_000, 1, 1, 10)",
    )
    .bind(&txid)
    .execute(&pool)
    .await?;
    sqlx::query(
      "INSERT INTO tx_inputs (
         txid, vin_index, prev_txid, prev_vout, script_sig, sequence, prev_address, prev_value
       ) VALUES ($1, 0, NULL, NULL, $2, 0, $3, $4)",
    )
    .bind(&txid)
    .bind(vec![0x51_u8])
    .bind("XreverseNoCanonical1111111111111111")
    .bind(25_i64)
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    let err = reverse_address_stats_at_height(&mut tx, 11)
      .await
      .expect_err("reverse should fail after one repair attempt when canonical rows stay missing");
    tx.rollback().await?;

    assert!(matches!(
      err,
      DaemonError::Index(message) if message.contains("missing address_stats row during reverse")
    ));
    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_balances_restores_summary_from_undo_meta_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    seed_block(&pool, 11).await?;
    let mut tx = pool.begin().await?;
    crate::sql::undo::upsert_block_undo_meta(&mut tx, 11, &[11_u8; 32], &[10_u8; 32], 0).await?;
    tx.commit().await?;

    sqlx::query(
      "INSERT INTO address_balances_summary (singleton, total, included_supply)
       VALUES (TRUE, 999, 999)
       ON CONFLICT (singleton) DO UPDATE SET
         total = EXCLUDED.total,
         included_supply = EXCLUDED.included_supply",
    )
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    reverse_address_balances_at_height(&mut tx, 11).await?;
    tx.commit().await?;

    let summary = balances_summary(&pool).await?;
    assert_eq!(summary.total, 0);
    assert_eq!(summary.included_supply, 0);
    Ok(())
  }

  #[tokio::test]
  async fn reverse_address_stats_rejects_missing_touched_undo_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    seed_block(&pool, 11).await?;
    let txid = synthetic_hash(b'C', 11);
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
       ) VALUES ($1, 11, 0, 3, 'normal', 0, 200, 1_000, 0, 1, 10)",
    )
    .bind(&txid)
    .execute(&pool)
    .await?;
    sqlx::query(
      "INSERT INTO address_txs (address, txid, block_height, block_pos)
       VALUES ($1, $2, 11, 0)",
    )
    .bind("XreverseStatsUndo111111111111111111111")
    .bind(&txid)
    .execute(&pool)
    .await?;
    seed_utxo(
      &pool,
      "XreverseStatsUndo111111111111111111111",
      11,
      txid,
      0,
      30,
    )
    .await?;

    let mut tx = pool.begin().await?;
    let err = reverse_address_stats_at_height(&mut tx, 11)
      .await
      .expect_err("missing touched stats undo rows should fail");
    tx.rollback().await?;

    assert!(matches!(
      err,
      DaemonError::Index(message)
        if message.contains("block_undo_address_stats mismatch at 11: expected 1, found 0")
    ));
    Ok(())
  }

  #[tokio::test]
  async fn mark_outputs_spent_chunk_matches_prevout_height_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    seed_block(&pool, 5).await?;
    seed_block(&pool, 6).await?;
    seed_transaction(&pool, 5, &synthetic_hash(b'X', 5)).await?;
    seed_transaction(&pool, 6, &synthetic_hash(b'X', 6)).await?;

    let prev_txid = synthetic_hash(b'P', 5);
    let spending_txid = synthetic_hash(b'S', 6);
    seed_transaction(&pool, 5, &prev_txid).await?;
    seed_transaction(&pool, 6, &spending_txid).await?;

    sqlx::query(
      "INSERT INTO tx_outputs (
         txid, vout_index, block_height, value, script_pubkey, address, address_type, spent_txid, spent_vin, spent_height
       ) VALUES ($1, 0, 5, 50, $2, $3, 'p2pkh', NULL, NULL, NULL)",
    )
    .bind(&prev_txid)
    .bind(vec![0x51_u8])
    .bind("XprevoutHeight11111111111111111111111")
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    prepare_prevout_stage(&mut tx).await?;
    stage_prevout_snapshot_rows(
      &mut tx,
      std::slice::from_ref(&prev_txid),
      &[0],
      &[9],
      std::slice::from_ref(&spending_txid),
      &[0],
      &[6],
      &[None],
      &[None],
      &[None],
      &[Some("XprevoutHeight11111111111111111111111".to_owned())],
      &[50],
    )
    .await?;
    mark_outputs_spent_chunk(&mut tx, 0, 1).await?;

    let wrong_height_update = sqlx::query_scalar::<_, Option<Vec<u8>>>(
      "SELECT spent_txid FROM tx_outputs WHERE block_height = 5 AND txid = $1 AND vout_index = 0",
    )
    .bind(&prev_txid)
    .fetch_one(&mut *tx)
    .await?;
    assert!(
      wrong_height_update.is_none(),
      "wrong prev_height should not update output row",
    );

    sqlx::query("TRUNCATE tmp_prevout_spend_delta RESTART IDENTITY")
      .execute(&mut *tx)
      .await?;
    stage_prevout_snapshot_rows(
      &mut tx,
      std::slice::from_ref(&prev_txid),
      &[0],
      &[5],
      std::slice::from_ref(&spending_txid),
      &[0],
      &[6],
      &[None],
      &[None],
      &[None],
      &[Some("XprevoutHeight11111111111111111111111".to_owned())],
      &[50],
    )
    .await?;
    mark_outputs_spent_chunk(&mut tx, 0, 1).await?;

    let correct_height_update = sqlx::query_scalar::<_, Option<Vec<u8>>>(
      "SELECT spent_txid FROM tx_outputs WHERE block_height = 5 AND txid = $1 AND vout_index = 0",
    )
    .bind(&prev_txid)
    .fetch_one(&mut *tx)
    .await?;
    assert_eq!(correct_height_update, Some(spending_txid));

    tx.rollback().await?;
    Ok(())
  }
}

/// Stage fully materialized prevout spend rows for later spent marking.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
#[expect(clippy::too_many_arguments)]
pub async fn stage_prevout_snapshot_rows(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  prev_txids: &[Vec<u8>],
  prev_vouts: &[i32],
  prev_heights: &[i32],
  spending_txids: &[Vec<u8>],
  spending_vins: &[i32],
  spending_heights: &[i32],
  prev_spent_txids: &[Option<Vec<u8>>],
  prev_spent_vins: &[Option<i32>],
  prev_spent_heights: &[Option<i32>],
  prev_addresses: &[Option<String>],
  prev_values: &[i64],
) -> Result<(), DaemonError> {
  let n = prev_txids.len();
  if n == 0 {
    return Ok(());
  }

  for start in (0..n).step_by(PREVOUT_STAGE_CHUNK_SIZE) {
    let end = (start + PREVOUT_STAGE_CHUNK_SIZE).min(n);
    sqlx::query(
      "INSERT INTO tmp_prevout_spend_delta (
         prev_txid, prev_vout, prev_height, spending_txid, spending_vin, spending_height,
         prev_spent_txid, prev_spent_vin, prev_spent_height, prev_address, prev_value
       )
       SELECT * FROM unnest(
         $1::bytea[], $2::int[], $3::int[], $4::bytea[], $5::int[], $6::int[],
         $7::bytea[], $8::int[], $9::int[], $10::text[], $11::bigint[]
       )",
    )
    .bind(&prev_txids[start..end])
    .bind(&prev_vouts[start..end])
    .bind(&prev_heights[start..end])
    .bind(&spending_txids[start..end])
    .bind(&spending_vins[start..end])
    .bind(&spending_heights[start..end])
    .bind(&prev_spent_txids[start..end])
    .bind(&prev_spent_vins[start..end])
    .bind(&prev_spent_heights[start..end])
    .bind(&prev_addresses[start..end])
    .bind(&prev_values[start..end])
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("stage prevout snapshot rows", e))?;
  }

  Ok(())
}

/// Count the prevout rows currently staged for chunked processing.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn staged_prevout_count(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<i64, DaemonError> {
  sqlx::query_scalar("SELECT COUNT(*) FROM tmp_prevout_spend_delta")
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| pg_err("count staged prevouts", e))
}

/// Snapshot spent-output history rows using a precomputed prevout snapshot.
#[expect(clippy::too_many_arguments)]
pub async fn insert_spent_output_history(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
  prev_txids: &[Vec<u8>],
  prev_vouts: &[i32],
  prev_spent_txids: &[Option<Vec<u8>>],
  prev_spent_vins: &[Option<i32>],
  prev_spent_heights: &[Option<i32>],
  spending_txids: &[Vec<u8>],
  spending_vins: &[i32],
) -> Result<(), DaemonError> {
  let n = prev_txids.len();
  if n == 0 {
    return Ok(());
  }

  crate::sql::undo::insert_block_undo_outputs(
    tx,
    height,
    prev_txids,
    prev_vouts,
    prev_spent_txids,
    prev_spent_vins,
    prev_spent_heights,
  )
  .await?;

  for start in (0..n).step_by(SPENT_HISTORY_WRITE_CHUNK_SIZE) {
    let end = (start + SPENT_HISTORY_WRITE_CHUNK_SIZE).min(n);
    sqlx::query(
      "INSERT INTO spent_output_history (
         height, spending_txid, spending_vin, prev_txid, prev_vout,
         prev_spent_txid, prev_spent_vin, prev_spent_height
       )
       SELECT * FROM unnest(
         $1::int[], $2::bytea[], $3::int[], $4::bytea[], $5::int[],
         $6::bytea[], $7::int[], $8::int[]
       )
       ON CONFLICT (height, spending_txid, spending_vin) DO NOTHING",
    )
    .bind(vec![height as i32; end - start])
    .bind(&spending_txids[start..end])
    .bind(&spending_vins[start..end])
    .bind(&prev_txids[start..end])
    .bind(&prev_vouts[start..end])
    .bind(&prev_spent_txids[start..end])
    .bind(&prev_spent_vins[start..end])
    .bind(&prev_spent_heights[start..end])
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("insert spent output history", e))?;
  }

  Ok(())
}

/// Mark outputs as spent using precomputed prevout keys and spend metadata.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn mark_outputs_spent_chunk(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  start_stage_id: i64,
  end_stage_id: i64,
) -> Result<(), DaemonError> {
  sqlx::query(
    "UPDATE tx_outputs AS o
        SET spent_txid = s.spending_txid,
            spent_vin = s.spending_vin,
            spent_height = s.spending_height
       FROM (
         SELECT prev_txid, prev_vout, prev_height, spending_txid, spending_vin, spending_height
           FROM tmp_prevout_spend_delta
          WHERE stage_id > $1 AND stage_id <= $2
       ) AS s
      WHERE o.block_height = s.prev_height
        AND o.txid = s.prev_txid
        AND o.vout_index = s.prev_vout",
  )
  .bind(start_stage_id)
  .bind(end_stage_id)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("mark outputs spent chunk", e))?;
  Ok(())
}

/// Backfill prev_address/prev_value on tx_inputs.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn batch_update_tx_inputs(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  txids: &[Vec<u8>],
  vins: &[i32],
  addrs: &[Option<String>],
  values: &[i64],
) -> Result<(), DaemonError> {
  sqlx::query(
    "UPDATE tx_inputs AS i
       SET prev_address = v.addr, prev_value = v.val
      FROM unnest($1::bytea[], $2::int[], $3::text[], $4::bigint[])
        AS v(txid, vin, addr, val)
     WHERE i.txid = v.txid AND i.vin_index = v.vin",
  )
  .bind(txids)
  .bind(vins)
  .bind(addrs)
  .bind(values)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("batch update tx_inputs", e))?;
  Ok(())
}

/// Set transaction fees.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn batch_update_fees(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  block_height: i32,
  txids: &[Vec<u8>],
  fees: &[i64],
) -> Result<(), DaemonError> {
  sqlx::query(
    "UPDATE transactions AS t
       SET fee = v.fee
      FROM unnest($1::bytea[], $2::bigint[]) AS v(txid, fee)
     WHERE t.block_height = $3
       AND t.txid = v.txid",
  )
  .bind(txids)
  .bind(fees)
  .bind(block_height)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("batch update fees", e))?;
  Ok(())
}

/// Upsert address balances rows.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn insert_address_history(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addrs: &[String],
  txids: &[Vec<u8>],
  block_heights: &[i32],
  block_positions: &[i32],
) -> Result<(), DaemonError> {
  let n = addrs.len();
  if n == 0 {
    return Ok(());
  }

  for start in (0..n).step_by(ADDRESS_HISTORY_WRITE_CHUNK_SIZE) {
    let end = (start + ADDRESS_HISTORY_WRITE_CHUNK_SIZE).min(n);
    sqlx::query(
      "INSERT INTO address_txs (address, txid, block_height, block_pos)
       SELECT address, txid, block_height, block_pos
         FROM unnest($1::text[], $2::bytea[], $3::int[], $4::int[])
           AS v(address, txid, block_height, block_pos)
       ON CONFLICT (address, block_height, txid) DO NOTHING",
    )
    .bind(&addrs[start..end])
    .bind(&txids[start..end])
    .bind(&block_heights[start..end])
    .bind(&block_positions[start..end])
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("insert address history", e))?;
  }

  Ok(())
}

/// Upsert address balances rows.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn stage_address_balances_delta(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addrs: &[String],
  balances: &[i64],
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO tmp_address_balances_delta (address, balance)
     SELECT addr, bal
       FROM unnest($1::text[], $2::bigint[]) AS v(addr, bal)",
  )
  .bind(addrs)
  .bind(balances)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("stage address_balances delta", e))?;
  Ok(())
}

/// Apply one address balances delta chunk directly from precomputed arrays.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn apply_address_balances_chunk(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addrs: &[String],
  balances: &[i64],
) -> Result<(), DaemonError> {
  if addrs.is_empty() {
    return Ok(());
  }

  sqlx::query(
    "WITH delta AS (
       SELECT address, balance
         FROM unnest($1::text[], $2::bigint[]) AS d(address, balance)
     ),
     updated AS (
       UPDATE address_balances AS s
          SET balance = s.balance + d.balance
         FROM delta AS d
        WHERE s.address = d.address
      RETURNING d.address
     )
     INSERT INTO address_balances (address, balance)
     SELECT d.address, d.balance
       FROM delta AS d
       LEFT JOIN updated AS u ON u.address = d.address
      WHERE u.address IS NULL",
  )
  .bind(addrs)
  .bind(balances)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("apply address_balances chunk", e))?;
  Ok(())
}

/// Apply one address balances summary delta chunk directly from precomputed arrays.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn apply_address_balances_summary_chunk(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addrs: &[String],
  balances: &[i64],
) -> Result<(), DaemonError> {
  if addrs.is_empty() {
    return Ok(());
  }

  sqlx::query("SAVEPOINT address_balances_summary_delta")
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("savepoint address balances summary delta", e))?;
  match apply_address_balances_summary_chunk_once(tx, addrs, balances).await {
    Ok(()) => {
      sqlx::query("RELEASE SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("release savepoint address balances summary delta", e))?;
      Ok(())
    }
    Err(DaemonError::Pg { message, .. }) if is_balances_summary_constraint_error(&message) => {
      sqlx::query("ROLLBACK TO SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("rollback savepoint address balances summary delta", e))?;
      sqlx::query("RELEASE SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("release savepoint address balances summary delta", e))?;
      tracing::warn!(
        rows = addrs.len(),
        "address_balances_summary constraint failed during direct apply; refreshing singleton to post-chunk state"
      );
      refresh_address_balances_summary_for_addresses(tx, addrs, balances).await
    }
    Err(error) => {
      let _ = sqlx::query("ROLLBACK TO SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await;
      let _ = sqlx::query("RELEASE SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await;
      Err(error)
    }
  }
}

/// Apply one balances-summary delta chunk using current balances from
/// `address_stats`.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn apply_address_stats_summary_chunk(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addrs: &[String],
  balances: &[i64],
) -> Result<(), DaemonError> {
  if addrs.is_empty() {
    return Ok(());
  }

  sqlx::query("SAVEPOINT address_balances_summary_delta")
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("savepoint address balances summary delta", e))?;
  match apply_address_stats_summary_chunk_once(tx, addrs, balances).await {
    Ok(()) => {
      sqlx::query("RELEASE SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("release savepoint address balances summary delta", e))?;
      Ok(())
    }
    Err(DaemonError::Pg { message, .. }) if is_balances_summary_constraint_error(&message) => {
      sqlx::query("ROLLBACK TO SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("rollback savepoint address balances summary delta", e))?;
      sqlx::query("RELEASE SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("release savepoint address balances summary delta", e))?;
      tracing::warn!(
        rows = addrs.len(),
        "address_balances_summary constraint failed during address_stats apply; refreshing singleton to post-chunk state"
      );
      refresh_address_balances_summary_for_stats_addresses(tx, addrs, balances).await
    }
    Err(error) => {
      let _ = sqlx::query("ROLLBACK TO SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await;
      let _ = sqlx::query("RELEASE SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await;
      Err(error)
    }
  }
}

/// Apply one block-scoped address summary delta chunk to the final summary
/// table.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn apply_address_stats_chunk(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addrs: &[String],
  balances: &[i64],
  total_received: &[i64],
  tx_counts: &[i32],
  utxo_counts: &[i32],
  first_seen_heights: &[Option<i32>],
  last_seen_heights: &[Option<i32>],
) -> Result<(), DaemonError> {
  if addrs.is_empty() {
    return Ok(());
  }

  sqlx::query(
    "WITH delta AS (
       SELECT d.address, d.balance, d.total_received, d.tx_count, d.utxo_count,
              d.first_seen_height, d.last_seen_height
         FROM unnest($1::text[], $2::bigint[], $3::bigint[], $4::int[], $5::int[], $6::int[], $7::int[])
           AS d(address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height)
     ),
     updated AS (
       UPDATE address_stats AS s
          SET balance = s.balance + d.balance,
              total_received = s.total_received + d.total_received,
              tx_count = s.tx_count + d.tx_count,
              utxo_count = s.utxo_count + d.utxo_count,
              first_seen_height = CASE
                WHEN s.first_seen_height IS NULL THEN d.first_seen_height
                WHEN d.first_seen_height IS NULL THEN s.first_seen_height
                ELSE LEAST(s.first_seen_height, d.first_seen_height)
              END,
              last_seen_height = CASE
                WHEN s.last_seen_height IS NULL THEN d.last_seen_height
                WHEN d.last_seen_height IS NULL THEN s.last_seen_height
                ELSE GREATEST(s.last_seen_height, d.last_seen_height)
              END
         FROM delta AS d
        WHERE s.address = d.address
      RETURNING d.address
     )
     INSERT INTO address_stats (
       address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
     )
     SELECT d.address, d.balance, d.total_received, d.tx_count, d.utxo_count,
            d.first_seen_height, d.last_seen_height
       FROM delta AS d
       LEFT JOIN updated AS u ON u.address = d.address
      WHERE u.address IS NULL",
  )
  .bind(addrs)
  .bind(balances)
  .bind(total_received)
  .bind(tx_counts)
  .bind(utxo_counts)
  .bind(first_seen_heights)
  .bind(last_seen_heights)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("upsert address_stats chunk", e))?;

  sqlx::query(
    "DELETE FROM address_stats
      WHERE address = ANY($1::text[])
        AND balance = 0
        AND total_received = 0
        AND tx_count = 0
        AND utxo_count = 0",
  )
  .bind(addrs)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("delete zeroed address_stats after apply", e))?;

  Ok(())
}

/// Prepare the temporary address balances delta staging table.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn prepare_address_balances_staging(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), DaemonError> {
  if !temp_table_exists(tx, "tmp_address_balances_delta").await? {
    sqlx::query(
      "CREATE TEMP TABLE tmp_address_balances_delta (
         stage_id bigint GENERATED ALWAYS AS IDENTITY,
         address text PRIMARY KEY,
         balance bigint NOT NULL
       ) ON COMMIT PRESERVE ROWS",
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("create address_balances staging", e))?;
  }

  sqlx::query("TRUNCATE tmp_address_balances_delta RESTART IDENTITY")
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("truncate address_balances staging", e))?;
  Ok(())
}

/// Count the staged address balances delta rows awaiting application.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn staged_address_balances_count(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<i64, DaemonError> {
  sqlx::query_scalar("SELECT COUNT(*) FROM tmp_address_balances_delta")
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| pg_err("count staged address_balances", e))
}

/// Apply one staged address balances delta chunk to the final snapshot table.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn apply_staged_address_balances_chunk(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  start_stage_id: i64,
  end_stage_id: i64,
) -> Result<(), DaemonError> {
  sqlx::query(
    "WITH delta AS (
       SELECT address, SUM(balance)::bigint AS balance
         FROM tmp_address_balances_delta
        WHERE stage_id > $1 AND stage_id <= $2
        GROUP BY address
     ),
     updated AS (
       UPDATE address_balances AS s
          SET balance = s.balance + d.balance
         FROM delta AS d
        WHERE s.address = d.address
      RETURNING d.address
     )
     INSERT INTO address_balances (address, balance)
     SELECT d.address, d.balance
       FROM delta AS d
       LEFT JOIN updated AS u ON u.address = d.address
      WHERE u.address IS NULL
      ORDER BY d.address",
  )
  .bind(start_stage_id)
  .bind(end_stage_id)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("apply address_balances staging chunk", e))?;
  Ok(())
}

/// Apply one staged address balances summary delta chunk to the singleton summary.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn apply_staged_address_balances_summary_chunk(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  start_stage_id: i64,
  end_stage_id: i64,
) -> Result<(), DaemonError> {
  sqlx::query("SAVEPOINT address_balances_summary_delta")
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("savepoint address balances summary delta", e))?;
  match apply_staged_address_balances_summary_chunk_once(tx, start_stage_id, end_stage_id).await {
    Ok(()) => {
      sqlx::query("RELEASE SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("release savepoint address balances summary delta", e))?;
      Ok(())
    }
    Err(DaemonError::Pg { message, .. }) if is_balances_summary_constraint_error(&message) => {
      sqlx::query("ROLLBACK TO SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("rollback savepoint address balances summary delta", e))?;
      sqlx::query("RELEASE SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("release savepoint address balances summary delta", e))?;
      tracing::warn!(
        start_stage_id,
        end_stage_id,
        "address_balances_summary constraint failed during incremental apply; refreshing singleton to post-chunk state"
      );
      match load_address_balances_summary_delta_diag(tx, start_stage_id, end_stage_id).await {
        Ok(diag) => {
          tracing::warn!(
            start_stage_id,
            end_stage_id,
            staged_rows = diag.staged_rows,
            distinct_addresses = diag.distinct_addresses,
            current_total = diag.current_total,
            current_included_supply = diag.current_included_supply,
            total_delta = diag.total_delta,
            included_supply_delta = diag.included_supply_delta,
            projected_total = diag.current_total + diag.total_delta,
            projected_included_supply = diag.current_included_supply + diag.included_supply_delta,
            exact_total = diag.exact_total,
            exact_included_supply = diag.exact_included_supply,
            "address_balances_summary delta mismatch diagnosed"
          );
        }
        Err(diag_error) => {
          tracing::warn!(
            start_stage_id,
            end_stage_id,
            error = %diag_error,
            "failed to diagnose address_balances_summary delta mismatch"
          );
        }
      }
      refresh_address_balances_summary_for_chunk(tx, start_stage_id, end_stage_id).await
    }
    Err(error) => {
      let _ = sqlx::query("ROLLBACK TO SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await;
      let _ = sqlx::query("RELEASE SAVEPOINT address_balances_summary_delta")
        .execute(&mut **tx)
        .await;
      Err(error)
    }
  }
}

async fn load_address_balances_summary_delta_diag(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  start_stage_id: i64,
  end_stage_id: i64,
) -> Result<BalancesSummaryDeltaDiag, DaemonError> {
  let row = sqlx::query_as::<_, (i64, i64, i64, i64, i64, i64, i64, i64)>(
    "WITH delta AS (
       SELECT d.address,
              SUM(d.balance)::bigint AS balance
         FROM tmp_address_balances_delta AS d
        WHERE d.stage_id > $1 AND d.stage_id <= $2
        GROUP BY d.address
     ),
     current_summary AS (
       SELECT
         COALESCE((SELECT total FROM address_balances_summary LIMIT 1), 0)::bigint AS total,
         COALESCE(
           (SELECT included_supply FROM address_balances_summary LIMIT 1),
           0
         )::bigint AS included_supply
     ),
     summary_delta AS (
       SELECT
         COALESCE(SUM(
           (CASE WHEN state.new_balance > 0 THEN state.new_balance ELSE 0 END) -
           (CASE WHEN state.old_balance > 0 THEN state.old_balance ELSE 0 END)
         ), 0)::bigint AS included_supply_delta,
         COALESCE(SUM(
           CASE
             WHEN state.new_balance > 0 AND state.old_balance <= 0 THEN 1
             WHEN state.new_balance <= 0 AND state.old_balance > 0 THEN -1
             ELSE 0
           END
         ), 0)::bigint AS total_delta
       FROM (
         SELECT d.address,
                COALESCE(s.balance, 0) AS old_balance,
                COALESCE(s.balance, 0) + d.balance AS new_balance
           FROM delta AS d
           LEFT JOIN address_balances AS s ON s.address = d.address
       ) AS state
     ),
     untouched AS (
       SELECT COUNT(*)::bigint AS total,
              COALESCE(SUM(balance), 0)::bigint AS included_supply
         FROM address_balances AS b
        WHERE b.balance > 0
          AND NOT EXISTS (
                SELECT 1
                  FROM delta AS d
                 WHERE d.address = b.address
              )
     ),
     touched_positive AS (
       SELECT COUNT(*) FILTER (WHERE new_balance > 0)::bigint AS total,
              COALESCE(SUM(
                CASE
                  WHEN new_balance > 0 THEN new_balance
                  ELSE 0
                END
              ), 0)::bigint AS included_supply
         FROM (
           SELECT d.address,
                  COALESCE(s.balance, 0) + d.balance AS new_balance
             FROM delta AS d
             LEFT JOIN address_balances AS s ON s.address = d.address
         ) AS touched
     ),
     staged AS (
       SELECT COUNT(*)::bigint AS staged_rows
         FROM tmp_address_balances_delta AS d
        WHERE d.stage_id > $1 AND d.stage_id <= $2
     )
     SELECT current_summary.total,
            current_summary.included_supply,
            summary_delta.total_delta,
            summary_delta.included_supply_delta,
            untouched.total + touched_positive.total AS exact_total,
            untouched.included_supply + touched_positive.included_supply AS exact_included_supply,
            staged.staged_rows,
            (SELECT COUNT(*)::bigint FROM delta) AS distinct_addresses
       FROM current_summary, summary_delta, untouched, touched_positive, staged",
  )
  .bind(start_stage_id)
  .bind(end_stage_id)
  .fetch_one(&mut **tx)
  .await
  .map_err(|e| pg_err("diagnose address_balances summary delta", e))?;

  Ok(BalancesSummaryDeltaDiag {
    current_total: row.0,
    current_included_supply: row.1,
    total_delta: row.2,
    included_supply_delta: row.3,
    exact_total: row.4,
    exact_included_supply: row.5,
    staged_rows: row.6,
    distinct_addresses: row.7,
  })
}

async fn refresh_address_balances_summary_for_chunk(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  start_stage_id: i64,
  end_stage_id: i64,
) -> Result<(), DaemonError> {
  sqlx::query(
    "WITH touched AS (
       SELECT d.address,
              COALESCE(s.balance, 0) + SUM(d.balance)::bigint AS new_balance
         FROM tmp_address_balances_delta AS d
         LEFT JOIN address_balances AS s ON s.address = d.address
        WHERE d.stage_id > $1 AND d.stage_id <= $2
        GROUP BY d.address, s.balance
     ),
     untouched AS (
       SELECT COUNT(*)::bigint AS total,
              COALESCE(SUM(balance), 0)::bigint AS included_supply
         FROM address_balances AS b
        WHERE b.balance > 0
          AND NOT EXISTS (
                SELECT 1
                  FROM touched AS t
                 WHERE t.address = b.address
              )
     ),
     touched_positive AS (
       SELECT COUNT(*) FILTER (WHERE new_balance > 0)::bigint AS total,
              COALESCE(SUM(
                CASE
                  WHEN new_balance > 0 THEN new_balance
                  ELSE 0
                END
              ), 0)::bigint AS included_supply
         FROM touched
     )
     INSERT INTO address_balances_summary (singleton, total, included_supply)
     SELECT TRUE,
            untouched.total + touched_positive.total,
            untouched.included_supply + touched_positive.included_supply
       FROM untouched, touched_positive
     ON CONFLICT (singleton) DO UPDATE SET
       total = EXCLUDED.total,
       included_supply = EXCLUDED.included_supply",
  )
  .bind(start_stage_id)
  .bind(end_stage_id)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("refresh address_balances summary for staged chunk", e))?;
  Ok(())
}

async fn refresh_address_balances_summary_for_addresses(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addrs: &[String],
  balances: &[i64],
) -> Result<(), DaemonError> {
  sqlx::query(
    "WITH delta AS (
       SELECT address, balance
         FROM unnest($1::text[], $2::bigint[]) AS d(address, balance)
     ),
     touched AS (
       SELECT d.address,
              COALESCE(s.balance, 0) + d.balance AS new_balance
         FROM delta AS d
         LEFT JOIN address_balances AS s ON s.address = d.address
     ),
     untouched AS (
       SELECT COUNT(*)::bigint AS total,
              COALESCE(SUM(balance), 0)::bigint AS included_supply
         FROM address_balances AS b
        WHERE b.balance > 0
          AND NOT EXISTS (
                SELECT 1
                  FROM touched AS t
                 WHERE t.address = b.address
              )
     ),
     touched_positive AS (
       SELECT COUNT(*) FILTER (WHERE new_balance > 0)::bigint AS total,
              COALESCE(SUM(
                CASE
                  WHEN new_balance > 0 THEN new_balance
                  ELSE 0
                END
              ), 0)::bigint AS included_supply
         FROM touched
     )
     INSERT INTO address_balances_summary (singleton, total, included_supply)
     SELECT TRUE,
            untouched.total + touched_positive.total,
            untouched.included_supply + touched_positive.included_supply
       FROM untouched, touched_positive
     ON CONFLICT (singleton) DO UPDATE SET
       total = EXCLUDED.total,
       included_supply = EXCLUDED.included_supply",
  )
  .bind(addrs)
  .bind(balances)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("refresh address_balances summary for direct chunk", e))?;
  Ok(())
}

async fn refresh_address_balances_summary_for_stats_addresses(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addrs: &[String],
  balances: &[i64],
) -> Result<(), DaemonError> {
  sqlx::query(
    "WITH delta AS (
       SELECT address, balance
         FROM unnest($1::text[], $2::bigint[]) AS d(address, balance)
     ),
     touched AS (
       SELECT d.address,
              COALESCE(s.balance, 0) + d.balance AS new_balance
         FROM delta AS d
         LEFT JOIN address_stats AS s ON s.address = d.address
     ),
     untouched AS (
       SELECT COUNT(*)::bigint AS total,
              COALESCE(SUM(balance), 0)::bigint AS included_supply
         FROM address_stats AS s
        WHERE s.balance > 0
          AND NOT EXISTS (
                SELECT 1
                  FROM touched AS t
                 WHERE t.address = s.address
              )
     ),
     touched_positive AS (
       SELECT COUNT(*) FILTER (WHERE new_balance > 0)::bigint AS total,
              COALESCE(SUM(
                CASE
                  WHEN new_balance > 0 THEN new_balance
                  ELSE 0
                END
              ), 0)::bigint AS included_supply
         FROM touched
     )
     INSERT INTO address_balances_summary (singleton, total, included_supply)
     SELECT TRUE,
            untouched.total + touched_positive.total,
            untouched.included_supply + touched_positive.included_supply
       FROM untouched, touched_positive
     ON CONFLICT (singleton) DO UPDATE SET
       total = EXCLUDED.total,
       included_supply = EXCLUDED.included_supply",
  )
  .bind(addrs)
  .bind(balances)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("refresh address_balances summary for address_stats chunk", e))?;
  Ok(())
}

async fn apply_staged_address_balances_summary_chunk_once(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  start_stage_id: i64,
  end_stage_id: i64,
) -> Result<(), DaemonError> {
  sqlx::query(
    "WITH delta AS (
       SELECT d.address,
              SUM(d.balance)::bigint AS balance
         FROM tmp_address_balances_delta AS d
        WHERE d.stage_id > $1 AND d.stage_id <= $2
        GROUP BY d.address
     ),
     summary_delta AS (
       SELECT
         COALESCE(SUM(
           (CASE WHEN state.new_balance > 0 THEN state.new_balance ELSE 0 END) -
           (CASE WHEN state.old_balance > 0 THEN state.old_balance ELSE 0 END)
         ), 0)::bigint AS included_supply_delta,
         COALESCE(SUM(
           CASE
             WHEN state.new_balance > 0 AND state.old_balance <= 0 THEN 1
             WHEN state.new_balance <= 0 AND state.old_balance > 0 THEN -1
             ELSE 0
           END
         ), 0)::bigint AS total_delta
       FROM (
         SELECT d.address,
                COALESCE(s.balance, 0) AS old_balance,
                COALESCE(s.balance, 0) + d.balance AS new_balance
           FROM delta AS d
           LEFT JOIN address_balances AS s ON s.address = d.address
       ) AS state
     ),
     updated AS (
       UPDATE address_balances_summary AS s
          SET total = s.total + d.total_delta,
              included_supply = s.included_supply + d.included_supply_delta
         FROM summary_delta AS d
        WHERE s.singleton
      RETURNING s.singleton
     )
     INSERT INTO address_balances_summary (singleton, total, included_supply)
     SELECT TRUE, d.total_delta, d.included_supply_delta
       FROM summary_delta AS d
       LEFT JOIN updated AS u ON TRUE
      WHERE u.singleton IS NULL",
  )
  .bind(start_stage_id)
  .bind(end_stage_id)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("apply address_balances summary staging chunk", e))?;
  Ok(())
}

async fn apply_address_balances_summary_chunk_once(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addrs: &[String],
  balances: &[i64],
) -> Result<(), DaemonError> {
  sqlx::query(
    "WITH delta AS (
       SELECT address, balance
         FROM unnest($1::text[], $2::bigint[]) AS d(address, balance)
     ),
     summary_delta AS (
       SELECT
         COALESCE(SUM(
           (CASE WHEN state.new_balance > 0 THEN state.new_balance ELSE 0 END) -
           (CASE WHEN state.old_balance > 0 THEN state.old_balance ELSE 0 END)
         ), 0)::bigint AS included_supply_delta,
         COALESCE(SUM(
           CASE
             WHEN state.new_balance > 0 AND state.old_balance <= 0 THEN 1
             WHEN state.new_balance <= 0 AND state.old_balance > 0 THEN -1
             ELSE 0
           END
         ), 0)::bigint AS total_delta
       FROM (
         SELECT d.address,
                COALESCE(s.balance, 0) AS old_balance,
                COALESCE(s.balance, 0) + d.balance AS new_balance
           FROM delta AS d
           LEFT JOIN address_balances AS s ON s.address = d.address
       ) AS state
     ),
     updated AS (
       UPDATE address_balances_summary AS s
          SET total = s.total + d.total_delta,
              included_supply = s.included_supply + d.included_supply_delta
         FROM summary_delta AS d
        WHERE s.singleton
      RETURNING s.singleton
     )
     INSERT INTO address_balances_summary (singleton, total, included_supply)
     SELECT TRUE, d.total_delta, d.included_supply_delta
       FROM summary_delta AS d
       LEFT JOIN updated AS u ON TRUE
      WHERE u.singleton IS NULL",
  )
  .bind(addrs)
  .bind(balances)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("apply address_balances summary chunk", e))?;
  Ok(())
}

async fn apply_address_stats_summary_chunk_once(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addrs: &[String],
  balances: &[i64],
) -> Result<(), DaemonError> {
  sqlx::query(
    "WITH delta AS (
       SELECT address, balance
         FROM unnest($1::text[], $2::bigint[]) AS d(address, balance)
     ),
     summary_delta AS (
       SELECT
         COALESCE(SUM(
           (CASE WHEN state.new_balance > 0 THEN state.new_balance ELSE 0 END) -
           (CASE WHEN state.old_balance > 0 THEN state.old_balance ELSE 0 END)
         ), 0)::bigint AS included_supply_delta,
         COALESCE(SUM(
           CASE
             WHEN state.new_balance > 0 AND state.old_balance <= 0 THEN 1
             WHEN state.new_balance <= 0 AND state.old_balance > 0 THEN -1
             ELSE 0
           END
         ), 0)::bigint AS total_delta
       FROM (
         SELECT d.address,
                COALESCE(s.balance, 0) AS old_balance,
                COALESCE(s.balance, 0) + d.balance AS new_balance
           FROM delta AS d
           LEFT JOIN address_stats AS s ON s.address = d.address
       ) AS state
     ),
     updated AS (
       UPDATE address_balances_summary AS s
          SET total = s.total + d.total_delta,
              included_supply = s.included_supply + d.included_supply_delta
         FROM summary_delta AS d
        WHERE s.singleton
      RETURNING s.singleton
     )
     INSERT INTO address_balances_summary (singleton, total, included_supply)
     SELECT TRUE, d.total_delta, d.included_supply_delta
       FROM summary_delta AS d
       LEFT JOIN updated AS u ON TRUE
      WHERE u.singleton IS NULL",
  )
  .bind(addrs)
  .bind(balances)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("apply address_stats summary chunk", e))?;
  Ok(())
}

fn is_balances_summary_constraint_error(message: &str) -> bool {
  message.contains("address_balances_summary_total_check")
    || message.contains("address_balances_summary_included_supply_check")
}

/// Rebuild the singleton balances summary from the canonical balances table.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn refresh_address_balances_summary_pool(pool: &sqlx::PgPool) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO address_balances_summary (singleton, total, included_supply)
     SELECT TRUE,
            COUNT(*)::bigint,
            COALESCE(SUM(balance), 0)::bigint
       FROM address_stats
      WHERE balance > 0
     ON CONFLICT (singleton) DO UPDATE SET
       total = EXCLUDED.total,
       included_supply = EXCLUDED.included_supply",
  )
  .execute(pool)
  .await
  .map_err(|e| pg_err("refresh address_balances summary", e))?;
  Ok(())
}

/// Rebuild the singleton balances summary from the canonical balances table.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn refresh_address_balances_summary(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO address_balances_summary (singleton, total, included_supply)
     SELECT TRUE,
            COUNT(*)::bigint,
            COALESCE(SUM(balance), 0)::bigint
       FROM address_stats
      WHERE balance > 0
     ON CONFLICT (singleton) DO UPDATE SET
       total = EXCLUDED.total,
       included_supply = EXCLUDED.included_supply",
  )
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("refresh address_balances summary", e))?;
  Ok(())
}

async fn temp_table_exists(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  table_name: &str,
) -> Result<bool, DaemonError> {
  sqlx::query_scalar::<_, Option<String>>("SELECT to_regclass(format('pg_temp.%s', $1))::text")
    .bind(table_name)
    .fetch_one(&mut **tx)
    .await
    .map(|value| value.is_some())
    .map_err(|e| pg_err(&format!("check temp table {table_name}"), e))
}

// ── Disconnect phases ──────────────────────────────────────────────────────

/// Restore spent markers on outputs for one disconnect height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn restore_spent_at_height(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  let undo_meta_rows = sqlx::query_scalar::<_, i64>(
    "SELECT COUNT(*)
       FROM block_undo_meta
      WHERE height = $1",
  )
  .bind(height)
  .fetch_one(&mut **tx)
  .await
  .map_err(|e| pg_err(&format!("count block_undo_meta at {height}"), e))?;
  if undo_meta_rows == 0 {
    return Err(DaemonError::Index(format!(
      "missing block_undo_meta at {height}; wipe or reindex postgres before disconnecting"
    )));
  }
  let expected_prevouts = sqlx::query_scalar::<_, i64>(
    "SELECT COUNT(*)::bigint
       FROM tx_inputs AS i
       JOIN transactions AS t ON t.txid = i.txid
      WHERE t.block_height = $1
        AND i.prev_txid IS NOT NULL
        AND i.prev_vout IS NOT NULL",
  )
  .bind(height)
  .fetch_one(&mut **tx)
  .await
  .map_err(|e| pg_err(&format!("count expected prevouts at {height}"), e))?;
  let undo_rows = sqlx::query_scalar::<_, i64>(
    "SELECT COUNT(*)
       FROM block_undo_outputs
      WHERE height = $1",
  )
  .bind(height)
  .fetch_one(&mut **tx)
  .await
  .map_err(|e| pg_err(&format!("count block_undo_outputs at {height}"), e))?;
  if undo_rows != expected_prevouts {
    return Err(DaemonError::Index(format!(
      "block_undo_outputs mismatch at {height}: expected {expected_prevouts}, found {undo_rows}; wipe or reindex postgres before disconnecting"
    )));
  }
  crate::sql::undo::restore_tx_outputs_from_undo(tx, height).await
}

async fn count_expected_touched_addresses_at_height(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<i64, DaemonError> {
  sqlx::query_scalar::<_, i64>(
    "SELECT COUNT(*)::bigint
       FROM (
         SELECT DISTINCT o.address
           FROM tx_outputs AS o
          WHERE o.block_height = $1
            AND o.address IS NOT NULL
         UNION
         SELECT DISTINCT i.prev_address AS address
           FROM tx_inputs AS i
          WHERE i.block_height = $1
            AND i.prev_address IS NOT NULL
       ) AS touched",
  )
  .bind(height)
  .fetch_one(&mut **tx)
  .await
  .map_err(|e| pg_err(&format!("count touched addresses at {height}"), e))
}

/// Reverse address balances updates owned by one disconnected height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn reverse_address_balances_at_height(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  crate::sql::undo::restore_address_balances_summary_from_undo(tx, height).await?;

  sqlx::query("DELETE FROM spent_output_history WHERE height = $1")
    .bind(height)
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("delete spent history", e))?;
  Ok(())
}

/// Remove derived address-history rows owned by one disconnected height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn reverse_address_history_at_height(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  sqlx::query(
    "DELETE FROM address_txs AS a
      USING transactions AS t
     WHERE t.block_height = $1
       AND a.txid = t.txid",
  )
  .bind(height)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err(&format!("reverse address history at {height}"), e))?;
  Ok(())
}

#[cfg(test)]
pub async fn seed_address_history_tx_for_rest_tests(
  pool: &PgPool,
  address: &str,
  height: i32,
  block_pos: i32,
  txid: &[u8],
  value_out: i64,
  fee: i64,
  received_value: i64,
  sent_value: i64,
) -> Result<(), Box<dyn std::error::Error>> {
  sqlx::query(
    "INSERT INTO blocks (
       height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count, chainwork
     ) VALUES ($1, $2, $3, $4, $1, 486604799, $1, 4, 512, 1, $5)
     ON CONFLICT (height) DO NOTHING",
  )
  .bind(height)
  .bind(vec![height as u8; 32])
  .bind(vec![height.saturating_sub(1) as u8; 32])
  .bind(vec![0x22_u8; 32])
  .bind(vec![0x33_u8; 32])
  .execute(pool)
  .await?;
  sqlx::query(
    "INSERT INTO transactions (
       txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count, fee
     ) VALUES ($1, $2, $3, 3, 'normal', 0, 200, $4, 1, 1, $5)",
  )
  .bind(txid)
  .bind(height)
  .bind(block_pos)
  .bind(value_out)
  .bind(Some(fee))
  .execute(pool)
  .await?;
  sqlx::query(
    "INSERT INTO address_txs (address, txid, block_height, block_pos)
     VALUES ($1, $2, $3, $4)",
  )
  .bind(address)
  .bind(txid)
  .bind(height)
  .bind(block_pos)
  .execute(pool)
  .await?;
  if received_value > 0 {
    sqlx::query(
      "INSERT INTO tx_outputs (
         txid, vout_index, block_height, value, script_pubkey, script_type, address
       ) VALUES ($1, 0, $2, $3, $4, 'p2pkh', $5)",
    )
    .bind(txid)
    .bind(height)
    .bind(received_value)
    .bind(vec![0x51_u8])
    .bind(address)
    .execute(pool)
    .await?;
  }
  if sent_value > 0 {
    sqlx::query(
      "INSERT INTO tx_inputs (
         txid, vin_index, prev_txid, prev_vout, script_sig, sequence, prev_address, prev_value
       ) VALUES ($1, 0, NULL, NULL, $2, 0, $3, $4)",
    )
    .bind(txid)
    .bind(vec![0x51_u8])
    .bind(address)
    .bind(sent_value)
    .execute(pool)
    .await?;
  }
  Ok(())
}

/// Reverse derived address summary rows owned by one disconnected height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn reverse_address_stats_at_height(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  let expected_rows = count_expected_touched_addresses_at_height(tx, height).await?;
  let undo_rows = sqlx::query_scalar::<_, i64>(
    "SELECT COUNT(*)
       FROM block_undo_address_stats
      WHERE height = $1",
  )
  .bind(height)
  .fetch_one(&mut **tx)
  .await
  .map_err(|e| pg_err(&format!("count block_undo_address_stats at {height}"), e))?;
  if undo_rows != expected_rows {
    return Err(DaemonError::Index(format!(
      "block_undo_address_stats mismatch at {height}: expected {expected_rows}, found {undo_rows}; wipe or reindex postgres before disconnecting"
    )));
  }
  crate::sql::undo::restore_address_stats_from_undo(tx, height).await
}
#[cfg(test)]
async fn refresh_address_stats_for_addresses(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  addresses: &[String],
) -> Result<(), DaemonError> {
  if addresses.is_empty() {
    return Ok(());
  }

  sqlx::query(
    "DELETE FROM address_stats
      WHERE address = ANY($1::text[])",
  )
  .bind(addresses)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("clear touched address_stats rows", e))?;

  sqlx::query(
    "WITH touched AS (
       SELECT DISTINCT address
         FROM unnest($1::text[]) AS input(address)
     ),
     tx_history AS (
       SELECT h.address,
              COUNT(*)::int AS tx_count,
              MIN(h.block_height) AS first_seen_height,
              MAX(h.block_height) AS last_seen_height
         FROM address_txs AS h
         JOIN touched AS t ON t.address = h.address
        GROUP BY h.address
     ),
     output_summary AS (
       SELECT o.address,
              COALESCE(SUM(o.value), 0)::bigint AS total_received,
              COALESCE(SUM(CASE WHEN o.spent_txid IS NULL THEN o.value ELSE 0 END), 0)::bigint AS balance,
              COALESCE(SUM(CASE WHEN o.spent_txid IS NULL THEN 1 ELSE 0 END), 0)::int AS utxo_count
         FROM tx_outputs AS o
         JOIN touched AS t ON t.address = o.address
        GROUP BY o.address
     )
     INSERT INTO address_stats (
       address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
     )
     SELECT t.address,
            COALESCE(o.balance, 0) AS balance,
            COALESCE(o.total_received, 0) AS total_received,
            COALESCE(h.tx_count, 0) AS tx_count,
            COALESCE(o.utxo_count, 0) AS utxo_count,
            h.first_seen_height,
            h.last_seen_height
       FROM touched AS t
       LEFT JOIN output_summary AS o ON o.address = t.address
       LEFT JOIN tx_history AS h ON h.address = t.address
      WHERE COALESCE(o.balance, 0) <> 0
         OR COALESCE(o.total_received, 0) <> 0
         OR COALESCE(h.tx_count, 0) <> 0
         OR COALESCE(o.utxo_count, 0) <> 0",
  )
  .bind(addresses)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("refresh touched address_stats rows", e))?;
  Ok(())
}
