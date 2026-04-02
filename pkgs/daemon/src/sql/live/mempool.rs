//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL helpers for live mempool summary and recent transaction state.

use sqlx::PgPool;

use crate::error::DaemonError;

/// Current mempool summary snapshot stored in PostgreSQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolSummaryRow {
  pub tx_count: i64,
  pub total_bytes: i64,
  pub usage_bytes: Option<i64>,
  pub max_bytes: Option<i64>,
  pub updated_at: i64,
}

/// One recent mempool transaction snapshot stored in PostgreSQL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolRecentTxRow {
  pub txid: Vec<u8>,
  pub raw_tx: Vec<u8>,
  pub first_seen_at: i64,
  pub last_seen_at: i64,
}

/// One normalized mempool transaction detail row.
#[derive(Debug, Clone, PartialEq)]
pub struct MempoolTxDetailRow {
  pub txid: Vec<u8>,
  pub version: i16,
  pub tx_type: String,
  pub lock_time: i32,
  pub size: i32,
  pub value_out: i64,
  pub input_count: i32,
  pub output_count: i32,
  pub fee: Option<i64>,
  pub payload: Option<serde_json::Value>,
  pub first_seen_at: i64,
  pub last_seen_at: i64,
}

/// One normalized mempool transaction input row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolTxInputRow {
  pub txid: Vec<u8>,
  pub vin_index: i32,
  pub sequence: i64,
  pub prev_txid: Option<Vec<u8>>,
  pub prev_vout: Option<i32>,
  pub coinbase: Option<Vec<u8>>,
  pub script_sig: Vec<u8>,
  pub prev_address: Option<String>,
  pub prev_value: Option<i64>,
}

/// One normalized mempool transaction output row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolTxOutputRow {
  pub txid: Vec<u8>,
  pub vout_index: i32,
  pub value: i64,
  pub script_pubkey: Vec<u8>,
  pub address: Option<String>,
}

/// One address-to-mempool transaction membership row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MempoolAddressTxRow {
  pub address: String,
  pub txid: Vec<u8>,
  pub last_seen_at: i64,
}

/// Upsert the singleton mempool summary row.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the write.
pub async fn upsert_summary(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  summary: &MempoolSummaryRow,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO mempool_summary
       (singleton_key, tx_count, total_bytes, usage_bytes, max_bytes, updated_at)
     VALUES (TRUE, $1, $2, $3, $4, $5)
     ON CONFLICT (singleton_key) DO UPDATE SET
       tx_count = EXCLUDED.tx_count,
       total_bytes = EXCLUDED.total_bytes,
       usage_bytes = EXCLUDED.usage_bytes,
       max_bytes = EXCLUDED.max_bytes,
       updated_at = EXCLUDED.updated_at",
  )
  .bind(summary.tx_count)
  .bind(summary.total_bytes)
  .bind(summary.usage_bytes)
  .bind(summary.max_bytes)
  .bind(summary.updated_at)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("upsert mempool summary: {e}")))?;
  Ok(())
}

/// Upsert one recent mempool transaction row.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the write.
pub async fn upsert_recent_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  row: &MempoolRecentTxRow,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO mempool_recent_txs
       (txid, raw_tx, first_seen_at, last_seen_at)
     VALUES ($1, $2, $3, $4)
     ON CONFLICT (txid) DO UPDATE SET
       raw_tx = EXCLUDED.raw_tx,
       last_seen_at = EXCLUDED.last_seen_at",
  )
  .bind(&row.txid)
  .bind(&row.raw_tx)
  .bind(row.first_seen_at)
  .bind(row.last_seen_at)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("upsert recent mempool tx: {e}")))?;
  Ok(())
}

/// Delete one recent mempool transaction row by txid.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the delete.
pub async fn delete_recent_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  txid: &[u8],
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM mempool_recent_txs WHERE txid = $1")
    .bind(txid)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("delete recent mempool tx: {e}")))?;
  Ok(())
}

