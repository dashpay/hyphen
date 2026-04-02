//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block index SQL operations.

use num_bigint::BigUint;
use sqlx::PgPool;

use crate::error::DaemonError;

use crate::index::block::util::CbTxFields;

/// One canonical block row for paginated block list responses.
#[derive(Debug, Clone)]
pub struct BlockListRow {
  pub height: i32,
  pub hash: Vec<u8>,
  pub prev_hash: Vec<u8>,
  pub time: i32,
  pub bits: i32,
  pub nonce: i64,
  pub version: i32,
  pub size: i32,
  pub tx_count: i32,
}

/// One canonical or orphan block row for block detail responses.
#[derive(Debug, Clone)]
pub struct BlockDetailRow {
  pub height: i32,
  pub hash: Vec<u8>,
  pub prev_hash: Vec<u8>,
  pub merkle_root: Vec<u8>,
  pub time: i32,
  pub bits: i32,
  pub nonce: i64,
  pub version: i32,
  pub size: i32,
  pub tx_count: i32,
  pub chainwork: Option<String>,
  pub cb_height: Option<i32>,
  pub cb_merkle_root: Option<Vec<u8>>,
  pub cb_merkle_flags: Option<i32>,
  pub cb_best_cl_height: Option<i32>,
  pub cb_best_cl_sig: Option<Vec<u8>>,
  pub cb_credit_pool: Option<i64>,
  pub orphaned_at_epoch: Option<i64>,
}

/// One transaction summary row within a block transaction listing.
#[derive(Debug, Clone)]
pub struct BlockTxRow {
  pub txid: Vec<u8>,
  pub block_pos: i32,
  pub version: i32,
  pub tx_type: crate::sql::types::TxTypeDb,
  pub lock_time: i32,
  pub size: i32,
  pub value_out: i64,
  pub input_count: i32,
  pub output_count: i32,
}

#[derive(Debug, Clone)]
pub struct BlockSearchRow {
  pub hash: Vec<u8>,
  pub height: i32,
}

/// Read the previous block chainwork for cumulative work calculation.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn prev_chainwork(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
) -> Result<BigUint, DaemonError> {
  if height == 0 {
    return Ok(BigUint::ZERO);
  }
  let row: Option<String> = sqlx::query_scalar("SELECT chainwork FROM blocks WHERE height = $1")
    .bind((height - 1) as i32)
    .fetch_optional(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("prev chainwork: {e}")))?;
  Ok(
    row
      .and_then(|s| BigUint::parse_bytes(s.as_bytes(), 16))
      .unwrap_or_default(),
  )
}

