//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The blocks domain: canonical headers from `blocks`, plus raw bytes and
//! topology for any block (canonical or orphan) from `ser_blocks`.
//!
//! The crate never decodes a block. Canonical detail is served straight from
//! the decoded `blocks` columns; an orphan's decoded header is the daemon's job
//! — it fetches [`HyDbBlockIdx::raw_bytes`] and decodes with its own data
//! source, using [`HyDbBlockIdx::topology`] to confirm the block is off the
//! main chain.

use async_trait::async_trait;

use super::PgRead;
use crate::error::{HyDbError, HyDbResult};
use crate::types::{HyDbCursor, HyDbHash, HyDbHeight, HyDbPage, HyDbTxType};
use crate::util::hex_prefix_pattern;

/// Full canonical block header detail.
#[derive(Debug, Clone)]
pub struct HyDbQrBlockDetail {
  pub height: HyDbHeight,
  pub hash: HyDbHash,
  pub prev_hash: HyDbHash,
  pub merkle_root: HyDbHash,
  pub time: i64,
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
}

/// Topology of any block ever seen: its parent link, the height the node
/// reported, and whether it currently sits on the canonical chain.
#[derive(Debug, Clone)]
pub struct HyDbQrBlockRef {
  pub hash: HyDbHash,
  pub prev_hash: HyDbHash,
  pub height: HyDbHeight,
  pub canonical: bool,
}

/// A block as it appears in a list view.
#[derive(Debug, Clone)]
pub struct HyDbQrBlockSummary {
  pub height: HyDbHeight,
  pub hash: HyDbHash,
  pub prev_hash: HyDbHash,
  pub time: i64,
  pub bits: i32,
  pub nonce: i64,
  pub version: i32,
  pub size: i32,
  pub tx_count: i32,
}

/// A transaction summary within a block's transaction listing.
#[derive(Debug, Clone)]
pub struct HyDbQrBlockTxRef {
  pub txid: HyDbHash,
  pub block_pos: i32,
  pub version: i32,
  pub tx_type: HyDbTxType,
  pub lock_time: i64,
  pub size: i32,
  pub value_out: i64,
  pub input_count: i32,
  pub output_count: i32,
}