/// Upsert one normalized mempool transaction and its dependent rows.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the write.
pub async fn upsert_tx_detail(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  detail: &MempoolTxDetailRow,
  inputs: &[MempoolTxInputRow],
  outputs: &[MempoolTxOutputRow],
  address_rows: &[MempoolAddressTxRow],
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO mempool_tx_details
       (txid, version, tx_type, lock_time, size, value_out, input_count, output_count, fee, payload, first_seen_at, last_seen_at)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
     ON CONFLICT (txid) DO UPDATE SET
       version = EXCLUDED.version,
       tx_type = EXCLUDED.tx_type,
       lock_time = EXCLUDED.lock_time,
       size = EXCLUDED.size,
       value_out = EXCLUDED.value_out,
       input_count = EXCLUDED.input_count,
       output_count = EXCLUDED.output_count,
       fee = EXCLUDED.fee,
       payload = EXCLUDED.payload,
       first_seen_at = LEAST(mempool_tx_details.first_seen_at, EXCLUDED.first_seen_at),
       last_seen_at = EXCLUDED.last_seen_at",
  )
  .bind(&detail.txid)
  .bind(detail.version)
  .bind(&detail.tx_type)
  .bind(detail.lock_time)
  .bind(detail.size)
  .bind(detail.value_out)
  .bind(detail.input_count)
  .bind(detail.output_count)
  .bind(detail.fee)
  .bind(&detail.payload)
  .bind(detail.first_seen_at)
  .bind(detail.last_seen_at)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("upsert mempool tx detail: {e}")))?;

  sqlx::query("DELETE FROM mempool_tx_inputs WHERE txid = $1")
    .bind(&detail.txid)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear mempool tx inputs: {e}")))?;
  sqlx::query("DELETE FROM mempool_tx_outputs WHERE txid = $1")
    .bind(&detail.txid)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear mempool tx outputs: {e}")))?;
  sqlx::query("DELETE FROM mempool_address_txs WHERE txid = $1")
    .bind(&detail.txid)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear mempool address txs: {e}")))?;

  if !inputs.is_empty() {
    sqlx::query(
      "INSERT INTO mempool_tx_inputs
         (txid, vin_index, sequence, prev_txid, prev_vout, coinbase, script_sig, prev_address, prev_value)
       SELECT * FROM unnest(
         $1::bytea[], $2::int[], $3::bigint[], $4::bytea[],
         $5::int[], $6::bytea[], $7::bytea[], $8::text[], $9::bigint[]
       )",
    )
    .bind(inputs.iter().map(|row| row.txid.clone()).collect::<Vec<_>>())
    .bind(inputs.iter().map(|row| row.vin_index).collect::<Vec<_>>())
    .bind(inputs.iter().map(|row| row.sequence).collect::<Vec<_>>())
    .bind(inputs.iter().map(|row| row.prev_txid.clone()).collect::<Vec<_>>())
    .bind(inputs.iter().map(|row| row.prev_vout).collect::<Vec<_>>())
    .bind(inputs.iter().map(|row| row.coinbase.clone()).collect::<Vec<_>>())
    .bind(inputs.iter().map(|row| row.script_sig.clone()).collect::<Vec<_>>())
    .bind(inputs.iter().map(|row| row.prev_address.clone()).collect::<Vec<_>>())
    .bind(inputs.iter().map(|row| row.prev_value).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("insert mempool tx inputs: {e}")))?;
  }

  if !outputs.is_empty() {
    sqlx::query(
      "INSERT INTO mempool_tx_outputs
         (txid, vout_index, value, script_pubkey, address)
       SELECT * FROM unnest(
         $1::bytea[], $2::int[], $3::bigint[], $4::bytea[], $5::text[]
       )",
    )
    .bind(
      outputs
        .iter()
        .map(|row| row.txid.clone())
        .collect::<Vec<_>>(),
    )
    .bind(outputs.iter().map(|row| row.vout_index).collect::<Vec<_>>())
    .bind(outputs.iter().map(|row| row.value).collect::<Vec<_>>())
    .bind(
      outputs
        .iter()
        .map(|row| row.script_pubkey.clone())
        .collect::<Vec<_>>(),
    )
    .bind(
      outputs
        .iter()
        .map(|row| row.address.clone())
        .collect::<Vec<_>>(),
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("insert mempool tx outputs: {e}")))?;
  }

  if !address_rows.is_empty() {
    sqlx::query(
      "INSERT INTO mempool_address_txs (address, txid, last_seen_at)
       SELECT * FROM unnest($1::text[], $2::bytea[], $3::bigint[])",
    )
    .bind(
      address_rows
        .iter()
        .map(|row| row.address.clone())
        .collect::<Vec<_>>(),
    )
    .bind(
      address_rows
        .iter()
        .map(|row| row.txid.clone())
        .collect::<Vec<_>>(),
    )
    .bind(
      address_rows
        .iter()
        .map(|row| row.last_seen_at)
        .collect::<Vec<_>>(),
    )
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("insert mempool address txs: {e}")))?;
  }

  Ok(())
}

/// Delete one normalized mempool transaction and its dependent rows by txid.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the delete.
pub async fn delete_tx_detail(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  txid: &[u8],
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM mempool_tx_details WHERE txid = $1")
    .bind(txid)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("delete mempool tx detail: {e}")))?;
  Ok(())
}