/// List canonical blocks in descending height order.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn list_blocks(
  pool: &PgPool,
  limit: i64,
  before_height: Option<i32>,
) -> Result<Vec<BlockListRow>, DaemonError> {
  use sqlx::Row;

  let rows = match before_height {
    Some(before_height) => {
      sqlx::query(
        "SELECT height, hash, prev_hash, time, bits, nonce, version, size, tx_count
         FROM blocks
         WHERE height < $1
         ORDER BY height DESC
         LIMIT $2",
      )
      .bind(before_height)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    None => {
      sqlx::query(
        "SELECT height, hash, prev_hash, time, bits, nonce, version, size, tx_count
         FROM blocks
         ORDER BY height DESC
         LIMIT $1",
      )
      .bind(limit)
      .fetch_all(pool)
      .await
    }
  }
  .map_err(|e| DaemonError::Pg(format!("list blocks: {e}")))?;

  Ok(
    rows
      .iter()
      .map(|row| BlockListRow {
        height: row.get("height"),
        hash: row.get("hash"),
        prev_hash: row.get("prev_hash"),
        time: row.get("time"),
        bits: row.get("bits"),
        nonce: row.get("nonce"),
        version: row.get("version"),
        size: row.get("size"),
        tx_count: row.get("tx_count"),
      })
      .collect(),
  )
}

/// Read one canonical block by height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn block_by_height(
  pool: &PgPool,
  height: i32,
) -> Result<Option<BlockDetailRow>, DaemonError> {
  block_detail(
    sqlx::query(
      "SELECT height, hash, prev_hash, merkle_root, time, bits, nonce,
              version, size, tx_count, chainwork,
              cb_height, cb_merkle_root, cb_merkle_flags,
              cb_best_cl_height, cb_best_cl_sig, cb_credit_pool,
              NULL::bigint AS orphaned_at_epoch
       FROM blocks WHERE height = $1",
    )
    .bind(height)
    .fetch_optional(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read block {height}: {e}")))?,
  )
}

/// Read one canonical block by hash.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn block_by_hash(
  pool: &PgPool,
  hash: &[u8],
) -> Result<Option<BlockDetailRow>, DaemonError> {
  block_detail(
    sqlx::query(
      "SELECT height, hash, prev_hash, merkle_root, time, bits, nonce,
              version, size, tx_count, chainwork,
              cb_height, cb_merkle_root, cb_merkle_flags,
              cb_best_cl_height, cb_best_cl_sig, cb_credit_pool,
              NULL::bigint AS orphaned_at_epoch
       FROM blocks WHERE hash = $1",
    )
    .bind(hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read block by hash: {e}")))?,
  )
}

/// Read one orphan block by hash.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn orphan_block_by_hash(
  pool: &PgPool,
  hash: &[u8],
) -> Result<Option<BlockDetailRow>, DaemonError> {
  block_detail(
    sqlx::query(
      "SELECT height, hash, prev_hash, merkle_root, time, bits, nonce,
              version, size, tx_count, chainwork,
              cb_height, cb_merkle_root, cb_merkle_flags,
              cb_best_cl_height, cb_best_cl_sig, cb_credit_pool,
              EXTRACT(EPOCH FROM orphaned_at)::bigint AS orphaned_at_epoch
       FROM orphan_blocks WHERE hash = $1",
    )
    .bind(hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read orphan block by hash: {e}")))?,
  )
}

/// Read canonical block suggestions by display-hash prefix.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn search_blocks_by_prefix(
  pool: &PgPool,
  prefix: &str,
  limit: i64,
) -> Result<Vec<BlockSearchRow>, DaemonError> {
  use sqlx::Row;

  let pattern = format!("{prefix}%");
  let rows = sqlx::query(
    "SELECT hash, height
       FROM blocks
      WHERE encode(hash, 'hex') LIKE $1
      ORDER BY height DESC
      LIMIT $2",
  )
  .bind(pattern)
  .bind(limit)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("search block prefix: {e}")))?;

  Ok(
    rows
      .into_iter()
      .map(|row| BlockSearchRow {
        hash: row.get("hash"),
        height: row.get("height"),
      })
      .collect(),
  )
}

#[cfg(any(test, all(feature = "tests", feature = "daemon")))]
/// Insert one orphan block row because non-SQL tests need archived block state without owning SQL inline.
pub async fn insert_orphan_block_for_tests(
  pool: &PgPool,
  height: i32,
  hash: &[u8],
  prev_hash: &[u8],
  merkle_root: &[u8],
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO orphan_blocks (
       height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count, chainwork,
       cb_height, cb_merkle_root, cb_merkle_flags, cb_best_cl_height, cb_best_cl_sig, cb_credit_pool, orphaned_at
     ) VALUES ($1, $2, $3, $4, $5, 486604799, 0, 1, 512, 1, $6, NULL, NULL, NULL, NULL, NULL, NULL, NOW())",
  )
  .bind(height)
  .bind(hash)
  .bind(prev_hash)
  .bind(merkle_root)
  .bind(height)
  .bind(format!("{height:064x}"))
  .execute(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("insert orphan block {height}: {e}")))?;
  Ok(())
}

