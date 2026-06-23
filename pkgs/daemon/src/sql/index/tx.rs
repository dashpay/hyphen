//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Transaction index SQL operations (batch unnest inserts).

use sqlx::PgPool;

use crate::error::DaemonError;
use crate::sql::pg_err;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TransactionsSummaryRow {
  pub total: i64,
}

#[derive(Debug, Clone)]
pub struct TxRow {
  pub txid: Vec<u8>,
  pub block_height: i32,
  pub block_pos: i32,
  pub version: i32,
  pub tx_type: crate::sql::types::TxTypeDb,
  pub lock_time: i32,
  pub size: i32,
  pub value_out: i64,
  pub input_count: i32,
  pub output_count: i32,
  pub fee: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TxInputRow {
  pub vin_index: i32,
  pub prev_txid: Option<Vec<u8>>,
  pub prev_vout: Option<i32>,
  pub script_sig: Vec<u8>,
  pub sequence: i64,
  pub prev_address: Option<String>,
  pub prev_value: Option<i64>,
}

#[derive(Debug, Clone)]
pub struct TxOutputRow {
  pub vout_index: i32,
  pub value: i64,
  pub script_pubkey: Vec<u8>,
  pub address: Option<String>,
  pub spent_txid: Option<Vec<u8>>,
}

#[derive(Debug, Clone)]
pub struct TxPayloadRow {
  pub block_height: i32,
  pub tx_type: crate::sql::types::TxTypeDb,
  pub payload: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TxLocation {
  pub block_height: i32,
  pub block_pos: i32,
}

#[derive(Debug, Clone)]
pub struct TxSearchRow {
  pub txid: Vec<u8>,
  pub block_height: i32,
  pub block_pos: i32,
}

/// Read one indexed transaction row.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn transaction(pool: &PgPool, txid: &[u8]) -> Result<Option<TxRow>, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query(
    "SELECT txid, block_height, block_pos, version, tx_type, lock_time,
            size, value_out, input_count, output_count, fee
     FROM transactions WHERE txid = $1",
  )
  .bind(txid)
  .fetch_optional(pool)
  .await
  .map_err(|e| pg_err("read transaction", e))?;

  Ok(row.map(|row| TxRow {
    txid: row.get("txid"),
    block_height: row.get("block_height"),
    block_pos: row.get("block_pos"),
    version: row.get("version"),
    tx_type: row.get("tx_type"),
    lock_time: row.get("lock_time"),
    size: row.get("size"),
    value_out: row.get("value_out"),
    input_count: row.get("input_count"),
    output_count: row.get("output_count"),
    fee: row.get("fee"),
  }))
}

/// Read the input rows for one transaction.
///
/// When prevout details are requested, this query prefers the denormalized
/// `tx_inputs.prev_address` / `prev_value` columns but falls back to the
/// committed `tx_outputs` prevout row when older indexed rows still have null
/// input metadata.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn transaction_inputs(
  pool: &PgPool,
  txid: &[u8],
  include_prevout_details: bool,
) -> Result<Vec<TxInputRow>, DaemonError> {
  use sqlx::Row;

  let query = if include_prevout_details {
    "SELECT i.vin_index, i.prev_txid, i.prev_vout, i.script_sig, i.sequence,
            COALESCE(i.prev_address, o.address) AS prev_address,
            COALESCE(i.prev_value, o.value) AS prev_value
       FROM tx_inputs AS i
       LEFT JOIN tx_outputs AS o
         ON o.txid = i.prev_txid
        AND o.vout_index = i.prev_vout
      WHERE i.txid = $1
      ORDER BY i.vin_index"
  } else {
    "SELECT vin_index, prev_txid, prev_vout, script_sig, sequence,
            NULL::text AS prev_address, NULL::bigint AS prev_value
     FROM tx_inputs WHERE txid = $1 ORDER BY vin_index"
  };

  let rows = sqlx::query(query)
    .bind(txid)
    .fetch_all(pool)
    .await
    .map_err(|e| pg_err("read transaction inputs", e))?;

  Ok(
    rows
      .iter()
      .map(|row| TxInputRow {
        vin_index: row.get("vin_index"),
        prev_txid: row.get("prev_txid"),
        prev_vout: row.get("prev_vout"),
        script_sig: row.get("script_sig"),
        sequence: row.get("sequence"),
        prev_address: row.get("prev_address"),
        prev_value: row.get("prev_value"),
      })
      .collect(),
  )
}

/// Read the output rows for one transaction.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn transaction_outputs(
  pool: &PgPool,
  txid: &[u8],
  include_address_details: bool,
) -> Result<Vec<TxOutputRow>, DaemonError> {
  use sqlx::Row;

  let query = if include_address_details {
    "SELECT vout_index, value, script_pubkey, address, spent_txid
     FROM tx_outputs WHERE txid = $1 ORDER BY vout_index"
  } else {
    "SELECT vout_index, value, script_pubkey, NULL::text AS address, NULL::bytea AS spent_txid
     FROM tx_outputs WHERE txid = $1 ORDER BY vout_index"
  };

  let rows = sqlx::query(query)
    .bind(txid)
    .fetch_all(pool)
    .await
    .map_err(|e| pg_err("read transaction outputs", e))?;

  Ok(
    rows
      .iter()
      .map(|row| TxOutputRow {
        vout_index: row.get("vout_index"),
        value: row.get("value"),
        script_pubkey: row.get("script_pubkey"),
        address: row.get("address"),
        spent_txid: row.get("spent_txid"),
      })
      .collect(),
  )
}