/// Delete the full persisted recent mempool snapshot.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the delete.
pub async fn clear_recent_snapshot(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM mempool_address_txs")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear mempool address txs: {e}")))?;
  sqlx::query("DELETE FROM mempool_tx_outputs")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear mempool tx outputs: {e}")))?;
  sqlx::query("DELETE FROM mempool_tx_inputs")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear mempool tx inputs: {e}")))?;
  sqlx::query("DELETE FROM mempool_tx_details")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear mempool tx details: {e}")))?;
  sqlx::query("DELETE FROM mempool_recent_txs")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear recent mempool txs: {e}")))?;
  Ok(())
}

/// Keep only the newest `keep` recent mempool transactions by `last_seen_at`.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the delete.
pub async fn prune_recent_txs(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  keep: i64,
) -> Result<(), DaemonError> {
  sqlx::query(
    "DELETE FROM mempool_recent_txs
     WHERE txid IN (
       SELECT txid
       FROM mempool_recent_txs
       ORDER BY last_seen_at DESC
       OFFSET $1
     )",
  )
  .bind(keep)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("prune recent mempool txs: {e}")))?;
  Ok(())
}

/// Read the current mempool summary snapshot.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn summary(pool: &PgPool) -> Result<Option<MempoolSummaryRow>, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query(
    "SELECT tx_count, total_bytes, usage_bytes, max_bytes, updated_at
     FROM mempool_summary
     WHERE singleton_key = TRUE",
  )
  .fetch_optional(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read mempool summary: {e}")))?;

  Ok(row.map(|row| MempoolSummaryRow {
    tx_count: row.get("tx_count"),
    total_bytes: row.get("total_bytes"),
    usage_bytes: row.get("usage_bytes"),
    max_bytes: row.get("max_bytes"),
    updated_at: row.get("updated_at"),
  }))
}

/// Read the newest recent mempool transactions ordered by `last_seen_at`.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn recent_txs(pool: &PgPool, limit: i64) -> Result<Vec<MempoolRecentTxRow>, DaemonError> {
  use sqlx::Row;

  let rows = sqlx::query(
    "SELECT txid, raw_tx, first_seen_at, last_seen_at
     FROM mempool_recent_txs
     ORDER BY last_seen_at DESC
     LIMIT $1",
  )
  .bind(limit)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read recent mempool txs: {e}")))?;

  Ok(
    rows
      .iter()
      .map(|row| MempoolRecentTxRow {
        txid: row.get("txid"),
        raw_tx: row.get("raw_tx"),
        first_seen_at: row.get("first_seen_at"),
        last_seen_at: row.get("last_seen_at"),
      })
      .collect(),
  )
}

/// Read one recent mempool transaction by txid.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn recent_tx_by_txid(
  pool: &PgPool,
  txid: &[u8],
) -> Result<Option<MempoolRecentTxRow>, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query(
    "SELECT txid, raw_tx, first_seen_at, last_seen_at
     FROM mempool_recent_txs
     WHERE txid = $1",
  )
  .bind(txid)
  .fetch_optional(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read recent mempool tx: {e}")))?;

  Ok(row.map(|row| MempoolRecentTxRow {
    txid: row.get("txid"),
    raw_tx: row.get("raw_tx"),
    first_seen_at: row.get("first_seen_at"),
    last_seen_at: row.get("last_seen_at"),
  }))
}