/// Query handle for block data.
#[async_trait]
pub trait HyDbBlockIdx: Send + Sync {
  /// Newest-first page of canonical blocks.
  async fn list(
    &self,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrBlockSummary>>;

  /// Canonical block by height.
  async fn by_height(&self, height: HyDbHeight) -> HyDbResult<Option<HyDbQrBlockDetail>>;

  /// Canonical block by hash.
  async fn by_hash(&self, hash: HyDbHash) -> HyDbResult<Option<HyDbQrBlockDetail>>;

  /// The transactions of a block, in block order, keyset-paginated.
  async fn txs(
    &self,
    height: HyDbHeight,
    limit: u32,
    after: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrBlockTxRef>>;

  /// Raw serialized bytes of any block ever seen (canonical or orphan).
  async fn raw_bytes(&self, hash: HyDbHash) -> HyDbResult<Option<Vec<u8>>>;

  /// Topology of any block ever seen — the seam for orphan reporting. Returns
  /// `None` if the hash was never seen.
  async fn topology(&self, hash: HyDbHash) -> HyDbResult<Option<HyDbQrBlockRef>>;

  /// Canonical hashes whose DISPLAY-order hex (the byte-reverse of the stored
  /// bytes — exactly what the UI renders and users copy) starts with
  /// `prefix`, highest-height first. The prefix is lowercased and
  /// `LIKE`-escaped here; callers pass it through verbatim.
  async fn search_prefix(&self, prefix: String, limit: u32) -> HyDbResult<Vec<HyDbQrBlockRef>>;

  /// Whether a canonical block exists at this height.
  async fn exists(&self, height: HyDbHeight) -> HyDbResult<bool>;

  /// Canonical height for a hash, if it is on the main chain.
  async fn height_for_hash(&self, hash: HyDbHash) -> HyDbResult<Option<HyDbHeight>>;

  /// Canonical hash at a height.
  async fn hash_for_height(&self, height: HyDbHeight) -> HyDbResult<Option<HyDbHash>>;
}

/// Postgres implementation.
#[derive(Debug)]
pub(crate) struct PgBlockIdx {
  pub(crate) pg: PgRead,
}

/// Decode a display-hash (reversed hex) prefix cursor into a bound height.
fn height_cursor(cursor: Option<HyDbCursor>) -> HyDbResult<Option<i32>> {
  match cursor {
    None => Ok(None),
    Some(HyDbCursor(s)) => s
      .parse::<i32>()
      .map(Some)
      .map_err(|_| HyDbError::Decode(format!("invalid block cursor: {s}"))),
  }
}

#[async_trait]
impl HyDbBlockIdx for PgBlockIdx {
  async fn list(
    &self,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrBlockSummary>> {
    let limit = limit.clamp(1, 500) as i64;
    // Fetch one extra row: its presence is the has-more signal, so a terminal
    // page that is exactly full never hands out a cursor to an empty page.
    let fetch = limit + 1;
    let before = height_cursor(before)?;
    // Each `query!` call site produces its own anonymous record type, so the
    // two branches cannot share one, and dynamic SQL is not available to
    // `query!`. Normalize each arm to the same explicit tuple, then map once.
    type Row = (i32, Vec<u8>, Vec<u8>, i64, i32, i64, i32, i32, i32);
    let rows: Vec<Row> = match before {
      Some(h) => sqlx::query!(
        r#"SELECT height, hash, prev_hash, time, bits, nonce, version, size, tx_count
             FROM blocks WHERE height < $1 ORDER BY height DESC LIMIT $2"#,
        h,
        fetch
      )
      .fetch_all(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("list blocks", e))?
      .into_iter()
      .map(|r| {
        (
          r.height,
          r.hash,
          r.prev_hash,
          r.time,
          r.bits,
          r.nonce,
          r.version,
          r.size,
          r.tx_count,
        )
      })
      .collect(),
      None => sqlx::query!(
        r#"SELECT height, hash, prev_hash, time, bits, nonce, version, size, tx_count
             FROM blocks ORDER BY height DESC LIMIT $1"#,
        fetch
      )
      .fetch_all(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("list blocks", e))?
      .into_iter()
      .map(|r| {
        (
          r.height,
          r.hash,
          r.prev_hash,
          r.time,
          r.bits,
          r.nonce,
          r.version,
          r.size,
          r.tx_count,
        )
      })
      .collect(),
    };

    let has_more = rows.len() as i64 > limit;
    let mut items = Vec::with_capacity(rows.len().min(limit as usize));
    for r in rows.iter().take(limit as usize) {
      items.push(HyDbQrBlockSummary {
        height: HyDbHeight::try_from_i32(r.0)?,
        hash: HyDbHash::from_slice(&r.1)?,
        prev_hash: HyDbHash::from_slice(&r.2)?,
        time: r.3,
        bits: r.4,
        nonce: r.5,
        version: r.6,
        size: r.7,
        tx_count: r.8,
      });
    }
    let next = items
      .last()
      .filter(|_| has_more)
      .map(|b| HyDbCursor(b.height.get().to_string()));
    Ok(HyDbPage::new(items, next))
  }

  async fn by_height(&self, height: HyDbHeight) -> HyDbResult<Option<HyDbQrBlockDetail>> {
    let row = sqlx::query!(
      r#"SELECT height, hash, prev_hash, merkle_root, time, bits, nonce, version, size,
                tx_count, chainwork, cb_height, cb_merkle_root, cb_merkle_flags,
                cb_best_cl_height, cb_best_cl_sig, cb_credit_pool
           FROM blocks WHERE height = $1"#,
      height.as_i32()?
    )
    .fetch_optional(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("block by height", e))?;

    row
      .map(|r| {
        Ok(HyDbQrBlockDetail {
          height: HyDbHeight::try_from_i32(r.height)?,
          hash: HyDbHash::from_slice(&r.hash)?,
          prev_hash: HyDbHash::from_slice(&r.prev_hash)?,
          merkle_root: HyDbHash::from_slice(&r.merkle_root)?,
          time: r.time,
          bits: r.bits,
          nonce: r.nonce,
          version: r.version,
          size: r.size,
          tx_count: r.tx_count,
          chainwork: r.chainwork,
          cb_height: r.cb_height,
          cb_merkle_root: r.cb_merkle_root,
          cb_merkle_flags: r.cb_merkle_flags,
          cb_best_cl_height: r.cb_best_cl_height,
          cb_best_cl_sig: r.cb_best_cl_sig,
          cb_credit_pool: r.cb_credit_pool,
        })
      })
      .transpose()
  }

  async fn by_hash(&self, hash: HyDbHash) -> HyDbResult<Option<HyDbQrBlockDetail>> {
    let row = sqlx::query!(
      r#"SELECT height, hash, prev_hash, merkle_root, time, bits, nonce, version, size,
                tx_count, chainwork, cb_height, cb_merkle_root, cb_merkle_flags,
                cb_best_cl_height, cb_best_cl_sig, cb_credit_pool
           FROM blocks WHERE hash = $1"#,
      &hash.0[..]
    )
    .fetch_optional(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("block by hash", e))?;

    row
      .map(|r| {
        Ok(HyDbQrBlockDetail {
          height: HyDbHeight::try_from_i32(r.height)?,
          hash: HyDbHash::from_slice(&r.hash)?,
          prev_hash: HyDbHash::from_slice(&r.prev_hash)?,
          merkle_root: HyDbHash::from_slice(&r.merkle_root)?,
          time: r.time,
          bits: r.bits,
          nonce: r.nonce,
          version: r.version,
          size: r.size,
          tx_count: r.tx_count,
          chainwork: r.chainwork,
          cb_height: r.cb_height,
          cb_merkle_root: r.cb_merkle_root,
          cb_merkle_flags: r.cb_merkle_flags,
          cb_best_cl_height: r.cb_best_cl_height,
          cb_best_cl_sig: r.cb_best_cl_sig,
          cb_credit_pool: r.cb_credit_pool,
        })
      })
      .transpose()
  }

  async fn txs(
    &self,
    height: HyDbHeight,
    limit: u32,
    after: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrBlockTxRef>> {
    let limit = limit.clamp(1, 500) as i64;
    let fetch = limit + 1;
    let after = match after {
      None => None,
      Some(HyDbCursor(s)) => {
        let (pos, txid_hex) = s
          .split_once(':')
          .ok_or_else(|| HyDbError::Decode(format!("invalid block-tx cursor: {s}")))?;
        let pos: i32 = pos
          .parse()
          .map_err(|_| HyDbError::Decode(format!("invalid block-tx cursor pos: {s}")))?;
        let txid = hex::decode(txid_hex)?;
        Some((pos, txid))
      }
    };

    type Row = (Vec<u8>, i32, i32, HyDbTxType, i64, i32, i64, i32, i32);
    let rows: Vec<Row> = match &after {
      Some((pos, txid)) => sqlx::query!(
        r#"SELECT txid, block_pos, version, tx_type AS "tx_type: HyDbTxType", lock_time, size,
                  value_out, input_count, output_count
             FROM transactions
            WHERE block_height = $1 AND (block_pos > $2 OR (block_pos = $2 AND txid > $3))
            ORDER BY block_pos ASC, txid ASC LIMIT $4"#,
        height.as_i32()?,
        pos,
        &txid[..],
        fetch
      )
      .fetch_all(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("block txs", e))?
      .into_iter()
      .map(|r| {
        (
          r.txid,
          r.block_pos,
          r.version,
          r.tx_type,
          r.lock_time,
          r.size,
          r.value_out,
          r.input_count,
          r.output_count,
        )
      })
      .collect(),
      None => sqlx::query!(
        r#"SELECT txid, block_pos, version, tx_type AS "tx_type: HyDbTxType", lock_time, size,
                  value_out, input_count, output_count
             FROM transactions
            WHERE block_height = $1
            ORDER BY block_pos ASC, txid ASC LIMIT $2"#,
        height.as_i32()?,
        fetch
      )
      .fetch_all(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("block txs", e))?
      .into_iter()
      .map(|r| {
        (
          r.txid,
          r.block_pos,
          r.version,
          r.tx_type,
          r.lock_time,
          r.size,
          r.value_out,
          r.input_count,
          r.output_count,
        )
      })
      .collect(),
    };

    let has_more = rows.len() as i64 > limit;
    let mut items = Vec::with_capacity(rows.len().min(limit as usize));
    for r in rows.iter().take(limit as usize) {
      items.push(HyDbQrBlockTxRef {
        txid: HyDbHash::from_slice(&r.0)?,
        block_pos: r.1,
        version: r.2,
        tx_type: r.3,
        lock_time: r.4,
        size: r.5,
        value_out: r.6,
        input_count: r.7,
        output_count: r.8,
      });
    }
    let next = items.last().filter(|_| has_more).map(|t| {
      HyDbCursor(format!(
        "{}:{}",
        t.block_pos,
        hex::encode(t.txid.as_bytes())
      ))
    });
    Ok(HyDbPage::new(items, next))
  }

  async fn raw_bytes(&self, hash: HyDbHash) -> HyDbResult<Option<Vec<u8>>> {
    sqlx::query_scalar!("SELECT raw FROM ser_blocks WHERE hash = $1", &hash.0[..])
      .fetch_optional(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("raw block bytes", e))
  }

  async fn topology(&self, hash: HyDbHash) -> HyDbResult<Option<HyDbQrBlockRef>> {
    let row = sqlx::query!(
      r#"SELECT s.hash, s.prev_hash, s.height, (b.hash IS NOT NULL) AS "canonical!"
           FROM ser_blocks s LEFT JOIN blocks b ON b.hash = s.hash
          WHERE s.hash = $1"#,
      &hash.0[..]
    )
    .fetch_optional(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("block topology", e))?;

    row
      .map(|r| {
        Ok(HyDbQrBlockRef {
          hash: HyDbHash::from_slice(&r.hash)?,
          prev_hash: HyDbHash::from_slice(&r.prev_hash)?,
          height: HyDbHeight::try_from_i32(r.height)?,
          canonical: r.canonical,
        })
      })
      .transpose()
  }

  async fn search_prefix(&self, prefix: String, limit: u32) -> HyDbResult<Vec<HyDbQrBlockRef>> {
    let limit = limit.clamp(1, 100) as i64;
    // Match on DISPLAY-order hex (see the search contract in the schema): the
    // prefix a user copies from a UI is the byte-reverse of the stored bytes.
    // Lowercased + escaped by the shared helper; backed by the expression
    // index over display_hex(hash).
    let pattern = hex_prefix_pattern(&prefix);
    let rows = sqlx::query!(
      r#"SELECT hash, prev_hash, height
           FROM blocks WHERE display_hex(hash) LIKE $1 ESCAPE '\'
          ORDER BY height DESC LIMIT $2"#,
      pattern,
      limit
    )
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("search blocks by prefix", e))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in &rows {
      out.push(HyDbQrBlockRef {
        hash: HyDbHash::from_slice(&r.hash)?,
        prev_hash: HyDbHash::from_slice(&r.prev_hash)?,
        height: HyDbHeight::try_from_i32(r.height)?,
        canonical: true,
      });
    }
    Ok(out)
  }

  async fn exists(&self, height: HyDbHeight) -> HyDbResult<bool> {
    let row = sqlx::query_scalar!("SELECT 1 FROM blocks WHERE height = $1", height.as_i32()?)
      .fetch_optional(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("block exists", e))?;
    Ok(row.is_some())
  }

  async fn height_for_hash(&self, hash: HyDbHash) -> HyDbResult<Option<HyDbHeight>> {
    let row = sqlx::query_scalar!("SELECT height FROM blocks WHERE hash = $1", &hash.0[..])
      .fetch_optional(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("block height for hash", e))?;
    row.map(HyDbHeight::try_from_i32).transpose()
  }

  async fn hash_for_height(&self, height: HyDbHeight) -> HyDbResult<Option<HyDbHash>> {
    let row = sqlx::query_scalar!(
      "SELECT hash FROM blocks WHERE height = $1",
      height.as_i32()?
    )
    .fetch_optional(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("block hash for height", e))?;
    row.map(|h| HyDbHash::from_slice(&h)).transpose()
  }
}