/// Read the special transaction payload for one transaction.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn transaction_payload(
  pool: &PgPool,
  txid: &[u8],
) -> Result<Option<TxPayloadRow>, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query(
    "SELECT block_height, tx_type, payload
       FROM special_tx_payloads
      WHERE txid = $1
      ORDER BY block_height DESC
      LIMIT 1",
  )
  .bind(txid)
  .fetch_optional(pool)
  .await
  .map_err(|e| pg_err("read transaction payload", e))?;

  Ok(row.map(|row| TxPayloadRow {
    block_height: row.get("block_height"),
    tx_type: row.get("tx_type"),
    payload: row.get("payload"),
  }))
}

/// Read transaction suggestions by canonical txid prefix.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn search_transactions_by_prefix(
  pool: &PgPool,
  prefix: &str,
  limit: i64,
) -> Result<Vec<TxSearchRow>, DaemonError> {
  use sqlx::Row;

  let pattern = format!("{prefix}%");
  let rows = sqlx::query(
    "SELECT txid, block_height, block_pos
       FROM transactions
      WHERE encode(txid, 'hex') LIKE $1
      ORDER BY block_height DESC, block_pos DESC, txid ASC
      LIMIT $2",
  )
  .bind(pattern)
  .bind(limit)
  .fetch_all(pool)
  .await
  .map_err(|e| pg_err("search transaction prefix", e))?;

  Ok(
    rows
      .into_iter()
      .map(|row| TxSearchRow {
        txid: row.get("txid"),
        block_height: row.get("block_height"),
        block_pos: row.get("block_pos"),
      })
      .collect(),
  )
}

/// Read the block location for one transaction.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn transaction_location(
  pool: &PgPool,
  txid: &[u8],
) -> Result<Option<TxLocation>, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query("SELECT block_height, block_pos FROM transactions WHERE txid = $1")
    .bind(txid)
    .fetch_optional(pool)
    .await
    .map_err(|e| pg_err("read transaction location", e))?;

  Ok(row.map(|row| TxLocation {
    block_height: row.get("block_height"),
    block_pos: row.get("block_pos"),
  }))
}

/// Insert one indexed transaction row.
///
/// # Errors
///
/// Returns an error when PostgreSQL rejects the insert.
pub async fn insert_transaction_row(
  conn: &mut sqlx::PgConnection,
  row: &TxRow,
) -> Result<(), DaemonError> {
  ensure_canonical_partitions_conn(conn, row.block_height).await?;
  sqlx::query(
    "INSERT INTO transactions (
       txid, block_height, block_pos, version, tx_type, lock_time, size,
       value_out, input_count, output_count, fee
     ) VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
     ON CONFLICT (block_height, txid) DO NOTHING",
  )
  .bind(&row.txid)
  .bind(row.block_height)
  .bind(row.block_pos)
  .bind(row.version)
  .bind(row.tx_type.to_string())
  .bind(row.lock_time)
  .bind(row.size)
  .bind(row.value_out)
  .bind(row.input_count)
  .bind(row.output_count)
  .bind(row.fee)
  .execute(&mut *conn)
  .await
  .map_err(|e| pg_err("insert transaction row", e))?;
  Ok(())
}

/// Insert one set of indexed input rows for a transaction.
///
/// # Errors
///
/// Returns an error when PostgreSQL rejects the insert.
pub async fn insert_transaction_input_rows(
  conn: &mut sqlx::PgConnection,
  block_height: i32,
  txid: &[u8],
  rows: &[TxInputRow],
) -> Result<(), DaemonError> {
  if rows.is_empty() {
    return Ok(());
  }
  let txids = std::iter::repeat_with(|| txid.to_vec())
    .take(rows.len())
    .collect::<Vec<_>>();
  let block_heights = std::iter::repeat_n(block_height, rows.len()).collect::<Vec<_>>();
  let vin_indexes = rows.iter().map(|row| row.vin_index).collect::<Vec<_>>();
  let prev_txids = rows
    .iter()
    .map(|row| row.prev_txid.clone())
    .collect::<Vec<_>>();
  let prev_vouts = rows.iter().map(|row| row.prev_vout).collect::<Vec<_>>();
  let script_sigs = rows
    .iter()
    .map(|row| row.script_sig.clone())
    .collect::<Vec<_>>();
  let sequences = rows.iter().map(|row| row.sequence).collect::<Vec<_>>();
  let prev_addresses = rows
    .iter()
    .map(|row| row.prev_address.clone())
    .collect::<Vec<_>>();
  let prev_values = rows.iter().map(|row| row.prev_value).collect::<Vec<_>>();
  sqlx::query(
    "INSERT INTO tx_inputs (
       block_height, txid, vin_index, prev_txid, prev_vout, script_sig, sequence,
       prev_address, prev_value
     )
     SELECT * FROM unnest(
       $1::int[], $2::bytea[], $3::int[], $4::bytea[], $5::int[],
       $6::bytea[], $7::bigint[], $8::text[], $9::bigint[]
     )
     ON CONFLICT (block_height, txid, vin_index) DO NOTHING",
  )
  .bind(block_heights)
  .bind(txids)
  .bind(vin_indexes)
  .bind(prev_txids)
  .bind(prev_vouts)
  .bind(script_sigs)
  .bind(sequences)
  .bind(prev_addresses)
  .bind(prev_values)
  .execute(&mut *conn)
  .await
  .map_err(|e| pg_err("insert transaction input rows", e))?;
  Ok(())
}