/// Read one normalized mempool transaction detail by txid.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn tx_detail(
  pool: &PgPool,
  txid: &[u8],
) -> Result<Option<MempoolTxDetailRow>, DaemonError> {
  use sqlx::Row;

  let row = sqlx::query(
    "SELECT txid, version, tx_type, lock_time, size, value_out, input_count, output_count, fee, payload, first_seen_at, last_seen_at
     FROM mempool_tx_details
     WHERE txid = $1",
  )
  .bind(txid)
  .fetch_optional(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read mempool tx detail: {e}")))?;

  Ok(row.map(|row| MempoolTxDetailRow {
    txid: row.get("txid"),
    version: row.get("version"),
    tx_type: row.get("tx_type"),
    lock_time: row.get("lock_time"),
    size: row.get("size"),
    value_out: row.get("value_out"),
    input_count: row.get("input_count"),
    output_count: row.get("output_count"),
    fee: row.get("fee"),
    payload: row.get("payload"),
    first_seen_at: row.get("first_seen_at"),
    last_seen_at: row.get("last_seen_at"),
  }))
}

/// Read newest normalized mempool transaction details ordered by `last_seen_at`.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn recent_tx_details(
  pool: &PgPool,
  limit: i64,
  after: Option<(i64, &[u8])>,
) -> Result<Vec<MempoolTxDetailRow>, DaemonError> {
  use sqlx::Row;

  let rows = match after {
    Some((after_last_seen_at, after_txid)) => {
      sqlx::query(
        "SELECT txid, version, tx_type, lock_time, size, value_out, input_count, output_count, fee, payload, first_seen_at, last_seen_at
         FROM mempool_tx_details
         WHERE last_seen_at < $1
            OR (last_seen_at = $1 AND txid > $2)
         ORDER BY last_seen_at DESC, txid ASC
         LIMIT $3",
      )
      .bind(after_last_seen_at)
      .bind(after_txid)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    None => {
      sqlx::query(
        "SELECT txid, version, tx_type, lock_time, size, value_out, input_count, output_count, fee, payload, first_seen_at, last_seen_at
         FROM mempool_tx_details
         ORDER BY last_seen_at DESC, txid ASC
         LIMIT $1",
      )
      .bind(limit)
      .fetch_all(pool)
      .await
    }
  }
  .map_err(|e| DaemonError::Pg(format!("read recent mempool tx details: {e}")))?;

  Ok(
    rows
      .into_iter()
      .map(|row| MempoolTxDetailRow {
        txid: row.get("txid"),
        version: row.get("version"),
        tx_type: row.get("tx_type"),
        lock_time: row.get("lock_time"),
        size: row.get("size"),
        value_out: row.get("value_out"),
        input_count: row.get("input_count"),
        output_count: row.get("output_count"),
        fee: row.get("fee"),
        payload: row.get("payload"),
        first_seen_at: row.get("first_seen_at"),
        last_seen_at: row.get("last_seen_at"),
      })
      .collect(),
  )
}

/// Read normalized mempool transaction inputs by txid.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn tx_inputs(pool: &PgPool, txid: &[u8]) -> Result<Vec<MempoolTxInputRow>, DaemonError> {
  use sqlx::Row;

  let rows = sqlx::query(
    "SELECT txid, vin_index, sequence, prev_txid, prev_vout, coinbase, script_sig, prev_address, prev_value
     FROM mempool_tx_inputs
     WHERE txid = $1
     ORDER BY vin_index",
  )
  .bind(txid)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read mempool tx inputs: {e}")))?;

  Ok(
    rows
      .into_iter()
      .map(|row| MempoolTxInputRow {
        txid: row.get("txid"),
        vin_index: row.get("vin_index"),
        sequence: row.get("sequence"),
        prev_txid: row.get("prev_txid"),
        prev_vout: row.get("prev_vout"),
        coinbase: row.get("coinbase"),
        script_sig: row.get("script_sig"),
        prev_address: row.get("prev_address"),
        prev_value: row.get("prev_value"),
      })
      .collect(),
  )
}

/// Read normalized mempool transaction outputs by txid.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn tx_outputs(
  pool: &PgPool,
  txid: &[u8],
) -> Result<Vec<MempoolTxOutputRow>, DaemonError> {
  use sqlx::Row;

  let rows = sqlx::query(
    "SELECT txid, vout_index, value, script_pubkey, address
     FROM mempool_tx_outputs
     WHERE txid = $1
     ORDER BY vout_index",
  )
  .bind(txid)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read mempool tx outputs: {e}")))?;

  Ok(
    rows
      .into_iter()
      .map(|row| MempoolTxOutputRow {
        txid: row.get("txid"),
        vout_index: row.get("vout_index"),
        value: row.get("value"),
        script_pubkey: row.get("script_pubkey"),
        address: row.get("address"),
      })
      .collect(),
  )
}

/// Read one page of recent mempool transaction summaries for one address.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
#[allow(dead_code)]
pub async fn address_txs_page(
  pool: &PgPool,
  address: &str,
  limit: i64,
  offset: i64,
) -> Result<Vec<MempoolTxDetailRow>, DaemonError> {
  use sqlx::Row;

  let rows = sqlx::query(
    "SELECT d.txid, d.version, d.tx_type, d.lock_time, d.size, d.value_out, d.input_count, d.output_count, d.fee, d.payload, d.first_seen_at, d.last_seen_at
     FROM mempool_address_txs a
     JOIN mempool_tx_details d ON d.txid = a.txid
     WHERE a.address = $1
     ORDER BY a.last_seen_at DESC, a.txid
     LIMIT $2
     OFFSET $3",
  )
  .bind(address)
  .bind(limit)
  .bind(offset)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read mempool address txs: {e}")))?;

  Ok(
    rows
      .into_iter()
      .map(|row| MempoolTxDetailRow {
        txid: row.get("txid"),
        version: row.get("version"),
        tx_type: row.get("tx_type"),
        lock_time: row.get("lock_time"),
        size: row.get("size"),
        value_out: row.get("value_out"),
        input_count: row.get("input_count"),
        output_count: row.get("output_count"),
        fee: row.get("fee"),
        payload: row.get("payload"),
        first_seen_at: row.get("first_seen_at"),
        last_seen_at: row.get("last_seen_at"),
      })
      .collect(),
  )
}

