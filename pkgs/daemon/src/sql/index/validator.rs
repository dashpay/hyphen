//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL queries for the validator index.

use serde_json::Value;
use sqlx::PgPool;
use sqlx::Row;
use std::collections::HashMap;

use crate::error::DaemonError;
use crate::tx::ValidatorInsert;

const PUBLIC_VALIDATOR_TYPE_SQL: &str = "CASE
         WHEN lower(regexp_replace(COALESCE(vps.validator_type, validators.validator_type, ''), '[^a-zA-Z0-9]+', '', 'g'))
           IN ('highperformance', 'evo', 'evonode')
           THEN 'evonode'
         WHEN lower(regexp_replace(COALESCE(vps.validator_type, validators.validator_type, ''), '[^a-zA-Z0-9]+', '', 'g')) = 'regular'
           THEN 'regular'
         ELSE 'unknown'
       END";

#[derive(Debug, Clone)]
pub struct ValidatorRow {
  pub pro_tx_hash: Vec<u8>,
  pub collateral_hash: Vec<u8>,
  pub collateral_index: i32,
  #[allow(dead_code)]
  pub collateral_address: Option<String>,
  pub owner_address: Option<String>,
  pub voting_address: Option<String>,
  pub operator_pubkey: Option<Vec<u8>>,
  pub operator_reward: i32,
  pub validator_type: String,
  pub status: String,
  pub addr_core_p2p: Vec<String>,
  pub addr_platform_p2p: Vec<String>,
  pub addr_platform_https: Vec<String>,
  pub payout_address: Option<String>,
  pub registered_height: i32,
  pub last_updated_height: i32,
}

/// One special transaction history entry associated with a validator.
#[derive(Debug, Clone)]
pub struct ValidatorHistoryRow {
  pub txid: Vec<u8>,
  pub tx_type: crate::sql::types::TxTypeDb,
  pub payload: Value,
  pub block_height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ValidatorCountRow {
  pub total: i64,
}

#[derive(Debug, Clone)]
pub struct ValidatorSearchRow {
  pub pro_tx_hash: Vec<u8>,
  pub status: String,
}

fn validator_row(row: sqlx::postgres::PgRow) -> ValidatorRow {
  ValidatorRow {
    pro_tx_hash: row.get("pro_tx_hash"),
    collateral_hash: row.get("collateral_hash"),
    collateral_index: row.get("collateral_index"),
    collateral_address: row.get("collateral_address"),
    owner_address: row.get("owner_address"),
    voting_address: row.get("voting_address"),
    operator_pubkey: row.get("operator_pubkey"),
    operator_reward: row.get("operator_reward"),
    validator_type: row.get("validator_type"),
    status: row.get("status"),
    addr_core_p2p: row.get("addr_core_p2p"),
    addr_platform_p2p: row.get("addr_platform_p2p"),
    addr_platform_https: row.get("addr_platform_https"),
    payout_address: row.get("payout_address"),
    registered_height: row.get("registered_height"),
    last_updated_height: row.get("last_updated_height"),
  }
}

/// Read block timestamps for the selected heights.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn block_times_by_height(
  pool: &PgPool,
  heights: &[i32],
) -> Result<HashMap<i32, i64>, DaemonError> {
  if heights.is_empty() {
    return Ok(HashMap::new());
  }
  let rows = sqlx::query_as::<_, (i32, i64)>(
    "SELECT height, time::bigint
       FROM blocks
      WHERE height = ANY($1::int[])",
  )
  .bind(heights)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read validator block times: {e}")))?;
  Ok(rows.into_iter().collect())
}