/// List transaction summaries for one block height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn block_txs(
  pool: &PgPool,
  height: i32,
  limit: i64,
  after: Option<(i32, &[u8])>,
) -> Result<Vec<BlockTxRow>, DaemonError> {
  use sqlx::Row;

  let rows = match after {
    Some((after_block_pos, after_txid)) => {
      sqlx::query(
        "SELECT txid, block_pos, version, tx_type, lock_time, size,
                value_out, input_count, output_count
         FROM transactions
         WHERE block_height = $1
           AND (block_pos > $2 OR (block_pos = $2 AND txid > $3))
         ORDER BY block_pos ASC, txid ASC
         LIMIT $4",
      )
      .bind(height)
      .bind(after_block_pos)
      .bind(after_txid)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    None => {
      sqlx::query(
        "SELECT txid, block_pos, version, tx_type, lock_time, size,
                value_out, input_count, output_count
         FROM transactions
         WHERE block_height = $1
         ORDER BY block_pos ASC, txid ASC
         LIMIT $2",
      )
      .bind(height)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
  }
  .map_err(|e| DaemonError::Pg(format!("read block txs at height {height}: {e}")))?;

  Ok(
    rows
      .iter()
      .map(|row| BlockTxRow {
        txid: row.get("txid"),
        block_pos: row.get("block_pos"),
        version: row.get("version"),
        tx_type: row.get("tx_type"),
        lock_time: row.get("lock_time"),
        size: row.get("size"),
        value_out: row.get("value_out"),
        input_count: row.get("input_count"),
        output_count: row.get("output_count"),
      })
      .collect(),
  )
}

/// Check whether a canonical block exists at the given height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn block_exists(pool: &PgPool, height: i32) -> Result<bool, DaemonError> {
  let exists: Option<(i32,)> = sqlx::query_as("SELECT 1 FROM blocks WHERE height = $1")
    .bind(height)
    .fetch_optional(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("check block {height}: {e}")))?;
  Ok(exists.is_some())
}

/// Read the canonical block height for a block hash.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn block_height_for_hash(pool: &PgPool, hash: &[u8]) -> Result<Option<i32>, DaemonError> {
  let row: Option<(i32,)> = sqlx::query_as("SELECT height FROM blocks WHERE hash = $1")
    .bind(hash)
    .fetch_optional(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read block height for hash: {e}")))?;
  Ok(row.map(|(height,)| height))
}