/// Insert one set of indexed output rows for a transaction.
///
/// # Errors
///
/// Returns an error when PostgreSQL rejects the insert.
pub async fn insert_transaction_output_rows(
  conn: &mut sqlx::PgConnection,
  txid: &[u8],
  block_height: i32,
  rows: &[TxOutputRow],
) -> Result<(), DaemonError> {
  ensure_canonical_partitions_conn(conn, block_height).await?;
  if rows.is_empty() {
    return Ok(());
  }
  let txids = std::iter::repeat_with(|| txid.to_vec())
    .take(rows.len())
    .collect::<Vec<_>>();
  let block_heights = std::iter::repeat_n(block_height, rows.len()).collect::<Vec<_>>();
  let vout_indexes = rows.iter().map(|row| row.vout_index).collect::<Vec<_>>();
  let values = rows.iter().map(|row| row.value).collect::<Vec<_>>();
  let script_pubkeys = rows
    .iter()
    .map(|row| row.script_pubkey.clone())
    .collect::<Vec<_>>();
  let addresses = rows
    .iter()
    .map(|row| row.address.clone())
    .collect::<Vec<_>>();
  let spent_txids = rows
    .iter()
    .map(|row| row.spent_txid.clone())
    .collect::<Vec<_>>();
  sqlx::query(
    "INSERT INTO tx_outputs (
       txid, vout_index, block_height, value, script_pubkey, address, spent_txid
     )
     SELECT * FROM unnest(
       $1::bytea[], $2::int[], $3::int[], $4::bigint[], $5::bytea[], $6::text[], $7::bytea[]
     )
     ON CONFLICT (block_height, txid, vout_index) DO NOTHING",
  )
  .bind(txids)
  .bind(vout_indexes)
  .bind(block_heights)
  .bind(values)
  .bind(script_pubkeys)
  .bind(addresses)
  .bind(spent_txids)
  .execute(&mut *conn)
  .await
  .map_err(|e| pg_err("insert transaction output rows", e))?;
  Ok(())
}

/// List indexed transactions in reverse chain order.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn list_transactions(
  pool: &PgPool,
  limit: i64,
  before: Option<(i32, i32, &[u8])>,
) -> Result<Vec<TxRow>, DaemonError> {
  use sqlx::Row;

  let rows = match before {
    Some((before_height, before_pos, before_txid)) => {
      sqlx::query(
        "SELECT txid, block_height, block_pos, version, tx_type, lock_time,
                size, value_out, input_count, output_count, fee
         FROM transactions
         WHERE (block_height, block_pos, txid) < ($1, $2, $3)
         ORDER BY block_height DESC, block_pos DESC, txid DESC
         LIMIT $4",
      )
      .bind(before_height)
      .bind(before_pos)
      .bind(before_txid)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    None => {
      sqlx::query(
        "SELECT txid, block_height, block_pos, version, tx_type, lock_time,
                size, value_out, input_count, output_count, fee
         FROM transactions
         ORDER BY block_height DESC, block_pos DESC, txid DESC
         LIMIT $1",
      )
      .bind(limit)
      .fetch_all(pool)
      .await
    }
  }
  .map_err(|e| pg_err("list transactions", e))?;

  Ok(
    rows
      .iter()
      .map(|row| TxRow {
        txid: row.get("txid"),
        block_height: row.get("block_height"),
        block_pos: row.get("block_pos"),
        version: row.get("version"),
        tx_type: row.get("tx_type"),
        lock_time: row.get("lock_time"),
        size: row.get("size"),
        value_out: row.get("value_out"),
        input_count: row.get("input_count"),
        output_count: row.get("output_count"),
        fee: row.get("fee"),
      })
      .collect(),
  )
}

#[expect(clippy::too_many_arguments)]
/// Insert a batch of transaction rows for one indexed height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn insert_transactions(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
  txids: &[Vec<u8>],
  heights: &[i32],
  positions: &[i32],
  versions: &[i32],
  types: &[String],
  lock_times: &[i32],
  sizes: &[i32],
  values_out: &[i64],
  input_counts: &[i32],
  output_counts: &[i32],
) -> Result<(), DaemonError> {
  let inserted = i64::try_from(
    sqlx::query(
      "INSERT INTO transactions (
       txid, block_height, block_pos, version, tx_type, lock_time,
       size, value_out, input_count, output_count
     ) SELECT * FROM unnest(
       $1::bytea[], $2::int[], $3::int[], $4::int[],
       $5::text[]::tx_type[], $6::int[], $7::int[], $8::bigint[],
       $9::int[], $10::int[]
     ) ON CONFLICT (block_height, txid) DO NOTHING",
    )
    .bind(txids)
    .bind(heights)
    .bind(positions)
    .bind(versions)
    .bind(types)
    .bind(lock_times)
    .bind(sizes)
    .bind(values_out)
    .bind(input_counts)
    .bind(output_counts)
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err(&format!("batch insert transactions at {height}"), e))?
    .rows_affected(),
  )
  .map_err(|_| DaemonError::Pg { message: "batch insert transactions rows_affected overflow".to_owned(), code: None })?;
  apply_transactions_summary_delta(tx, inserted).await?;
  Ok(())
}