/// List validators with optional status and type filters.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn list_validators(
  pool: &PgPool,
  status: Option<&str>,
  validator_type: Option<&str>,
  limit: i64,
  before: Option<(i32, &[u8])>,
) -> Result<Vec<ValidatorRow>, DaemonError> {
  let rows = match (status, validator_type, before) {
    (Some(status), Some(validator_type), Some((before_height, before_pro_tx_hash))) => {
      sqlx::query(&format!(
        "SELECT validators.pro_tx_hash, validators.collateral_hash, validators.collateral_index, validators.collateral_address,
                validators.owner_address, validators.voting_address, validators.operator_pubkey, validators.operator_reward, validators.validator_type, validators.status,
                validators.addr_core_p2p, validators.addr_platform_p2p, validators.addr_platform_https,
                validators.payout_address, validators.registered_height, validators.last_updated_height
         FROM validators
         LEFT JOIN validator_protx_snapshot AS vps USING (pro_tx_hash)
         WHERE status = $1
           AND {PUBLIC_VALIDATOR_TYPE_SQL} = $2
           AND (
             registered_height < $3
             OR (registered_height = $3 AND pro_tx_hash < $4)
           )
         ORDER BY registered_height DESC, pro_tx_hash DESC
         LIMIT $5"
      ))
      .bind(status)
      .bind(validator_type)
      .bind(before_height)
      .bind(before_pro_tx_hash)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    (Some(status), Some(validator_type), None) => {
      sqlx::query(&format!(
        "SELECT validators.pro_tx_hash, validators.collateral_hash, validators.collateral_index, validators.collateral_address,
                validators.owner_address, validators.voting_address, validators.operator_pubkey, validators.operator_reward, validators.validator_type, validators.status,
                validators.addr_core_p2p, validators.addr_platform_p2p, validators.addr_platform_https,
                validators.payout_address, validators.registered_height, validators.last_updated_height
         FROM validators
         LEFT JOIN validator_protx_snapshot AS vps USING (pro_tx_hash)
         WHERE status = $1 AND {PUBLIC_VALIDATOR_TYPE_SQL} = $2
         ORDER BY registered_height DESC, pro_tx_hash DESC
         LIMIT $3"
      ))
      .bind(status)
      .bind(validator_type)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    (Some(status), None, Some((before_height, before_pro_tx_hash))) => {
      sqlx::query(
        "SELECT validators.pro_tx_hash, validators.collateral_hash, validators.collateral_index, validators.collateral_address,
                validators.owner_address, validators.voting_address, validators.operator_pubkey, validators.operator_reward, validators.validator_type, validators.status,
                validators.addr_core_p2p, validators.addr_platform_p2p, validators.addr_platform_https,
                validators.payout_address, validators.registered_height, validators.last_updated_height
         FROM validators
         LEFT JOIN validator_protx_snapshot AS vps USING (pro_tx_hash)
         WHERE status = $1
           AND (
             registered_height < $2
             OR (registered_height = $2 AND pro_tx_hash < $3)
           )
         ORDER BY registered_height DESC, pro_tx_hash DESC
         LIMIT $4",
      )
      .bind(status)
      .bind(before_height)
      .bind(before_pro_tx_hash)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    (Some(status), None, None) => {
      sqlx::query(
        "SELECT validators.pro_tx_hash, validators.collateral_hash, validators.collateral_index, validators.collateral_address,
                validators.owner_address, validators.voting_address, validators.operator_pubkey, validators.operator_reward, validators.validator_type, validators.status,
                validators.addr_core_p2p, validators.addr_platform_p2p, validators.addr_platform_https,
                validators.payout_address, validators.registered_height, validators.last_updated_height
         FROM validators
         LEFT JOIN validator_protx_snapshot AS vps USING (pro_tx_hash)
         WHERE status = $1
         ORDER BY registered_height DESC, pro_tx_hash DESC
         LIMIT $2",
      )
      .bind(status)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    (None, Some(validator_type), Some((before_height, before_pro_tx_hash))) => {
      sqlx::query(&format!(
        "SELECT validators.pro_tx_hash, validators.collateral_hash, validators.collateral_index, validators.collateral_address,
                validators.owner_address, validators.voting_address, validators.operator_pubkey, validators.operator_reward, validators.validator_type, validators.status,
                validators.addr_core_p2p, validators.addr_platform_p2p, validators.addr_platform_https,
                validators.payout_address, validators.registered_height, validators.last_updated_height
         FROM validators
         LEFT JOIN validator_protx_snapshot AS vps USING (pro_tx_hash)
         WHERE {PUBLIC_VALIDATOR_TYPE_SQL} = $1
           AND (
             registered_height < $2
             OR (registered_height = $2 AND pro_tx_hash < $3)
           )
         ORDER BY registered_height DESC, pro_tx_hash DESC
         LIMIT $4"
      ))
      .bind(validator_type)
      .bind(before_height)
      .bind(before_pro_tx_hash)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    (None, Some(validator_type), None) => {
      sqlx::query(&format!(
        "SELECT validators.pro_tx_hash, validators.collateral_hash, validators.collateral_index, validators.collateral_address,
                validators.owner_address, validators.voting_address, validators.operator_pubkey, validators.operator_reward, validators.validator_type, validators.status,
                validators.addr_core_p2p, validators.addr_platform_p2p, validators.addr_platform_https,
                validators.payout_address, validators.registered_height, validators.last_updated_height
         FROM validators
         LEFT JOIN validator_protx_snapshot AS vps USING (pro_tx_hash)
         WHERE {PUBLIC_VALIDATOR_TYPE_SQL} = $1
         ORDER BY registered_height DESC, pro_tx_hash DESC
         LIMIT $2"
      ))
      .bind(validator_type)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    (None, None, Some((before_height, before_pro_tx_hash))) => {
      sqlx::query(
        "SELECT validators.pro_tx_hash, validators.collateral_hash, validators.collateral_index, validators.collateral_address,
                validators.owner_address, validators.voting_address, validators.operator_pubkey, validators.operator_reward, validators.validator_type, validators.status,
                validators.addr_core_p2p, validators.addr_platform_p2p, validators.addr_platform_https,
                validators.payout_address, validators.registered_height, validators.last_updated_height
         FROM validators
         LEFT JOIN validator_protx_snapshot AS vps USING (pro_tx_hash)
         WHERE registered_height < $1
            OR (registered_height = $1 AND pro_tx_hash < $2)
         ORDER BY registered_height DESC, pro_tx_hash DESC
         LIMIT $3",
      )
      .bind(before_height)
      .bind(before_pro_tx_hash)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    (None, None, None) => {
      sqlx::query(
        "SELECT validators.pro_tx_hash, validators.collateral_hash, validators.collateral_index, validators.collateral_address,
                validators.owner_address, validators.voting_address, validators.operator_pubkey, validators.operator_reward, validators.validator_type, validators.status,
                validators.addr_core_p2p, validators.addr_platform_p2p, validators.addr_platform_https,
                validators.payout_address, validators.registered_height, validators.last_updated_height
         FROM validators
         LEFT JOIN validator_protx_snapshot AS vps USING (pro_tx_hash)
         ORDER BY registered_height DESC, pro_tx_hash DESC
         LIMIT $1",
      )
      .bind(limit)
      .fetch_all(pool)
      .await
    }
  }
  .map_err(|e| DaemonError::Pg(format!("list validators: {e}")))?;

  Ok(rows.into_iter().map(validator_row).collect())
}

/// Read one validator by ProTx hash.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn validator(
  pool: &PgPool,
  pro_tx_hash: &[u8],
) -> Result<Option<ValidatorRow>, DaemonError> {
  Ok(
    sqlx::query(
      "SELECT pro_tx_hash, collateral_hash, collateral_index, collateral_address,
              owner_address, voting_address, operator_pubkey, operator_reward, validator_type, status,
              addr_core_p2p, addr_platform_p2p, addr_platform_https,
              payout_address, registered_height, last_updated_height
       FROM validators WHERE pro_tx_hash = $1",
    )
    .bind(pro_tx_hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read validator: {e}")))?
    .map(validator_row),
  )
}

/// Read the special transaction history for one validator.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn validator_history(
  pool: &PgPool,
  pro_tx_hash: &[u8],
) -> Result<Vec<ValidatorHistoryRow>, DaemonError> {
  let rows = sqlx::query(
    "SELECT s.txid, s.tx_type, s.payload, t.block_height
     FROM special_tx_payloads s
     JOIN transactions t ON t.txid = s.txid
     WHERE s.pro_tx_hash = $1
     ORDER BY t.block_height",
  )
  .bind(pro_tx_hash)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read validator history: {e}")))?;

  Ok(
    rows
      .into_iter()
      .map(|row| ValidatorHistoryRow {
        txid: row.get("txid"),
        tx_type: row.get("tx_type"),
        payload: row.get("payload"),
        block_height: row.get("block_height"),
      })
      .collect(),
  )
}

/// Read validator suggestions by canonical ProTx-hash prefix.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn search_validators_by_prefix(
  pool: &PgPool,
  prefix: &str,
  limit: i64,
) -> Result<Vec<ValidatorSearchRow>, DaemonError> {
  let pattern = format!("{prefix}%");
  let rows = sqlx::query(
    "SELECT pro_tx_hash, status
       FROM validators
      WHERE encode(pro_tx_hash, 'hex') LIKE $1
      ORDER BY last_updated_height DESC, registered_height DESC, pro_tx_hash ASC
      LIMIT $2",
  )
  .bind(pattern)
  .bind(limit)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("search validator prefix: {e}")))?;

  Ok(
    rows
      .into_iter()
      .map(|row| ValidatorSearchRow {
        pro_tx_hash: row.get("pro_tx_hash"),
        status: row.get("status"),
      })
      .collect(),
  )
}

struct ValidatorSnapshot {
  collateral_hash: Option<Vec<u8>>,
  collateral_index: Option<i32>,
  collateral_address: Option<String>,
  owner_address: Option<String>,
  operator_pubkey: Option<Vec<u8>>,
  voting_address: Option<String>,
  addr_core_p2p: Option<Vec<String>>,
  addr_platform_p2p: Option<Vec<String>>,
  addr_platform_https: Option<Vec<String>>,
  payout_address: Option<String>,
  operator_reward: Option<i32>,
  validator_type: Option<String>,
  status: Option<String>,
  registered_height: Option<i32>,
  last_updated_height: Option<i32>,
}

async fn snapshot(
  conn: &mut sqlx::PgConnection,
  height: i32,
  sequence: i32,
  pro_tx_hash: &[u8],
) -> Result<(), DaemonError> {
  let row = sqlx::query(
    "SELECT collateral_hash, collateral_index, collateral_address, owner_address, operator_pubkey, \
            voting_address, addr_core_p2p, addr_platform_p2p, addr_platform_https, \
            payout_address, operator_reward, validator_type, status, \
            registered_height, last_updated_height \
     FROM validators WHERE pro_tx_hash = $1",
  )
  .bind(pro_tx_hash)
  .fetch_optional(&mut *conn)
  .await
  .map_err(|e| DaemonError::Pg(format!("snapshot validator: {e}")))?;

  let snapshot = row.as_ref().map(|row| ValidatorSnapshot {
    collateral_hash: row.get("collateral_hash"),
    collateral_index: row.get("collateral_index"),
    collateral_address: row.get("collateral_address"),
    owner_address: row.get("owner_address"),
    operator_pubkey: row.get("operator_pubkey"),
    voting_address: row.get("voting_address"),
    addr_core_p2p: row.get("addr_core_p2p"),
    addr_platform_p2p: row.get("addr_platform_p2p"),
    addr_platform_https: row.get("addr_platform_https"),
    payout_address: row.get("payout_address"),
    operator_reward: row.get("operator_reward"),
    validator_type: row.get("validator_type"),
    status: row.get("status"),
    registered_height: row.get("registered_height"),
    last_updated_height: row.get("last_updated_height"),
  });

  sqlx::query(
    "INSERT INTO validator_history (
       height, sequence, pro_tx_hash, existed,
       collateral_hash, collateral_index, collateral_address, owner_address, operator_pubkey,
       voting_address, addr_core_p2p, addr_platform_p2p, addr_platform_https,
       payout_address, operator_reward, validator_type, status,
       registered_height, last_updated_height
     ) VALUES (
       $1, $2, $3, $4,
       $5, $6, $7, $8, $9,
       $10, $11, $12, $13,
       $14, $15, $16, $17,
       $18, $19
     )",
  )
  .bind(height)
  .bind(sequence)
  .bind(pro_tx_hash)
  .bind(snapshot.is_some())
  .bind(snapshot.as_ref().and_then(|s| s.collateral_hash.as_deref()))
  .bind(snapshot.as_ref().and_then(|s| s.collateral_index))
  .bind(snapshot.as_ref().and_then(|s| s.collateral_address.clone()))
  .bind(snapshot.as_ref().and_then(|s| s.owner_address.clone()))
  .bind(snapshot.as_ref().and_then(|s| s.operator_pubkey.as_deref()))
  .bind(snapshot.as_ref().and_then(|s| s.voting_address.clone()))
  .bind(snapshot.as_ref().and_then(|s| s.addr_core_p2p.clone()))
  .bind(snapshot.as_ref().and_then(|s| s.addr_platform_p2p.clone()))
  .bind(
    snapshot
      .as_ref()
      .and_then(|s| s.addr_platform_https.clone()),
  )
  .bind(snapshot.as_ref().and_then(|s| s.payout_address.clone()))
  .bind(snapshot.as_ref().and_then(|s| s.operator_reward))
  .bind(snapshot.as_ref().and_then(|s| s.validator_type.clone()))
  .bind(snapshot.as_ref().and_then(|s| s.status.clone()))
  .bind(snapshot.as_ref().and_then(|s| s.registered_height))
  .bind(snapshot.as_ref().and_then(|s| s.last_updated_height))
  .execute(conn)
  .await
  .map_err(|e| DaemonError::Pg(format!("insert validator history: {e}")))?;

  Ok(())
}

/// Insert or refresh a validator row from a registration transaction.
///
/// # Errors
///
/// Returns an error when validator snapshotting or PostgreSQL query execution
/// fails.
pub async fn insert_validator(
  conn: &mut sqlx::PgConnection,
  mut v: ValidatorInsert,
  height: i32,
  sequence: i32,
) -> Result<(), DaemonError> {
  if v.collateral_address.is_none() {
    v.collateral_address =
      resolve_collateral_address(conn, &v.pro_tx_hash, &v.collateral_hash, v.collateral_index)
        .await?;
  }
  snapshot(conn, height, sequence, &v.pro_tx_hash).await?;

  sqlx::query(
    "INSERT INTO validators (
       pro_tx_hash, collateral_hash, collateral_index, collateral_address,
       owner_address, operator_pubkey, voting_address, operator_reward,
       validator_type, status, registered_height, last_updated_height,
       addr_core_p2p, addr_platform_p2p, addr_platform_https, payout_address
     ) VALUES (
       $1, $2, $3, $4,
       $5, $6, $7, $8,
       $9, 'registered', $10, $10,
       $11, $12, $13, $14
     )
     ON CONFLICT (pro_tx_hash) DO UPDATE SET
       collateral_hash = EXCLUDED.collateral_hash,
       collateral_index = EXCLUDED.collateral_index,
       collateral_address = EXCLUDED.collateral_address,
       owner_address = EXCLUDED.owner_address,
       operator_pubkey = EXCLUDED.operator_pubkey,
       voting_address = EXCLUDED.voting_address,
       operator_reward = EXCLUDED.operator_reward,
       validator_type = EXCLUDED.validator_type,
       status = EXCLUDED.status,
       registered_height = EXCLUDED.registered_height,
       last_updated_height = EXCLUDED.last_updated_height,
       addr_core_p2p = EXCLUDED.addr_core_p2p,
       addr_platform_p2p = EXCLUDED.addr_platform_p2p,
       addr_platform_https = EXCLUDED.addr_platform_https,
       payout_address = EXCLUDED.payout_address",
  )
  .bind(v.pro_tx_hash.as_slice())
  .bind(v.collateral_hash.as_slice())
  .bind(v.collateral_index)
  .bind(&v.collateral_address)
  .bind(&v.owner_address)
  .bind(&v.operator_pubkey)
  .bind(&v.voting_address)
  .bind(v.operator_reward)
  .bind(&v.validator_type)
  .bind(height)
  .bind(&v.core_addrs)
  .bind(&v.plat_p2p_addrs)
  .bind(&v.plat_https_addrs)
  .bind(&v.payout_addr)
  .execute(conn)
  .await
  .map_err(|e| DaemonError::Pg(format!("insert validator: {e}")))?;
  Ok(())
}

async fn resolve_collateral_address(
  conn: &mut sqlx::PgConnection,
  pro_tx_hash: &[u8],
  collateral_hash: &[u8],
  collateral_index: i32,
) -> Result<Option<String>, DaemonError> {
  if collateral_hash.iter().all(|byte| *byte == 0) {
    return Ok(None);
  }
  let row = sqlx::query(
    "SELECT address
     FROM tx_outputs
     WHERE txid = $1 AND vout_index = $2",
  )
  .bind(collateral_hash)
  .bind(collateral_index)
  .fetch_optional(&mut *conn)
  .await
  .map_err(|e| DaemonError::Pg(format!("resolve collateral address: {e}")))?;

  let Some(row) = row else {
    tracing::debug!(
      validator = %hex::encode(pro_tx_hash),
      collateral_hash = %hex::encode(collateral_hash),
      collateral_index,
      "validator collateral outpoint not found in tx_outputs; leaving collateral address null"
    );
    return Ok(None);
  };
  Ok(row.get("address"))
}

/// Update the validator service address fields.
///
/// # Errors
///
/// Returns an error when validator snapshotting or PostgreSQL query execution
/// fails.
pub async fn update_service(
  conn: &mut sqlx::PgConnection,
  pro_tx_hash: &[u8],
  core_addrs: &[String],
  plat_p2p_addrs: &[String],
  plat_https_addrs: &[String],
  height: i32,
  sequence: i32,
) -> Result<(), DaemonError> {
  snapshot(conn, height, sequence, pro_tx_hash).await?;

  sqlx::query(
    "UPDATE validators SET addr_core_p2p = $1, addr_platform_p2p = $2, \
       addr_platform_https = $3, last_updated_height = $4 \
     WHERE pro_tx_hash = $5",
  )
  .bind(core_addrs)
  .bind(plat_p2p_addrs)
  .bind(plat_https_addrs)
  .bind(height)
  .bind(pro_tx_hash)
  .execute(conn)
  .await
  .map_err(|e| DaemonError::Pg(format!("update validator service: {e}")))?;
  Ok(())
}

/// Update the validator registrar-managed fields.
///
/// # Errors
///
/// Returns an error when validator snapshotting or PostgreSQL query execution
/// fails.
pub async fn update_registrar(
  conn: &mut sqlx::PgConnection,
  pro_tx_hash: &[u8],
  operator_pubkey: &[u8],
  payout_addr: &Option<String>,
  voting_addr: &Option<String>,
  height: i32,
  sequence: i32,
) -> Result<(), DaemonError> {
  snapshot(conn, height, sequence, pro_tx_hash).await?;

  sqlx::query(
    "UPDATE validators SET operator_pubkey = $1, payout_address = $2, \
       voting_address = $3, last_updated_height = $4 \
     WHERE pro_tx_hash = $5",
  )
  .bind(operator_pubkey)
  .bind(payout_addr)
  .bind(voting_addr)
  .bind(height)
  .bind(pro_tx_hash)
  .execute(conn)
  .await
  .map_err(|e| DaemonError::Pg(format!("update validator registrar: {e}")))?;
  Ok(())
}

/// Mark a validator as revoked.
///
/// # Errors
///
/// Returns an error when validator snapshotting or PostgreSQL query execution
/// fails.
pub async fn revoke(
  conn: &mut sqlx::PgConnection,
  pro_tx_hash: &[u8],
  height: i32,
  sequence: i32,
) -> Result<(), DaemonError> {
  snapshot(conn, height, sequence, pro_tx_hash).await?;

  sqlx::query(
    "UPDATE validators SET status = 'revoked', last_updated_height = $1 \
     WHERE pro_tx_hash = $2",
  )
  .bind(height)
  .bind(pro_tx_hash)
  .execute(conn)
  .await
  .map_err(|e| DaemonError::Pg(format!("revoke validator: {e}")))?;
  Ok(())
}

/// Restore validator rows from the history snapshot recorded for one height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn restore_history(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
) -> Result<(), DaemonError> {
  use sqlx::Row;

  let rows = sqlx::query(
    "SELECT sequence, pro_tx_hash, existed, collateral_hash, collateral_index, collateral_address, owner_address,
            operator_pubkey, voting_address, addr_core_p2p, addr_platform_p2p,
            addr_platform_https, payout_address, operator_reward, validator_type,
            status, registered_height, last_updated_height
     FROM validator_history
     WHERE height = $1
     ORDER BY sequence DESC",
  )
  .bind(height as i32)
  .fetch_all(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("read validator history: {e}")))?;

  let mut existing = std::collections::BTreeMap::new();
  let mut deleted = std::collections::BTreeSet::new();
  for row in rows {
    let pro_tx_hash: Vec<u8> = row.get("pro_tx_hash");
    let existed: bool = row.get("existed");
    if existed {
      deleted.remove(&pro_tx_hash);
      existing.insert(
        pro_tx_hash,
        (
          row.get::<Option<Vec<u8>>, _>("collateral_hash"),
          row.get::<Option<i32>, _>("collateral_index"),
          row.get::<Option<String>, _>("collateral_address"),
          row.get::<Option<String>, _>("owner_address"),
          row.get::<Option<Vec<u8>>, _>("operator_pubkey"),
          row.get::<Option<String>, _>("voting_address"),
          row.get::<Option<Vec<String>>, _>("addr_core_p2p"),
          row.get::<Option<Vec<String>>, _>("addr_platform_p2p"),
          row.get::<Option<Vec<String>>, _>("addr_platform_https"),
          row.get::<Option<String>, _>("payout_address"),
          row.get::<Option<i32>, _>("operator_reward"),
          row.get::<Option<String>, _>("validator_type"),
          row.get::<Option<String>, _>("status"),
          row.get::<Option<i32>, _>("registered_height"),
          row.get::<Option<i32>, _>("last_updated_height"),
        ),
      );
    } else if !existing.contains_key(&pro_tx_hash) {
      deleted.insert(pro_tx_hash);
    }
  }
  if !existing.is_empty() {
    let mut query = sqlx::QueryBuilder::<sqlx::Postgres>::new(
      "INSERT INTO validators (
         pro_tx_hash, collateral_hash, collateral_index, collateral_address,
         owner_address, operator_pubkey, voting_address, addr_core_p2p,
         addr_platform_p2p, addr_platform_https, payout_address,
         operator_reward, validator_type, status, registered_height,
         last_updated_height
       ) ",
    );
    query.push_values(existing.iter(), |mut builder, (pro_tx_hash, row)| {
      builder
        .push_bind(pro_tx_hash)
        .push_bind(&row.0)
        .push_bind(row.1)
        .push_bind(&row.2)
        .push_bind(&row.3)
        .push_bind(&row.4)
        .push_bind(&row.5)
        .push_bind(&row.6)
        .push_bind(&row.7)
        .push_bind(&row.8)
        .push_bind(&row.9)
        .push_bind(row.10)
        .push_bind(&row.11)
        .push_bind(&row.12)
        .push_bind(row.13)
        .push_bind(row.14);
    });
    query.push(
      " ON CONFLICT (pro_tx_hash) DO UPDATE SET
          collateral_hash = EXCLUDED.collateral_hash,
          collateral_index = EXCLUDED.collateral_index,
          collateral_address = EXCLUDED.collateral_address,
          owner_address = EXCLUDED.owner_address,
          operator_pubkey = EXCLUDED.operator_pubkey,
          voting_address = EXCLUDED.voting_address,
          addr_core_p2p = EXCLUDED.addr_core_p2p,
          addr_platform_p2p = EXCLUDED.addr_platform_p2p,
          addr_platform_https = EXCLUDED.addr_platform_https,
          payout_address = EXCLUDED.payout_address,
          operator_reward = EXCLUDED.operator_reward,
          validator_type = EXCLUDED.validator_type,
          status = EXCLUDED.status,
          registered_height = EXCLUDED.registered_height,
          last_updated_height = EXCLUDED.last_updated_height",
    );
    query
      .build()
      .execute(&mut **tx)
      .await
      .map_err(|e| DaemonError::Pg(format!("restore validator rows: {e}")))?;
  }
  if !deleted.is_empty() {
    sqlx::query("DELETE FROM validators WHERE pro_tx_hash = ANY($1::bytea[])")
      .bind(deleted.into_iter().collect::<Vec<_>>())
      .execute(&mut **tx)
      .await
      .map_err(|e| DaemonError::Pg(format!("delete validator rows: {e}")))?;
  }

  sqlx::query("DELETE FROM validator_history WHERE height = $1")
    .bind(height as i32)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("delete validator history: {e}")))?;
  Ok(())
}