/// Count recent mempool transactions currently associated with one address.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
#[allow(dead_code)]
pub async fn address_tx_count(pool: &PgPool, address: &str) -> Result<i64, DaemonError> {
  sqlx::query_scalar("SELECT COUNT(*) FROM mempool_address_txs WHERE address = $1")
    .bind(address)
    .fetch_one(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("count mempool address txs: {e}")))
}

#[cfg(test)]
mod tests {
  use sqlx::postgres::PgPoolOptions;
  use sqlx::PgPool;

  use super::{
    address_tx_count, address_txs_page, recent_tx_details, upsert_recent_tx, upsert_tx_detail,
    MempoolAddressTxRow, MempoolRecentTxRow, MempoolTxDetailRow, MempoolTxInputRow,
    MempoolTxOutputRow,
  };

  async fn seed_live_tx(
    pool: &PgPool,
    txid_byte: u8,
    address: &str,
    last_seen_at: i64,
  ) -> Result<(), Box<dyn std::error::Error>> {
    let txid = vec![txid_byte; 32];
    let mut tx = pool.begin().await?;
    upsert_recent_tx(
      &mut tx,
      &MempoolRecentTxRow {
        txid: txid.clone(),
        raw_tx: vec![txid_byte],
        first_seen_at: last_seen_at - 10,
        last_seen_at,
      },
    )
    .await?;
    upsert_tx_detail(
      &mut tx,
      &MempoolTxDetailRow {
        txid: txid.clone(),
        version: 3,
        tx_type: "normal".to_owned(),
        lock_time: 0,
        size: 100 + i32::from(txid_byte),
        value_out: 1_000 + i64::from(txid_byte),
        input_count: 1,
        output_count: 1,
        fee: Some(10 + i64::from(txid_byte)),
        payload: None,
        first_seen_at: last_seen_at - 10,
        last_seen_at,
      },
      &[MempoolTxInputRow {
        txid: txid.clone(),
        vin_index: 0,
        sequence: 1,
        prev_txid: Some(vec![0xAA; 32]),
        prev_vout: Some(0),
        coinbase: None,
        script_sig: vec![0x51],
        prev_address: Some("Xprev".to_owned()),
        prev_value: Some(1_234),
      }],
      &[MempoolTxOutputRow {
        txid: txid.clone(),
        vout_index: 0,
        value: 1_000 + i64::from(txid_byte),
        script_pubkey: vec![0x76, 0xa9],
        address: Some(address.to_owned()),
      }],
      &[MempoolAddressTxRow {
        address: address.to_owned(),
        txid,
        last_seen_at,
      }],
    )
    .await?;
    tx.commit().await?;
    Ok(())
  }

  #[tokio::test]
  async fn persisted_live_readers_page_and_order_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(database_url) = std::env::var_os("HYPHEN_TEST_DATABASE_URL") else {
      return Ok(());
    };
    let pool = PgPoolOptions::new()
      .max_connections(1)
      .connect(
        &database_url
          .into_string()
          .map_err(|_| "invalid database url")?,
      )
      .await?;
    crate::sql::bootstrap::reset(&pool).await?;

    seed_live_tx(&pool, 0x11, "Xlive", 100).await?;
    seed_live_tx(&pool, 0x22, "Xlive", 200).await?;
    seed_live_tx(&pool, 0x33, "Xlive", 150).await?;

    let total = address_tx_count(&pool, "Xlive").await?;
    assert_eq!(total, 3);

    let page = address_txs_page(&pool, "Xlive", 2, 1).await?;
    assert_eq!(page.len(), 2);
    assert_eq!(hex::encode(&page[0].txid), "33".repeat(32));
    assert_eq!(hex::encode(&page[1].txid), "11".repeat(32));

    let recent = recent_tx_details(&pool, 2, None).await?;
    assert_eq!(recent.len(), 2);
    assert_eq!(hex::encode(&recent[0].txid), "22".repeat(32));
    assert_eq!(hex::encode(&recent[1].txid), "33".repeat(32));

    Ok(())
  }
}
