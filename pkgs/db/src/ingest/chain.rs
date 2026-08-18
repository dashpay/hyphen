//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Chain-topology primitives over `ser_blocks` and the canonical `blocks`
//! projection: absorbing raw bytes, resolving heights, canonical-tip reads,
//! the bounded fork walk, partition creation, undo pruning, and raw-byte
//! reads. These are the shared plumbing the connect/disconnect spine and the
//! deriver build on.

use sqlx::PgPool;

use super::HyDbBlockSeen;
use crate::error::{HyDbError, HyDbResult};
use crate::records::HyDbDataSrc;
use crate::types::HyDbHash;

pub(super) const GENESIS_PREV: [u8; 32] = [0u8; 32];
pub(super) const PARTITION_SPAN: i32 = 100_000;

/// Store a block's raw bytes and parent link in `ser_blocks`, assigning its
/// height as `parent.height + 1` (genesis is 0). A block whose parent is not
/// yet known is not stored — height is mandatory — and its parent hash is
/// reported so the caller can fetch it.
pub(super) async fn absorb(
  pool: &PgPool,
  data_src: &dyn HyDbDataSrc,
  raw: &[u8],
) -> HyDbResult<HyDbBlockSeen> {
  let id = data_src.identify(raw)?;

  let known = sqlx::query_scalar!("SELECT 1 FROM ser_blocks WHERE hash = $1", &id.hash.0[..])
    .fetch_optional(pool)
    .await
    .map_err(|e| HyDbError::pg("absorb: existence check", e))?;
  if known.is_some() {
    return Ok(HyDbBlockSeen::Known);
  }

  let height = if id.prev_hash.0 == GENESIS_PREV {
    0
  } else {
    match sqlx::query_scalar!(
      "SELECT height FROM ser_blocks WHERE hash = $1",
      &id.prev_hash.0[..]
    )
    .fetch_optional(pool)
    .await
    .map_err(|e| HyDbError::pg("absorb: parent lookup", e))?
    {
      Some(parent_height) => parent_height + 1,
      None => return Ok(HyDbBlockSeen::MissingParent(id.prev_hash)),
    }
  };

  sqlx::query!(
    "INSERT INTO ser_blocks (hash, prev_hash, height, raw) VALUES ($1, $2, $3, $4)
       ON CONFLICT (hash) DO NOTHING",
    &id.hash.0[..],
    &id.prev_hash.0[..],
    height,
    raw
  )
  .execute(pool)
  .await
  .map_err(|e| HyDbError::pg("absorb: insert", e))?;

  Ok(HyDbBlockSeen::New)
}

/// Ensure the height-range partitions covering `[lo, hi]` exist for every
/// partitioned base table. Runs *outside* any block write transaction — the
/// `CREATE TABLE ... PARTITION OF` takes `ACCESS EXCLUSIVE` on the parent, so
/// it must never ride inside the per-block transaction — and probes
/// `to_regclass` first so the steady state issues no DDL at all.
pub(super) async fn ensure_partitions_for_range(pool: &PgPool, lo: i32, hi: i32) -> HyDbResult<()> {
  let lo_idx = lo.max(0) / PARTITION_SPAN;
  let hi_idx = hi.max(0) / PARTITION_SPAN;
  for idx in lo_idx..=hi_idx {
    let range_lo = idx * PARTITION_SPAN;
    let range_hi = range_lo + PARTITION_SPAN;
    for parent in [
      "transactions",
      "tx_inputs",
      "tx_outputs",
      "address_txs",
      "spends",
    ] {
      let name = format!("{parent}_p{idx}");
      let exists = sqlx::query_scalar!(r#"SELECT to_regclass($1) IS NOT NULL AS "exists!""#, &name)
        .fetch_one(pool)
        .await
        .map_err(|e| HyDbError::pg("check partition", e))?;
      if exists {
        continue;
      }
      let sql = format!(
        "CREATE TABLE IF NOT EXISTS {name} PARTITION OF {parent} FOR VALUES FROM ({range_lo}) TO ({range_hi})"
      );
      sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|e| HyDbError::pg("ensure partitions", e))?;
    }
  }
  Ok(())
}

/// The canonical tip (height and hash), if any.
pub(super) async fn canonical_tip(pool: &PgPool) -> HyDbResult<Option<(i32, HyDbHash)>> {
  let row = sqlx::query!("SELECT height, hash FROM blocks ORDER BY height DESC LIMIT 1")
    .fetch_optional(pool)
    .await
    .map_err(|e| HyDbError::pg("canonical tip", e))?;
  row
    .map(|r| Ok((r.height, HyDbHash::from_slice(&r.hash)?)))
    .transpose()
}

/// The height a block was absorbed at, if seen.
pub(super) async fn ser_height(pool: &PgPool, hash: &HyDbHash) -> HyDbResult<Option<i32>> {
  sqlx::query_scalar!("SELECT height FROM ser_blocks WHERE hash = $1", &hash.0[..])
    .fetch_optional(pool)
    .await
    .map_err(|e| HyDbError::pg("ser height", e))
}

/// A seen-block row: identity, parent link, absorbed height.
pub(super) struct SerRow {
  pub hash: HyDbHash,
  pub prev_hash: [u8; 32],
  pub height: i32,
}