/// Rebuild validator count snapshots for the supported filter combinations.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn refresh_validator_counts(conn: &mut sqlx::PgConnection) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM validator_counts")
    .execute(&mut *conn)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear validator counts: {e}")))?;

  sqlx::query(
    "INSERT INTO validator_counts (status, validator_type, total)
     SELECT COALESCE(status, '') AS status,
            COALESCE(validator_type, '') AS validator_type,
            COUNT(*)::bigint AS total
       FROM validators
      GROUP BY GROUPING SETS (
        (),
        (status),
        (validator_type),
        (status, validator_type)
      )",
  )
  .execute(&mut *conn)
  .await
  .map_err(|e| DaemonError::Pg(format!("refresh validator counts: {e}")))?;
  Ok(())
}

/// Rebuild validator count snapshots inside a transaction.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn refresh_validator_counts_tx(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM validator_counts")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear validator counts: {e}")))?;

  sqlx::query(
    "INSERT INTO validator_counts (status, validator_type, total)
     SELECT COALESCE(status, '') AS status,
            COALESCE(validator_type, '') AS validator_type,
            COUNT(*)::bigint AS total
       FROM validators
      GROUP BY GROUPING SETS (
        (),
        (status),
        (validator_type),
        (status, validator_type)
      )",
  )
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("refresh validator counts: {e}")))?;
  Ok(())
}