#[expect(clippy::too_many_arguments)]
/// Insert a batch of transaction input rows for one indexed height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn insert_tx_inputs(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
  txids: &[Vec<u8>],
  vins: &[i32],
  prev_txids: &[Option<Vec<u8>>],
  prev_vouts: &[Option<i32>],
  scripts: &[Vec<u8>],
  sequences: &[i64],
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO tx_inputs (
       block_height, txid, vin_index, prev_txid, prev_vout, script_sig, sequence
     )
     SELECT $1, txid, vin_index, prev_txid, prev_vout, script_sig, sequence
       FROM unnest(
         $2::bytea[], $3::int[], $4::bytea[], $5::int[], $6::bytea[], $7::bigint[]
       ) AS v(txid, vin_index, prev_txid, prev_vout, script_sig, sequence)
      ORDER BY txid, vin_index
     ON CONFLICT (block_height, txid, vin_index) DO NOTHING",
  )
  .bind(height as i32)
  .bind(txids)
  .bind(vins)
  .bind(prev_txids)
  .bind(prev_vouts)
  .bind(scripts)
  .bind(sequences)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err(&format!("batch insert tx_inputs at {height}"), e))?;
  Ok(())
}

/// Prepare the temporary staging table used for chunked `tx_outputs` writes.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn prepare_tx_outputs_staging(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), DaemonError> {
  if !temp_table_exists(tx, "tmp_tx_outputs_delta").await? {
    sqlx::query(
      "CREATE TEMP TABLE tmp_tx_outputs_delta (
         stage_id bigint GENERATED ALWAYS AS IDENTITY,
         txid bytea NOT NULL,
         vout_index int NOT NULL,
         block_height int NOT NULL,
         value bigint NOT NULL,
         script_pubkey bytea NOT NULL,
         address text,
         address_type address_type
       ) ON COMMIT PRESERVE ROWS",
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("create tx_outputs staging", e))?;
  }

  sqlx::query("TRUNCATE tmp_tx_outputs_delta RESTART IDENTITY")
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("truncate tx_outputs staging", e))?;
  Ok(())
}

#[expect(clippy::too_many_arguments)]
/// Stage a batch of transaction outputs into the temporary output delta table.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn stage_tx_outputs(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  txids: &[Vec<u8>],
  vouts: &[i32],
  heights: &[i32],
  values: &[i64],
  scripts: &[Vec<u8>],
  addrs: &[Option<String>],
  addr_types: &[Option<String>],
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO tmp_tx_outputs_delta (
       txid, vout_index, block_height, value, script_pubkey, address, address_type
     )
     SELECT * FROM unnest(
       $1::bytea[], $2::int[], $3::int[], $4::bigint[], $5::bytea[], $6::text[], $7::text[]::address_type[]
     )",
  )
  .bind(txids)
  .bind(vouts)
  .bind(heights)
  .bind(values)
  .bind(scripts)
  .bind(addrs)
  .bind(addr_types)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("stage tx_outputs", e))?;
  Ok(())
}

/// Count the rows currently staged for chunked `tx_outputs` application.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn staged_tx_outputs_count(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<i64, DaemonError> {
  sqlx::query_scalar("SELECT COUNT(*) FROM tmp_tx_outputs_delta")
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| pg_err("count staged tx_outputs", e))
}

/// Apply one staged `tx_outputs` chunk into the final table.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn apply_staged_tx_outputs_chunk(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
  start_stage_id: i64,
  end_stage_id: i64,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO tx_outputs (
       txid, vout_index, block_height, value, script_pubkey, address, address_type
     )
     SELECT txid, vout_index, block_height, value, script_pubkey, address, address_type
       FROM tmp_tx_outputs_delta
      WHERE stage_id > $1 AND stage_id <= $2
     ON CONFLICT (block_height, txid, vout_index) DO NOTHING",
  )
  .bind(start_stage_id)
  .bind(end_stage_id)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err(&format!("apply staged tx_outputs chunk at {height}"), e))?;
  Ok(())
}

/// Insert a batch of special transaction payload rows.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn insert_special_tx_payloads(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  heights: &[i32],
  txids: &[&[u8]],
  types: &[crate::sql::types::TxTypeDb],
  pro_tx_hashes: &[Option<&[u8]>],
  payloads: &[&serde_json::Value],
) -> Result<(), DaemonError> {
  let type_strs: Vec<String> = types.iter().map(|t| t.to_string()).collect();
  let txid_vecs: Vec<&[u8]> = txids.to_vec();
  let hash_vecs: Vec<Option<Vec<u8>>> = pro_tx_hashes
    .iter()
    .map(|h| h.map(|b| b.to_vec()))
    .collect();
  let payload_vals: Vec<serde_json::Value> = payloads.iter().map(|p| (*p).clone()).collect();
  sqlx::query(
    "INSERT INTO special_tx_payloads (block_height, txid, tx_type, pro_tx_hash, payload)
     SELECT * FROM unnest(
       $1::int[], $2::bytea[], $3::text[]::tx_type[], $4::bytea[], $5::jsonb[]
     )
     ON CONFLICT (block_height, txid) DO NOTHING",
  )
  .bind(heights)
  .bind(&txid_vecs)
  .bind(&type_strs)
  .bind(&hash_vecs)
  .bind(&payload_vals)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("batch insert special_tx_payloads", e))?;
  Ok(())
}