/// Every absorbed block with `lo <= height <= hi`, ordered by height.
pub(super) async fn ser_rows_in_range(pool: &PgPool, lo: i32, hi: i32) -> HyDbResult<Vec<SerRow>> {
  let rows = sqlx::query!(
    "SELECT hash, prev_hash, height FROM ser_blocks WHERE height >= $1 AND height <= $2
      ORDER BY height ASC",
    lo,
    hi
  )
  .fetch_all(pool)
  .await
  .map_err(|e| HyDbError::pg("ser rows in range", e))?;
  rows
    .into_iter()
    .map(|r| {
      Ok(SerRow {
        hash: HyDbHash::from_slice(&r.hash)?,
        prev_hash: r
          .prev_hash
          .as_slice()
          .try_into()
          .map_err(|_| HyDbError::Decode("ser_blocks.prev_hash not 32 bytes".to_owned()))?,
        height: r.height,
      })
    })
    .collect()
}

/// The result of walking parent links back from a candidate tip.
pub(super) enum WalkOutcome {
  /// The candidate tip is already canonical — nothing to do.
  AlreadyCanonical,
  /// A connectable segment (oldest → newest) above the fork point.
  /// `fork_height` is `None` when the segment starts at genesis.
  Segment {
    fork_height: Option<i32>,
    segment: Vec<(HyDbHash, i32)>,
  },
  /// An ancestor is missing from `ser_blocks`; the chain cannot be formed
  /// until the caller absorbs it.
  MissingAncestor,
  /// The walk hit `max_depth` without reaching a canonical ancestor.
  DepthExceeded,
}

/// Walk parents back from `tip`, entirely in SQL (one round trip, a bounded
/// recursive CTE), until a canonical ancestor, genesis, or `max_depth`.
pub(super) async fn walk_to_fork(
  pool: &PgPool,
  tip: &HyDbHash,
  max_depth: i64,
) -> HyDbResult<WalkOutcome> {
  let rows = sqlx::query!(
    r#"WITH RECURSIVE walk AS (
         SELECT s.hash, s.prev_hash, s.height, 1::int8 AS depth,
                (b.hash IS NOT NULL) AS canonical
           FROM ser_blocks s
           LEFT JOIN blocks b ON b.height = s.height AND b.hash = s.hash
          WHERE s.hash = $1
         UNION ALL
         SELECT s.hash, s.prev_hash, s.height, w.depth + 1,
                (b.hash IS NOT NULL) AS canonical
           FROM walk w
           JOIN ser_blocks s ON s.hash = w.prev_hash
           LEFT JOIN blocks b ON b.height = s.height AND b.hash = s.hash
          WHERE NOT w.canonical AND w.height > 0 AND w.depth < $2
       )
       SELECT hash AS "hash!", height AS "height!", canonical AS "canonical!"
         FROM walk ORDER BY height ASC"#,
    &tip.0[..],
    max_depth
  )
  .fetch_all(pool)
  .await
  .map_err(|e| HyDbError::pg("walk to fork", e))?;

  let Some(deepest) = rows.first() else {
    // The tip itself is not in ser_blocks.
    return Ok(WalkOutcome::MissingAncestor);
  };

  if deepest.canonical {
    if rows.len() == 1 {
      return Ok(WalkOutcome::AlreadyCanonical);
    }
    let fork_height = Some(deepest.height);
    let segment = rows[1..]
      .iter()
      .map(|r| Ok((HyDbHash::from_slice(&r.hash)?, r.height)))
      .collect::<HyDbResult<Vec<_>>>()?;
    return Ok(WalkOutcome::Segment {
      fork_height,
      segment,
    });
  }

  if deepest.height == 0 {
    // Reached genesis without meeting the canonical chain: connect from 0.
    let segment = rows
      .iter()
      .map(|r| Ok((HyDbHash::from_slice(&r.hash)?, r.height)))
      .collect::<HyDbResult<Vec<_>>>()?;
    return Ok(WalkOutcome::Segment {
      fork_height: None,
      segment,
    });
  }

  if rows.len() as i64 >= max_depth {
    return Ok(WalkOutcome::DepthExceeded);
  }
  // The recursion stopped because the parent of `deepest` is not absorbed.
  Ok(WalkOutcome::MissingAncestor)
}

/// The canonical block hash at `height`, if any (used to name a disconnected
/// block and to verify tip linkage).
pub(super) async fn canonical_hash_at<'e>(
  ex: impl sqlx::PgExecutor<'e>,
  height: i32,
) -> HyDbResult<Option<HyDbHash>> {
  let row = sqlx::query_scalar!("SELECT hash FROM blocks WHERE height = $1", height)
    .fetch_optional(ex)
    .await
    .map_err(|e| HyDbError::pg("canonical hash", e))?;
  row.map(|h| HyDbHash::from_slice(&h)).transpose()
}

/// The stored raw bytes of an absorbed block.
pub(super) async fn raw_bytes<'e>(
  ex: impl sqlx::PgExecutor<'e>,
  hash: &HyDbHash,
) -> HyDbResult<Vec<u8>> {
  sqlx::query_scalar!("SELECT raw FROM ser_blocks WHERE hash = $1", &hash.0[..])
    .fetch_optional(ex)
    .await
    .map_err(|e| HyDbError::pg("raw bytes", e))?
    .ok_or_else(|| HyDbError::Derive("connect: block missing from ser_blocks".to_owned()))
}

/// Depth-bounded pruning of the undo tables: rows at or below `cutoff` can no
/// longer be replayed (the reorg bound is shallower), so they are dead weight.
pub(super) async fn prune_undo(pool: &PgPool, cutoff: i32) -> HyDbResult<()> {
  if cutoff < 0 {
    return Ok(());
  }
  for table in ["undo_address_stats", "undo_blocks", "undo_validators"] {
    let sql = format!("DELETE FROM {table} WHERE height <= $1");
    sqlx::query(&sql)
      .bind(cutoff)
      .execute(pool)
      .await
      .map_err(|e| HyDbError::pg("prune undo", e))?;
  }
  Ok(())
}