/// Read one validator count snapshot for the current filter combination.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn validator_count(
  pool: &PgPool,
  status: Option<&str>,
  validator_type: Option<&str>,
) -> Result<ValidatorCountRow, DaemonError> {
  let total = sqlx::query_scalar::<_, i64>(&format!(
    "SELECT COUNT(*)::bigint
       FROM validators
       LEFT JOIN validator_protx_snapshot AS vps USING (pro_tx_hash)
      WHERE ($1::text IS NULL OR status = $1)
        AND ($2::text IS NULL OR {PUBLIC_VALIDATOR_TYPE_SQL} = $2)"
  ))
  .bind(status)
  .bind(validator_type)
  .fetch_one(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read validator count: {e}")))?;
  Ok(ValidatorCountRow { total })
}

#[cfg(test)]
mod tests {
  use sqlx::PgPool;
  use sqlx::Row;

  use super::{
    insert_validator, list_validators, refresh_validator_counts_tx, restore_history, revoke,
    update_registrar, update_service, validator, validator_count, validator_history,
    ValidatorInsert,
  };

  /// Build one representative validator registration payload for route and restore tests.
  fn sample_validator_insert() -> ValidatorInsert {
    ValidatorInsert {
      pro_tx_hash: vec![0x11; 32],
      collateral_hash: vec![0x22; 32],
      collateral_index: 1,
      collateral_address: Some("Xcollateral111111111111111111111111".to_owned()),
      operator_pubkey: vec![0x33; 48],
      owner_address: Some("Xowner11111111111111111111111111111".to_owned()),
      voting_address: Some("Xvote111111111111111111111111111111".to_owned()),
      operator_reward: 5_000,
      validator_type: "regular".to_owned(),
      payout_addr: Some("Xpayout111111111111111111111111111".to_owned()),
      core_addrs: vec!["1.2.3.4:9999".to_owned()],
      plat_p2p_addrs: vec!["5.6.7.8:26656".to_owned()],
      plat_https_addrs: vec!["5.6.7.8:443".to_owned()],
    }
  }

  /// Seed one collateral output because validator registration resolves collateral addresses from tx outputs.
  async fn seed_collateral_output(
    pool: &PgPool,
    txid: &[u8],
    vout_index: i32,
    address: &str,
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO blocks (
         height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count
       ) VALUES (0, $1, $2, $3, 0, 0, 0, 1, 80, 1)",
    )
    .bind(vec![0xaa_u8; 32])
    .bind(vec![0xbb_u8; 32])
    .bind(vec![0xcc_u8; 32])
    .execute(pool)
    .await?;
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size,
         value_out, input_count, output_count
       ) VALUES ($1, 0, 0, 1, 'normal', 0, 100, 1000, 1, 1)",
    )
    .bind(txid)
    .execute(pool)
    .await?;
    sqlx::query(
      "INSERT INTO tx_outputs (
         txid, vout_index, block_height, value, script_pubkey, address, address_type
       ) VALUES ($1, $2, 0, 1000, $3, $4, 'p2pkh')",
    )
    .bind(txid)
    .bind(vout_index)
    .bind(vec![0x76_u8, 0xa9, 0x14])
    .bind(address)
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one transaction row because validator history joins payloads back to transaction heights.
  async fn seed_transaction(
    pool: &PgPool,
    txid: &[u8],
    block_height: i32,
    block_pos: i32,
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size,
         value_out, input_count, output_count
       ) VALUES ($1, $2, $3, 3, 'provider_register', 0, 100, 1000, 1, 1)",
    )
    .bind(txid)
    .bind(block_height)
    .bind(block_pos)
    .execute(pool)
    .await?;
    Ok(())
  }

  /// Seed one validator payload row because the history route reads special transaction payloads directly.
  async fn seed_validator_payload(
    pool: &PgPool,
    txid: &[u8],
    pro_tx_hash: &[u8],
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO special_tx_payloads (txid, tx_type, pro_tx_hash, payload)
       VALUES ($1, 'provider_register', $2, $3)",
    )
    .bind(txid)
    .bind(pro_tx_hash)
    .bind(serde_json::json!({ "version": 1 }))
    .execute(pool)
    .await?;
    Ok(())
  }

  #[tokio::test]
  async fn validator_insert_disconnect_round_trip_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;
    seed_collateral_output(&pool, &[0x22; 32], 1, "Xcollateral111111111111111111111111").await?;

    let mut conn = pool.acquire().await?;
    insert_validator(&mut conn, sample_validator_insert(), 0, 0).await?;

    let validator_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM validators")
      .fetch_one(&pool)
      .await?;
    let history_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM validator_history")
      .fetch_one(&pool)
      .await?;
    assert_eq!(validator_count, 1);
    assert_eq!(history_count, 1);

    let mut tx = pool.begin().await?;
    restore_history(&mut tx, 0).await?;
    tx.commit().await?;

    let validator_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM validators")
      .fetch_one(&pool)
      .await?;
    let history_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM validator_history")
      .fetch_one(&pool)
      .await?;
    assert_eq!(validator_count, 0);
    assert_eq!(history_count, 0);

    Ok(())
  }

  #[tokio::test]
  async fn validator_history_restores_previous_state_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;
    seed_collateral_output(&pool, &[0x22; 32], 1, "Xcollateral111111111111111111111111").await?;

    let mut conn = pool.acquire().await?;
    let insert = sample_validator_insert();
    let pro_tx_hash = insert.pro_tx_hash.clone();

    insert_validator(&mut conn, insert, 0, 0).await?;
    update_service(
      &mut conn,
      &pro_tx_hash,
      &["9.9.9.9:9999".to_owned()],
      &["9.9.9.9:26656".to_owned()],
      &["9.9.9.9:443".to_owned()],
      1,
      0,
    )
    .await?;
    update_registrar(
      &mut conn,
      &pro_tx_hash,
      &[0x44; 48],
      &Some("Xpayout222222222222222222222222222".to_owned()),
      &Some("Xvote22222222222222222222222222222".to_owned()),
      2,
      0,
    )
    .await?;
    revoke(&mut conn, &pro_tx_hash, 3, 0).await?;
    drop(conn);

    let row = sqlx::query(
      "SELECT status, voting_address, payout_address, addr_core_p2p, last_updated_height
       FROM validators
       WHERE pro_tx_hash = $1",
    )
    .bind(&pro_tx_hash)
    .fetch_one(&pool)
    .await?;
    assert_eq!(row.get::<String, _>("status"), "revoked");
    assert_eq!(
      row.get::<Option<String>, _>("voting_address").as_deref(),
      Some("Xvote22222222222222222222222222222")
    );
    assert_eq!(
      row.get::<Option<String>, _>("payout_address").as_deref(),
      Some("Xpayout222222222222222222222222222")
    );
    assert_eq!(
      row.get::<Vec<String>, _>("addr_core_p2p"),
      vec!["9.9.9.9:9999"]
    );
    assert_eq!(row.get::<i32, _>("last_updated_height"), 3);

    let mut tx = pool.begin().await?;
    restore_history(&mut tx, 3).await?;
    tx.commit().await?;

    let row = sqlx::query(
      "SELECT status, voting_address, payout_address, addr_core_p2p, last_updated_height
       FROM validators
       WHERE pro_tx_hash = $1",
    )
    .bind(&pro_tx_hash)
    .fetch_one(&pool)
    .await?;
    assert_eq!(row.get::<String, _>("status"), "registered");
    assert_eq!(
      row.get::<Option<String>, _>("voting_address").as_deref(),
      Some("Xvote22222222222222222222222222222")
    );
    assert_eq!(
      row.get::<Option<String>, _>("payout_address").as_deref(),
      Some("Xpayout222222222222222222222222222")
    );
    assert_eq!(
      row.get::<Vec<String>, _>("addr_core_p2p"),
      vec!["9.9.9.9:9999"]
    );
    assert_eq!(row.get::<i32, _>("last_updated_height"), 2);

    let history_count: i64 =
      sqlx::query_scalar("SELECT COUNT(*) FROM validator_history WHERE height = 3")
        .fetch_one(&pool)
        .await?;
    assert_eq!(history_count, 0);

    Ok(())
  }

  #[tokio::test]
  async fn validator_insert_derives_collateral_address_when_missing(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;
    seed_collateral_output(&pool, &[0x22; 32], 1, "Xderived111111111111111111111111111").await?;

    let mut insert = sample_validator_insert();
    insert.collateral_address = None;

    let mut conn = pool.acquire().await?;
    insert_validator(&mut conn, insert.clone(), 0, 0).await?;
    drop(conn);

    let row = sqlx::query("SELECT collateral_address FROM validators WHERE pro_tx_hash = $1")
      .bind(&insert.pro_tx_hash)
      .fetch_one(&pool)
      .await?;
    assert_eq!(
      row
        .get::<Option<String>, _>("collateral_address")
        .as_deref(),
      Some("Xderived111111111111111111111111111")
    );

    Ok(())
  }

  #[tokio::test]
  async fn validator_insert_keeps_null_collateral_address_for_null_outpoint(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut insert = sample_validator_insert();
    insert.collateral_hash = vec![0; 32];
    insert.collateral_address = None;

    let mut conn = pool.acquire().await?;
    insert_validator(&mut conn, insert.clone(), 0, 0).await?;
    drop(conn);

    let row = sqlx::query("SELECT collateral_address FROM validators WHERE pro_tx_hash = $1")
      .bind(&insert.pro_tx_hash)
      .fetch_one(&pool)
      .await?;
    assert_eq!(row.get::<Option<String>, _>("collateral_address"), None);

    Ok(())
  }

  #[tokio::test]
  async fn validator_insert_keeps_null_collateral_address_for_missing_outpoint(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut insert = sample_validator_insert();
    insert.collateral_address = None;

    let mut conn = pool.acquire().await?;
    insert_validator(&mut conn, insert.clone(), 0, 0).await?;
    drop(conn);

    let row = sqlx::query("SELECT collateral_address FROM validators WHERE pro_tx_hash = $1")
      .bind(&insert.pro_tx_hash)
      .fetch_one(&pool)
      .await?;
    assert_eq!(row.get::<Option<String>, _>("collateral_address"), None);

    Ok(())
  }

  #[tokio::test]
  async fn validator_and_list_routes_read_seeded_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;
    seed_collateral_output(&pool, &[0x22; 32], 1, "Xcollateral111111111111111111111111").await?;

    let mut first = sample_validator_insert();
    first.pro_tx_hash = vec![0x11; 32];
    first.validator_type = "regular".to_owned();

    let mut second = sample_validator_insert();
    second.pro_tx_hash = vec![0x44; 32];
    second.validator_type = "regular".to_owned();
    second.collateral_hash = vec![0x55; 32];
    second.collateral_address = Some("Xcollateral222222222222222222222222".to_owned());

    let mut conn = pool.acquire().await?;
    insert_validator(&mut conn, first.clone(), 10, 0).await?;
    insert_validator(&mut conn, second.clone(), 11, 0).await?;
    drop(conn);

    let row = validator(&pool, second.pro_tx_hash.as_slice()).await?;
    assert!(row.is_some());
    let row = row.expect("seeded validator should exist");
    assert_eq!(row.registered_height, 11);
    assert_eq!(row.status, "registered");

    let first_page = list_validators(&pool, Some("registered"), Some("regular"), 1, None).await?;
    assert_eq!(first_page.len(), 1);
    assert_eq!(first_page[0].pro_tx_hash, second.pro_tx_hash);

    let later_page = list_validators(
      &pool,
      Some("registered"),
      Some("regular"),
      1,
      Some((
        first_page[0].registered_height,
        first_page[0].pro_tx_hash.as_slice(),
      )),
    )
    .await?;
    assert_eq!(later_page.len(), 1);
    assert_eq!(later_page[0].pro_tx_hash, first.pro_tx_hash);

    Ok(())
  }

  #[tokio::test]
  async fn validator_list_matches_canonical_type_filter_against_legacy_variants(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut first = sample_validator_insert();
    first.pro_tx_hash = vec![0xaa; 32];
    first.validator_type = "HighPerformance".to_owned();
    first.collateral_hash = vec![0xbb; 32];
    first.collateral_address = Some("Xcollateral333333333333333333333333".to_owned());

    let mut second = sample_validator_insert();
    second.pro_tx_hash = vec![0xcc; 32];
    second.validator_type = "Regular".to_owned();
    second.collateral_hash = vec![0xdd; 32];
    second.collateral_address = Some("Xcollateral444444444444444444444444".to_owned());

    let mut conn = pool.acquire().await?;
    insert_validator(&mut conn, first.clone(), 10, 0).await?;
    insert_validator(&mut conn, second.clone(), 11, 0).await?;
    drop(conn);

    let evonodes = list_validators(&pool, None, Some("evonode"), 10, None).await?;
    assert_eq!(evonodes.len(), 1);
    assert_eq!(evonodes[0].pro_tx_hash, first.pro_tx_hash);

    let regular = list_validators(&pool, None, Some("regular"), 10, None).await?;
    assert_eq!(regular.len(), 1);
    assert_eq!(regular[0].pro_tx_hash, second.pro_tx_hash);

    Ok(())
  }

  #[tokio::test]
  async fn validator_list_prefers_live_type_filter_when_overlay_snapshot_exists(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut validator_row = sample_validator_insert();
    validator_row.pro_tx_hash = vec![0xaa; 32];
    validator_row.validator_type = "Regular".to_owned();
    validator_row.collateral_hash = vec![0xbb; 32];
    validator_row.collateral_address = Some("Xcollateral555555555555555555555555".to_owned());

    let mut conn = pool.acquire().await?;
    insert_validator(&mut conn, validator_row.clone(), 10, 0).await?;
    drop(conn);

    let mut tx = pool.begin().await?;
    crate::sql::live::protx::replace_snapshot(
      &mut tx,
      &[crate::sql::live::protx::ValidatorProTxSnapshotRow {
        pro_tx_hash: validator_row.pro_tx_hash.clone(),
        validator_type: Some("Evo".to_owned()),
        current_status: Some("valid".to_owned()),
        last_paid_height: Some(99),
        consecutive_payments: None,
        pose_penalty: None,
        pose_revived_height: None,
        pose_ban_height: None,
        revocation_reason: None,
        updated_at: 100,
      }],
    )
    .await?;
    tx.commit().await?;

    let evonodes = list_validators(&pool, None, Some("evonode"), 10, None).await?;
    assert_eq!(evonodes.len(), 1);
    assert_eq!(evonodes[0].pro_tx_hash, validator_row.pro_tx_hash);

    let regular = list_validators(&pool, None, Some("regular"), 10, None).await?;
    assert!(regular.is_empty());

    let count = validator_count(&pool, None, Some("evonode")).await?;
    assert_eq!(count.total, 1);
    Ok(())
  }

  #[tokio::test]
  async fn validator_history_reads_seeded_payload_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let pro_tx_hash = vec![0x77; 32];
    let first_txid = vec![0x88; 32];
    let second_txid = vec![0x99; 32];
    seed_transaction(&pool, &first_txid, 7, 0).await?;
    seed_transaction(&pool, &second_txid, 8, 0).await?;
    seed_validator_payload(&pool, &first_txid, &pro_tx_hash).await?;
    seed_validator_payload(&pool, &second_txid, &pro_tx_hash).await?;

    let rows = validator_history(&pool, pro_tx_hash.as_slice()).await?;
    assert_eq!(rows.len(), 2);
    assert_eq!(rows[0].block_height, 7);
    assert_eq!(rows[1].block_height, 8);
    assert_eq!(rows[0].txid, first_txid);
    assert_eq!(rows[1].txid, second_txid);

    Ok(())
  }

  #[tokio::test]
  async fn validator_count_reads_filter_snapshots_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut first = sample_validator_insert();
    first.validator_type = "regular".to_owned();
    first.collateral_hash = vec![0x44; 32];
    first.collateral_address = Some("Xcollateral111111111111111111111111".to_owned());

    let mut second = sample_validator_insert();
    second.pro_tx_hash = vec![0x55; 32];
    second.collateral_hash = vec![0x66; 32];
    second.collateral_address = Some("Xcollateral222222222222222222222222".to_owned());
    second.validator_type = "evonode".to_owned();

    let mut conn = pool.acquire().await?;
    insert_validator(&mut conn, first.clone(), 10, 0).await?;
    insert_validator(&mut conn, second.clone(), 11, 0).await?;
    drop(conn);

    let mut tx = pool.begin().await?;
    sqlx::query("UPDATE validators SET status = 'revoked' WHERE pro_tx_hash = $1")
      .bind(&second.pro_tx_hash)
      .execute(&mut *tx)
      .await?;
    refresh_validator_counts_tx(&mut tx).await?;
    tx.commit().await?;

    assert_eq!(validator_count(&pool, None, None).await?.total, 2);
    assert_eq!(
      validator_count(&pool, Some("registered"), None)
        .await?
        .total,
      1
    );
    assert_eq!(
      validator_count(&pool, Some("revoked"), None).await?.total,
      1
    );
    assert_eq!(
      validator_count(&pool, None, Some("regular")).await?.total,
      1
    );
    assert_eq!(
      validator_count(&pool, None, Some("evonode")).await?.total,
      1
    );
    assert_eq!(
      validator_count(&pool, Some("revoked"), Some("evonode"))
        .await?
        .total,
      1
    );

    Ok(())
  }
}
