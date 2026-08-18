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
use crate::layout::{partition_span, undo_span};
use crate::records::HyDbDataSrc;
use crate::types::HyDbHash;

pub(super) const GENESIS_PREV: [u8; 32] = [0u8; 32];

/// The height-partitioned base tables, created ahead of the tip.
const PARTITIONED_TABLES: [&str; 5] = [
  "transactions",
  "tx_inputs",
  "tx_outputs",
  "address_txs",
  "spends",
];

/// The height-partitioned undo tables, created ahead of the tip and pruned by
/// `DROP TABLE` on a whole partition. All three have the same shape — one row
/// per (height, key) written by connect and dead the moment the height falls
/// out of the reorg horizon — so all three are converted; `undo_blocks` and
/// `undo_validators` are lower-volume than `undo_address_stats` but their
/// prune is the same delete-everything-below-the-cutoff sweep, and leaving
/// them on it would mean two prune mechanisms for one horizon.
const UNDO_TABLES: [&str; 3] = ["undo_address_stats", "undo_blocks", "undo_validators"];

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
/// partitioned base table and every undo table. Runs *outside* any block
/// write transaction — the `CREATE TABLE ... PARTITION OF` takes
/// `ACCESS EXCLUSIVE` on the parent, so it must never ride inside the
/// per-block transaction — and probes `to_regclass` first so the steady state
/// issues no DDL at all.
///
/// The undo family is probed with a single round trip for the whole set,
/// because its span is small enough that an advance of one block still spans
/// a partition boundary now and then: a per-name probe would put three extra
/// queries on the per-block path to answer "nothing to do" almost every time.
pub(super) async fn ensure_partitions_for_range(pool: &PgPool, lo: i32, hi: i32) -> HyDbResult<()> {
  let lo_idx = lo.max(0) / partition_span();
  let hi_idx = hi.max(0) / partition_span();
  for idx in lo_idx..=hi_idx {
    let range_lo = idx * partition_span();
    let range_hi = range_lo + partition_span();
    for parent in PARTITIONED_TABLES {
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
  ensure_undo_partitions(pool, lo, hi).await
}

/// Create the undo partitions covering `[lo, hi]` that do not yet exist.
async fn ensure_undo_partitions(pool: &PgPool, lo: i32, hi: i32) -> HyDbResult<()> {
  let lo_idx = lo.max(0) / undo_span();
  let hi_idx = hi.max(0) / undo_span();
  let wanted: Vec<String> = (lo_idx..=hi_idx)
    .flat_map(|idx| UNDO_TABLES.map(move |parent| format!("{parent}_p{idx}")))
    .collect();

  // One probe for the whole set: the boundary check that keeps DDL — and all
  // but this single query — off the per-block path.
  let present: Vec<String> = sqlx::query_scalar!(
    r#"SELECT n AS "n!" FROM UNNEST($1::text[]) AS n WHERE to_regclass(n) IS NOT NULL"#,
    &wanted
  )
  .fetch_all(pool)
  .await
  .map_err(|e| HyDbError::pg("check undo partitions", e))?;
  if present.len() == wanted.len() {
    return Ok(());
  }

  for idx in lo_idx..=hi_idx {
    let range_lo = idx * undo_span();
    let range_hi = range_lo + undo_span();
    for parent in UNDO_TABLES {
      let name = format!("{parent}_p{idx}");
      if present.contains(&name) {
        continue;
      }
      let sql = format!(
        "CREATE TABLE IF NOT EXISTS {name} PARTITION OF {parent} FOR VALUES FROM ({range_lo}) TO ({range_hi})"
      );
      sqlx::query(&sql)
        .execute(pool)
        .await
        .map_err(|e| HyDbError::pg("ensure undo partitions", e))?;
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

/// Depth-bounded pruning of the undo tables, by dropping whole height
/// partitions.
///
/// Rows at or below `cutoff` can no longer be replayed (the reorg bound is
/// shallower), so they are dead weight — but deleting them was the single
/// most expensive thing this database did: 36.68M deletes against
/// `undo_address_stats` over a 967k-block mainnet sync, and the 93 autovacuum
/// passes needed to reclaim them, more than any other object. A dropped
/// partition leaves no dead tuples to reclaim; the space comes back as an
/// unlink.
///
/// **The horizon rule.** A partition is dropped only when the *highest height
/// it could possibly hold* — `(idx + 1) * span - 1`, not the highest height
/// actually in it — is at or below the cutoff. The bound is a property of the
/// partition's declared range, so it holds whether or not the partition is
/// fully populated, and it can never be satisfied by a partition containing a
/// height the reorg bound still reaches. That is what makes the coarser
/// granularity safe: the cost of a partial partition is retention, never
/// reversibility.
///
/// Runs outside the block transaction (from `finish_advance`, after the
/// commit). `DROP TABLE` on a partition takes `ACCESS EXCLUSIVE` on the
/// parent, so each drop is fenced by a short `lock_timeout` and simply
/// deferred to the next advance if a reader holds the parent — the partition
/// is already past the horizon and no correctness depends on when it goes.
pub(super) async fn prune_undo(pool: &PgPool, cutoff: i32) -> HyDbResult<()> {
  if cutoff < 0 {
    return Ok(());
  }
  let parents: Vec<String> = UNDO_TABLES.iter().map(|t| (*t).to_owned()).collect();
  let children = sqlx::query!(
    r#"SELECT p.relname AS "parent!", c.relname AS "child!"
         FROM pg_inherits i
         JOIN pg_class c ON c.oid = i.inhrelid
         JOIN pg_class p ON p.oid = i.inhparent
        WHERE p.relname = ANY($1)"#,
    &parents
  )
  .fetch_all(pool)
  .await
  .map_err(|e| HyDbError::pg("list undo partitions", e))?;

  for row in children {
    let Some(idx) = row
      .child
      .strip_prefix(&format!("{}_p", row.parent))
      .and_then(|suffix| suffix.parse::<i64>().ok())
    else {
      // Not one of ours: leave it alone rather than guess its range.
      continue;
    };
    let highest = (idx + 1) * i64::from(undo_span()) - 1;
    if highest > i64::from(cutoff) {
      continue;
    }
    drop_partition(pool, &row.child).await?;
  }
  Ok(())
}

/// Drop one undo partition under a short lock timeout, treating a lock
/// timeout or deadlock as "try again next advance" rather than an error.
async fn drop_partition(pool: &PgPool, name: &str) -> HyDbResult<()> {
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| HyDbError::pg("prune undo: begin", e))?;
  let attempt = async {
    sqlx::query("SET LOCAL lock_timeout = '3s'")
      .execute(&mut *tx)
      .await?;
    sqlx::query(&format!("DROP TABLE IF EXISTS {name}"))
      .execute(&mut *tx)
      .await?;
    Ok::<(), sqlx::Error>(())
  }
  .await;

  match attempt {
    Ok(()) => {
      tx.commit()
        .await
        .map_err(|e| HyDbError::pg("prune undo: commit", e))?;
      tracing::debug!(partition = name, "dropped undo partition below the horizon");
      Ok(())
    }
    Err(e) if contended(&e) => {
      tracing::debug!(
        partition = name,
        "undo partition drop deferred: parent is locked by a reader"
      );
      Ok(())
    }
    Err(e) => Err(HyDbError::pg("prune undo: drop partition", e)),
  }
}

/// `lock_timeout` (55P03) and `deadlock_detected` (40P01) — the two ways the
/// `ACCESS EXCLUSIVE` on the parent can lose a race with a reader.
fn contended(e: &sqlx::Error) -> bool {
  matches!(
    e.as_database_error().and_then(|d| d.code()).as_deref(),
    Some("55P03" | "40P01")
  )
}