/// Delete transaction outputs belonging to one block height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn delete_tx_outputs(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM tx_outputs WHERE block_height = $1")
    .bind(height as i32)
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err(&format!("delete tx_outputs {height}"), e))?;
  Ok(())
}

/// Delete transaction inputs belonging to one block height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn delete_tx_inputs(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM tx_inputs WHERE block_height = $1")
    .bind(height as i32)
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err(&format!("delete tx_inputs {height}"), e))?;
  Ok(())
}

/// Delete special transaction payloads belonging to one block height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn delete_special_tx_payloads(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM special_tx_payloads WHERE block_height = $1")
    .bind(height as i32)
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err(&format!("delete special_tx_payloads {height}"), e))?;
  Ok(())
}

/// Delete transaction rows belonging to one block height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn delete_transactions(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
) -> Result<(), DaemonError> {
  let deleted = i64::try_from(
    sqlx::query("DELETE FROM transactions WHERE block_height = $1")
      .bind(height as i32)
      .execute(&mut **tx)
      .await
      .map_err(|e| pg_err(&format!("delete transactions {height}"), e))?
      .rows_affected(),
  )
  .map_err(|_| DaemonError::Pg { message: "delete transactions rows_affected overflow".to_owned(), code: None })?;
  apply_transactions_summary_delta(tx, -deleted).await?;
  Ok(())
}

/// Delete transaction rows belonging to one block height and rebuild the
/// singleton summary exactly from canonical rows.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn delete_transactions_exact_summary(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM transactions WHERE block_height = $1")
    .bind(height as i32)
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err(&format!("delete transactions {height}"), e))?;
  let undo_rows = sqlx::query_scalar::<_, i64>(
    "SELECT COUNT(*)
       FROM block_undo_meta
      WHERE height = $1",
  )
  .bind(height as i32)
  .fetch_one(&mut **tx)
  .await
  .map_err(|e| pg_err(&format!("count block_undo_meta at {height}"), e))?;
  if undo_rows == 0 {
    return Err(DaemonError::Index(format!(
      "missing block_undo_meta at {height}; wipe or reindex postgres before disconnecting"
    )));
  }
  crate::sql::undo::restore_transactions_summary_from_undo(tx, height as i32).await
}

/// Read the current indexed transactions total.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn transactions_summary(pool: &PgPool) -> Result<TransactionsSummaryRow, DaemonError> {
  let total = sqlx::query_scalar::<_, Option<i64>>(
    "SELECT total
       FROM transactions_summary
      WHERE singleton = TRUE",
  )
  .fetch_optional(pool)
  .await
  .map_err(|e| pg_err("read transactions summary", e))?
  .flatten()
  .unwrap_or(0);
  Ok(TransactionsSummaryRow { total })
}

/// Rebuild the singleton transactions summary from the canonical transactions table.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn refresh_transactions_summary(pool: &PgPool) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO transactions_summary (singleton, total)
     SELECT TRUE, COUNT(*)::bigint
       FROM transactions
     ON CONFLICT (singleton) DO UPDATE SET
       total = EXCLUDED.total",
  )
  .execute(pool)
  .await
  .map_err(|e| pg_err("refresh transactions summary", e))?;
  Ok(())
}

async fn refresh_transactions_summary_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO transactions_summary (singleton, total)
     SELECT TRUE, COUNT(*)::bigint
       FROM transactions
     ON CONFLICT (singleton) DO UPDATE SET
       total = EXCLUDED.total",
  )
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("refresh transactions summary in tx", e))?;
  Ok(())
}