/// Read the canonical block hash stored at one height.
///
/// # Errors
///
/// Returns an error when PostgreSQL query execution fails.
pub async fn block_hash_for_height(
  pool: &PgPool,
  height: i32,
) -> Result<Option<Vec<u8>>, DaemonError> {
  sqlx::query_scalar("SELECT hash FROM blocks WHERE height = $1")
    .bind(height)
    .fetch_optional(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read block hash at height {height}: {e}")))
}

fn block_detail(row: Option<sqlx::postgres::PgRow>) -> Result<Option<BlockDetailRow>, DaemonError> {
  use sqlx::Row;

  Ok(row.map(|row| BlockDetailRow {
    height: row.get("height"),
    hash: row.get("hash"),
    prev_hash: row.get("prev_hash"),
    merkle_root: row.get("merkle_root"),
    time: row.get("time"),
    bits: row.get("bits"),
    nonce: row.get("nonce"),
    version: row.get("version"),
    size: row.get("size"),
    tx_count: row.get("tx_count"),
    chainwork: row.get("chainwork"),
    cb_height: row.get("cb_height"),
    cb_merkle_root: row.get("cb_merkle_root"),
    cb_merkle_flags: row.get("cb_merkle_flags"),
    cb_best_cl_height: row.get("cb_best_cl_height"),
    cb_best_cl_sig: row.get("cb_best_cl_sig"),
    cb_credit_pool: row.get("cb_credit_pool"),
    orphaned_at_epoch: row.get("orphaned_at_epoch"),
  }))
}

#[cfg(test)]
mod tests {
  use sqlx::PgPool;

  use super::{
    block_by_hash, block_by_height, block_exists, block_hash_for_height, block_height_for_hash,
    block_txs, list_blocks, orphan_block_by_hash,
  };

  /// Build one stable block hash for seeded block-route rows.
  fn synthetic_hash(domain: u8, id: i32) -> Vec<u8> {
    let mut value = [0_u8; 32];
    value[0] = domain;
    value[1..5].copy_from_slice(&id.to_be_bytes());
    for (index, byte) in value[5..].iter_mut().enumerate() {
      *byte = domain.wrapping_add(index as u8).wrapping_mul(19);
    }
    value.to_vec()
  }

  /// Seed one canonical block row because block read routes query persisted block facts directly.
  async fn seed_block(pool: &PgPool, height: i32) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let hash = synthetic_hash(b'B', height);
    let prev_hash = if height == 0 {
      vec![0_u8; 32]
    } else {
      synthetic_hash(b'B', height - 1)
    };
    sqlx::query(
      "INSERT INTO blocks (
         height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count, chainwork,
         cb_height, cb_merkle_root, cb_merkle_flags, cb_best_cl_height, cb_best_cl_sig, cb_credit_pool
       ) VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 1024, 2, $8, NULL, NULL, NULL, NULL, NULL, NULL)",
    )
    .bind(height)
    .bind(&hash)
    .bind(prev_hash)
    .bind(synthetic_hash(b'M', height))
    .bind(height)
    .bind(486_604_799_i32)
    .bind(i64::from(height))
    .bind(format!("{height:064x}"))
    .execute(pool)
    .await?;
    Ok(hash)
  }

  /// Seed one orphan block row because orphan lookup uses a separate archive table.
  async fn seed_orphan_block(
    pool: &PgPool,
    height: i32,
  ) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let hash = synthetic_hash(b'O', height);
    sqlx::query(
      "INSERT INTO orphan_blocks (
         height, hash, prev_hash, merkle_root, time, bits, nonce, version, size, tx_count, chainwork,
         cb_height, cb_merkle_root, cb_merkle_flags, cb_best_cl_height, cb_best_cl_sig, cb_credit_pool, orphaned_at
       ) VALUES ($1, $2, $3, $4, $5, $6, $7, 1, 512, 1, $8, NULL, NULL, NULL, NULL, NULL, NULL, NOW())",
    )
    .bind(height)
    .bind(&hash)
    .bind(vec![0_u8; 32])
    .bind(synthetic_hash(b'R', height))
    .bind(height)
    .bind(486_604_799_i32)
    .bind(i64::from(height))
    .bind(format!("{height:064x}"))
    .execute(pool)
    .await?;
    Ok(hash)
  }

  /// Seed one transaction row at the given block height because block tx paging reads `transactions`.
  async fn seed_block_tx(
    pool: &PgPool,
    txid: Vec<u8>,
    height: i32,
    block_pos: i32,
  ) -> Result<(), Box<dyn std::error::Error>> {
    sqlx::query(
      "INSERT INTO transactions (
         txid, block_height, block_pos, version, tx_type, lock_time, size, value_out, input_count, output_count
       ) VALUES ($1, $2, $3, 3, 'normal', 0, 200, 1000, 1, 1)",
    )
    .bind(txid)
    .bind(height)
    .bind(block_pos)
    .execute(pool)
    .await?;
    Ok(())
  }

  #[tokio::test]
  async fn block_read_routes_return_seeded_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let hash0 = seed_block(&pool, 0).await?;
    let hash1 = seed_block(&pool, 1).await?;

    let listed = list_blocks(&pool, 2, None).await?;
    assert_eq!(listed.len(), 2);
    assert_eq!(listed[0].height, 1);
    assert_eq!(listed[1].height, 0);

    let by_height = block_by_height(&pool, 1).await?;
    assert_eq!(by_height.expect("block should exist").hash, hash1);

    let by_hash = block_by_hash(&pool, hash0.as_slice()).await?;
    assert_eq!(by_hash.expect("block should exist").height, 0);

    assert!(block_exists(&pool, 1).await?);
    assert_eq!(
      block_height_for_hash(&pool, hash1.as_slice()).await?,
      Some(1)
    );
    assert_eq!(block_hash_for_height(&pool, 0).await?, Some(hash0));

    Ok(())
  }

  #[tokio::test]
  async fn block_txs_and_orphan_lookup_read_seeded_rows_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    seed_block(&pool, 7).await?;
    let orphan_hash = seed_orphan_block(&pool, 8).await?;
    let first_txid = synthetic_hash(b'T', 1);
    let second_txid = synthetic_hash(b'T', 2);
    seed_block_tx(&pool, first_txid.clone(), 7, 0).await?;
    seed_block_tx(&pool, second_txid.clone(), 7, 1).await?;

    let first_page = block_txs(&pool, 7, 1, None).await?;
    assert_eq!(first_page.len(), 1);
    assert_eq!(first_page[0].txid, first_txid);

    let later_page = block_txs(
      &pool,
      7,
      2,
      Some((first_page[0].block_pos, first_page[0].txid.as_slice())),
    )
    .await?;
    assert_eq!(later_page.len(), 1);
    assert_eq!(later_page[0].txid, second_txid);

    let orphan = orphan_block_by_hash(&pool, orphan_hash.as_slice()).await?;
    assert_eq!(orphan.expect("orphan block should exist").height, 8);

    Ok(())
  }
}