async fn apply_transactions_summary_delta(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  delta: i64,
) -> Result<(), DaemonError> {
  if delta == 0 {
    return Ok(());
  }
  sqlx::query("SAVEPOINT transactions_summary_delta")
    .execute(&mut **tx)
    .await
    .map_err(|e| pg_err("savepoint transactions summary delta", e))?;
  match apply_transactions_summary_delta_once(tx, delta).await {
    Ok(()) => {
      sqlx::query("RELEASE SAVEPOINT transactions_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("release savepoint transactions summary delta", e))?;
      Ok(())
    }
    Err(DaemonError::Pg { message, .. }) if message.contains("transactions_summary_total_check") => {
      sqlx::query("ROLLBACK TO SAVEPOINT transactions_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("rollback savepoint transactions summary delta", e))?;
      sqlx::query("RELEASE SAVEPOINT transactions_summary_delta")
        .execute(&mut **tx)
        .await
        .map_err(|e| pg_err("release savepoint transactions summary delta", e))?;
      tracing::warn!(
        "transactions_summary constraint failed during incremental apply; refreshing singleton exactly"
      );
      refresh_transactions_summary_tx(tx).await?;
      Ok(())
    }
    Err(err) => {
      let _ = sqlx::query("ROLLBACK TO SAVEPOINT transactions_summary_delta")
        .execute(&mut **tx)
        .await;
      let _ = sqlx::query("RELEASE SAVEPOINT transactions_summary_delta")
        .execute(&mut **tx)
        .await;
      Err(err)
    }
  }
}

async fn apply_transactions_summary_delta_once(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  delta: i64,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO transactions_summary (singleton, total)
     VALUES (TRUE, $1)
     ON CONFLICT (singleton) DO UPDATE SET
       total = transactions_summary.total + EXCLUDED.total",
  )
  .bind(delta)
  .execute(&mut **tx)
  .await
  .map_err(|e| pg_err("apply transactions summary delta", e))?;
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

pub async fn ensure_canonical_partitions(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: i32,
) -> Result<(), DaemonError> {
  for sql in crate::sql::partition::canonical_partition_sqls(height) {
    sqlx::query(&sql)
      .execute(&mut **tx)
      .await
      .map_err(|e| pg_err("ensure canonical partitions", e))?;
  }
  Ok(())
}

async fn ensure_canonical_partitions_conn(
  conn: &mut sqlx::PgConnection,
  height: i32,
) -> Result<(), DaemonError> {
  for sql in crate::sql::partition::canonical_partition_sqls(height) {
    sqlx::query(&sql)
      .execute(&mut *conn)
      .await
      .map_err(|e| pg_err("ensure canonical partitions", e))?;
  }
  Ok(())
}

#[cfg(test)]
mod tests {
  use serde_json::json;
  use sqlx::PgPool;

  use super::{
    list_transactions, transaction, transaction_inputs, transaction_location, transaction_outputs,
    transaction_payload, transactions_summary, TxInputRow, TxOutputRow, TxRow,
  };

  /// Build one stable test hash for seeded transaction-route rows.
  fn synthetic_hash(domain: u8, id: i32) -> Vec<u8> {
    let mut value = [0_u8; 32];
    value[0] = domain;
    value[1..5].copy_from_slice(&id.to_be_bytes());
    for (index, byte) in value[5..].iter_mut().enumerate() {
      *byte = domain.wrapping_add(index as u8).wrapping_mul(11);
    }
    value.to_vec()
  }

  /// Seed one canonical transaction row for point-read route tests.
  async fn seed_tx(
    pool: &PgPool,
    txid: Vec<u8>,
    block_height: i32,
    block_pos: i32,
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size,
         value_out, input_count, output_count, fee
       ) VALUES ($1, $2, $3, $4, 'normal', $5, $6, $7, $8, $9, $10)",
    )
    .bind(txid)
    .bind(block_height)
    .bind(block_pos)
    .bind(3_i32)
    .bind(0_i32)
    .bind(250_i32)
    .bind(1_000_i64)
    .bind(1_i32)
    .bind(1_i32)
    .bind(Some(10_i64))
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one transaction row from its typed SQL route shape.
  async fn seed_typed_tx(pool: &PgPool, row: &TxRow) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = pool.acquire().await?;
    super::insert_transaction_row(&mut conn, row).await?;
    Ok(())
  }

  /// Seed one transaction input set from the real typed insert helper.
  async fn seed_typed_inputs(
    pool: &PgPool,
    block_height: i32,
    txid: &[u8],
    rows: &[TxInputRow],
  ) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = pool.acquire().await?;
    super::insert_transaction_input_rows(&mut conn, block_height, txid, rows).await?;
    Ok(())
  }

  /// Seed one transaction output set from the real typed insert helper.
  async fn seed_typed_outputs(
    pool: &PgPool,
    txid: &[u8],
    block_height: i32,
    rows: &[TxOutputRow],
  ) -> Result<(), Box<dyn std::error::Error>> {
    let mut conn = pool.acquire().await?;
    super::insert_transaction_output_rows(&mut conn, txid, block_height, rows).await?;
    Ok(())
  }

  /// Seed one special transaction payload row because the point payload route reads its own table.
  async fn seed_payload_row(
    pool: &PgPool,
    block_height: i32,
    txid: &[u8],
  ) -> Result<(), Box<dyn std::error::Error>> {
    let mut tx = pool.begin().await?;
    let payload = json!({ "version": 1, "operatorReward": 0 });
    super::insert_special_tx_payloads(
      &mut tx,
      &[block_height],
      &[txid],
      &[crate::sql::types::TxTypeDb::ProviderRegister],
      &[None],
      &[&payload],
    )
    .await?;
    tx.commit().await?;
    Ok(())
  }

  #[tokio::test]
  async fn list_transactions_reads_later_page_with_tuple_cursor_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let newest_same_pos = synthetic_hash(b'T', 99);
    let older_same_pos = synthetic_hash(b'T', 1);
    seed_tx(&pool, newest_same_pos.clone(), 10, 0).await?;
    seed_tx(&pool, older_same_pos.clone(), 10, 0).await?;
    seed_tx(&pool, synthetic_hash(b'T', 2), 10, 1).await?;
    seed_tx(&pool, synthetic_hash(b'T', 3), 9, 0).await?;
    seed_tx(&pool, synthetic_hash(b'T', 4), 8, 0).await?;

    let first_page = list_transactions(&pool, 3, None).await?;
    assert_eq!(first_page.len(), 3);
    assert_eq!(first_page[0].block_height, 10);
    assert_eq!(first_page[0].block_pos, 1);
    assert_eq!(first_page[1].txid, newest_same_pos);
    assert_eq!(first_page[2].txid, older_same_pos);

    let later_page = list_transactions(&pool, 3, Some((10, 0, older_same_pos.as_slice()))).await?;
    assert_eq!(later_page.len(), 2);
    assert_eq!(later_page[0].block_height, 9);
    assert_eq!(later_page[1].block_height, 8);

    Ok(())
  }

  #[tokio::test]
  async fn transaction_reads_seeded_row_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let txid = synthetic_hash(b'T', 7);
    seed_typed_tx(
      &pool,
      &TxRow {
        txid: txid.clone(),
        block_height: 7,
        block_pos: 1,
        version: 3,
        tx_type: crate::sql::types::TxTypeDb::Normal,
        lock_time: 11,
        size: 222,
        value_out: 9_000,
        input_count: 2,
        output_count: 3,
        fee: Some(10),
      },
    )
    .await?;

    let row = transaction(&pool, txid.as_slice()).await?;
    assert!(row.is_some());
    let row = row.expect("seeded transaction should exist");
    assert_eq!(row.block_height, 7);
    assert_eq!(row.block_pos, 1);
    assert_eq!(row.value_out, 9_000);
    assert_eq!(row.output_count, 3);

    Ok(())
  }

  #[tokio::test]
  async fn transaction_inputs_prefer_denormalized_prevout_fields_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let prev_txid = synthetic_hash(b'P', 1);
    let txid = synthetic_hash(b'T', 8);
    seed_typed_tx(
      &pool,
      &TxRow {
        txid: txid.clone(),
        block_height: 8,
        block_pos: 0,
        version: 3,
        tx_type: crate::sql::types::TxTypeDb::Normal,
        lock_time: 0,
        size: 200,
        value_out: 1_000,
        input_count: 1,
        output_count: 1,
        fee: Some(5),
      },
    )
    .await?;
    seed_typed_inputs(
      &pool,
      8,
      txid.as_slice(),
      &[TxInputRow {
        vin_index: 0,
        prev_txid: Some(prev_txid.clone()),
        prev_vout: Some(1),
        script_sig: vec![0x51],
        sequence: 42,
        prev_address: Some("Xprev".to_owned()),
        prev_value: Some(1_234),
      }],
    )
    .await?;

    let rows = transaction_inputs(&pool, txid.as_slice(), true).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].prev_txid.as_deref(), Some(prev_txid.as_slice()));
    assert_eq!(rows[0].prev_address.as_deref(), Some("Xprev"));
    assert_eq!(rows[0].prev_value, Some(1_234));

    Ok(())
  }

  #[tokio::test]
  async fn transaction_outputs_read_address_details_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let txid = synthetic_hash(b'T', 9);
    let spent_txid = synthetic_hash(b'S', 1);
    seed_typed_tx(
      &pool,
      &TxRow {
        txid: txid.clone(),
        block_height: 9,
        block_pos: 0,
        version: 3,
        tx_type: crate::sql::types::TxTypeDb::Normal,
        lock_time: 0,
        size: 200,
        value_out: 2_000,
        input_count: 1,
        output_count: 2,
        fee: Some(5),
      },
    )
    .await?;
    seed_typed_outputs(
      &pool,
      txid.as_slice(),
      9,
      &[TxOutputRow {
        vout_index: 0,
        value: 2_000,
        script_pubkey: vec![0x51],
        address: Some("Xdest".to_owned()),
        spent_txid: Some(spent_txid.clone()),
      }],
    )
    .await?;

    let rows = transaction_outputs(&pool, txid.as_slice(), true).await?;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].address.as_deref(), Some("Xdest"));
    assert_eq!(rows[0].spent_txid.as_deref(), Some(spent_txid.as_slice()));

    Ok(())
  }

  #[tokio::test]
  async fn transaction_payload_and_location_read_seeded_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let txid = synthetic_hash(b'T', 10);
    seed_typed_tx(
      &pool,
      &TxRow {
        txid: txid.clone(),
        block_height: 10,
        block_pos: 2,
        version: 3,
        tx_type: crate::sql::types::TxTypeDb::ProviderRegister,
        lock_time: 0,
        size: 250,
        value_out: 4_000,
        input_count: 1,
        output_count: 1,
        fee: Some(8),
      },
    )
    .await?;
    seed_payload_row(&pool, 10, txid.as_slice()).await?;

    let payload = transaction_payload(&pool, txid.as_slice()).await?;
    assert!(payload.is_some());
    let payload = payload.expect("seeded payload should exist");
    assert_eq!(
      payload.tx_type,
      crate::sql::types::TxTypeDb::ProviderRegister
    );

    let location = transaction_location(&pool, txid.as_slice()).await?;
    assert_eq!(
      location,
      Some(super::TxLocation {
        block_height: 10,
        block_pos: 2,
      })
    );

    Ok(())
  }

  #[tokio::test]
  async fn transactions_summary_tracks_insert_and_delete_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let txid_a = synthetic_hash(b'T', 20);
    let txid_b = synthetic_hash(b'T', 21);
    seed_typed_tx(
      &pool,
      &TxRow {
        txid: txid_a.clone(),
        block_height: 20,
        block_pos: 0,
        version: 3,
        tx_type: crate::sql::types::TxTypeDb::Normal,
        lock_time: 0,
        size: 200,
        value_out: 1_000,
        input_count: 1,
        output_count: 1,
        fee: Some(5),
      },
    )
    .await?;
    seed_typed_tx(
      &pool,
      &TxRow {
        txid: txid_b.clone(),
        block_height: 20,
        block_pos: 1,
        version: 3,
        tx_type: crate::sql::types::TxTypeDb::Normal,
        lock_time: 0,
        size: 210,
        value_out: 2_000,
        input_count: 1,
        output_count: 2,
        fee: Some(6),
      },
    )
    .await?;
    assert_eq!(transactions_summary(&pool).await?.total, 2);

    let mut tx = pool.begin().await?;
    super::delete_transactions(&mut tx, 20).await?;
    tx.commit().await?;
    assert_eq!(transactions_summary(&pool).await?.total, 0);

    Ok(())
  }

  #[tokio::test]
  async fn transactions_summary_delta_repairs_stale_singleton_before_retry_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let txid_a = synthetic_hash(b'T', 30);
    seed_typed_tx(
      &pool,
      &TxRow {
        txid: txid_a,
        block_height: 30,
        block_pos: 0,
        version: 3,
        tx_type: crate::sql::types::TxTypeDb::Normal,
        lock_time: 0,
        size: 200,
        value_out: 1_000,
        input_count: 1,
        output_count: 1,
        fee: Some(5),
      },
    )
    .await?;
    assert_eq!(transactions_summary(&pool).await?.total, 1);

    sqlx::query(
      "INSERT INTO transactions_summary (singleton, total)
       VALUES (TRUE, -10)
       ON CONFLICT (singleton) DO UPDATE SET
         total = EXCLUDED.total",
    )
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    super::delete_transactions(&mut tx, 30).await?;
    tx.commit().await?;

    assert_eq!(transactions_summary(&pool).await?.total, 0);

    Ok(())
  }

  #[tokio::test]
  async fn transactions_summary_self_heal_rollback_leaves_committed_state_unchanged_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let txid_a = synthetic_hash(b'T', 31);
    seed_typed_tx(
      &pool,
      &TxRow {
        txid: txid_a,
        block_height: 31,
        block_pos: 0,
        version: 3,
        tx_type: crate::sql::types::TxTypeDb::Normal,
        lock_time: 0,
        size: 200,
        value_out: 1_000,
        input_count: 1,
        output_count: 1,
        fee: Some(5),
      },
    )
    .await?;

    sqlx::query(
      "INSERT INTO transactions_summary (singleton, total)
       VALUES (TRUE, -10)
       ON CONFLICT (singleton) DO UPDATE SET
         total = EXCLUDED.total",
    )
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    super::delete_transactions(&mut tx, 31).await?;
    tx.rollback().await?;

    assert_eq!(transactions_summary(&pool).await?.total, -10);
    let tx_count = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM transactions")
      .fetch_one(&pool)
      .await?;
    assert_eq!(tx_count, 1);

    Ok(())
  }

  #[tokio::test]
  async fn transactions_summary_delete_repair_refreshes_to_post_delete_total_without_reapplying_delta_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    for (index, height) in [(40_i32, 40_u32), (41_i32, 41_u32), (42_i32, 42_u32)] {
      seed_typed_tx(
        &pool,
        &TxRow {
          txid: synthetic_hash(b'T', index),
          block_height: height as i32,
          block_pos: 0,
          version: 3,
          tx_type: crate::sql::types::TxTypeDb::Normal,
          lock_time: 0,
          size: 200,
          value_out: 1_000,
          input_count: 1,
          output_count: 1,
          fee: Some(5),
        },
      )
      .await?;
    }
    assert_eq!(transactions_summary(&pool).await?.total, 3);

    sqlx::query(
      "INSERT INTO transactions_summary (singleton, total)
       VALUES (TRUE, -2817)
       ON CONFLICT (singleton) DO UPDATE SET
         total = EXCLUDED.total",
    )
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    super::delete_transactions(&mut tx, 42).await?;
    tx.commit().await?;

    assert_eq!(transactions_summary(&pool).await?.total, 2);
    let remaining = sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM transactions")
      .fetch_one(&pool)
      .await?;
    assert_eq!(remaining, 2);

    Ok(())
  }

  #[tokio::test]
  async fn delete_transactions_exact_summary_rebuilds_post_delete_total_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    for (index, height) in [(50_i32, 50_u32), (51_i32, 51_u32), (52_i32, 52_u32)] {
      seed_typed_tx(
        &pool,
        &TxRow {
          txid: synthetic_hash(b'T', index),
          block_height: height as i32,
          block_pos: 0,
          version: 3,
          tx_type: crate::sql::types::TxTypeDb::Normal,
          lock_time: 0,
          size: 200,
          value_out: 1_000,
          input_count: 1,
          output_count: 1,
          fee: Some(5),
        },
      )
      .await?;
    }
    sqlx::query(
      "INSERT INTO transactions_summary (singleton, total)
       VALUES (TRUE, -999)
       ON CONFLICT (singleton) DO UPDATE SET
         total = EXCLUDED.total",
    )
    .execute(&pool)
    .await?;

    let mut tx = pool.begin().await?;
    super::delete_transactions_exact_summary(&mut tx, 52).await?;
    tx.commit().await?;

    assert_eq!(transactions_summary(&pool).await?.total, 2);
    Ok(())
  }
}