#[expect(clippy::too_many_arguments)]
pub async fn insert_block(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
  block_hash: &[u8],
  prev_hash: &[u8],
  merkle_root: &[u8],
  time: i32,
  bits: i32,
  nonce: i64,
  version: i32,
  size: i32,
  tx_count: i32,
  chainwork: &str,
  cb: &CbTxFields,
) -> Result<(), DaemonError> {
  let cb_mr = cb.merkle_root.as_deref();
  let cb_sig = cb.best_cl_sig.as_deref();

  sqlx::query(
    "INSERT INTO blocks (
      height, hash, prev_hash, merkle_root, time, bits, nonce,
      version, size, tx_count, chainwork,
      cb_height, cb_merkle_root, cb_merkle_flags,
      cb_best_cl_height, cb_best_cl_sig, cb_credit_pool
    ) VALUES ($1,$2,$3,$4,$5,$6,$7,$8,$9,$10,$11,$12,$13,$14,$15,$16,$17)
    ON CONFLICT (height) DO UPDATE SET
      hash = EXCLUDED.hash,
      prev_hash = EXCLUDED.prev_hash,
      merkle_root = EXCLUDED.merkle_root,
      time = EXCLUDED.time,
      bits = EXCLUDED.bits,
      nonce = EXCLUDED.nonce,
      version = EXCLUDED.version,
      size = EXCLUDED.size,
      tx_count = EXCLUDED.tx_count,
      chainwork = EXCLUDED.chainwork,
      cb_height = EXCLUDED.cb_height,
      cb_merkle_root = EXCLUDED.cb_merkle_root,
      cb_merkle_flags = EXCLUDED.cb_merkle_flags,
      cb_best_cl_height = EXCLUDED.cb_best_cl_height,
      cb_best_cl_sig = EXCLUDED.cb_best_cl_sig,
      cb_credit_pool = EXCLUDED.cb_credit_pool",
  )
  .bind(height as i32)
  .bind(block_hash)
  .bind(prev_hash)
  .bind(merkle_root)
  .bind(time)
  .bind(bits)
  .bind(nonce)
  .bind(version)
  .bind(size)
  .bind(tx_count)
  .bind(chainwork)
  .bind(cb.height)
  .bind(cb_mr)
  .bind(cb.merkle_flags)
  .bind(cb.best_cl_height)
  .bind(cb_sig)
  .bind(cb.credit_pool)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("insert block {height}: {e}")))?;

  Ok(())
}

pub async fn delete_block(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  height: u32,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO orphan_blocks (
       hash, height, prev_hash, merkle_root, time, bits, nonce,
       version, size, tx_count, chainwork,
       cb_height, cb_merkle_root, cb_merkle_flags,
       cb_best_cl_height, cb_best_cl_sig, cb_credit_pool
     )
     SELECT
       hash, height, prev_hash, merkle_root, time, bits, nonce,
       version, size, tx_count, chainwork,
       cb_height, cb_merkle_root, cb_merkle_flags,
       cb_best_cl_height, cb_best_cl_sig, cb_credit_pool
     FROM blocks WHERE height = $1
     ON CONFLICT (hash) DO NOTHING",
  )
  .bind(height as i32)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("archive block {height}: {e}")))?;

  sqlx::query("DELETE FROM blocks WHERE height = $1")
    .bind(height as i32)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("delete block {height}: {e}")))?;
  Ok(())
}
