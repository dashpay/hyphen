//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The ingest pipeline, as a reversible sequence of stages.
//!
//! A bounded look-ahead ([`prefetch`]) reads each planned block out of
//! `ser_blocks` and decodes it *ahead of* the writer — both are pure with
//! respect to the branch, so neither belongs inside a transaction. One
//! read-only [`IngestDerive`] then turns the decoded block into a [`Scratch`]
//! of derived facts (resolved spends, per-address deltas, per-tx
//! fees, spent outpoints, chainwork, metric inputs). An ordered set of
//! [`IngestStage`s](IngestStage) then consumes that scratch, each owning a
//! slice of the projection and both directions of its mutation: `connect`
//! applies the block, `disconnect` reverses it. Because every write and its
//! undo live together in a stage, a reorg is just the stages replayed in
//! reverse — the deriver has no inverse. Derivation runs *inside* the block's
//! write transaction so reorg re-derivation observes the branch state the
//! transaction has already built.
//!
//! The spine here drives them: [`absorb`] stores raw bytes; [`advance`]
//! selects the canonical chain ending at the node's tip and moves the
//! projection toward it — a bounded segment per call, so a multi-million-block
//! cold sync reports progress continuously instead of walking the whole chain
//! silently. A reorg (disconnect + reconnect across the fork) commits as ONE
//! transaction: queries never observe a half-rewound chain, and a failure
//! while connecting the replacement branch preserves the old canonical state.

use std::sync::Arc;

use async_trait::async_trait;
use sqlx::PgPool;
use tokio::sync::watch;

use super::derive::PgDeriver;
use super::prefetch::{DecodeQueue, PrefetchCfg, Prefetched};
use super::scratch::{HyDbDeriveError, HyDbStageError, PipelineError, Scratch};
use super::{balances, base, chain, stats, validators};
use super::{HyDbBlockSeen, HyDbProgress, HyDbReorg, HyDbStage, HyDbTipReport};
use crate::error::{HyDbError, HyDbResult};
use crate::records::{HyDbBlock, HyDbDataSrc, PgTx};
use crate::stepped::{HyDbSyncMode, SteppedSync};
use crate::tuning;
use crate::types::{HyDbHash, HyDbHeight};

/// Upper bound on blocks connected per [`advance`] call. Keeps each call's
/// working set small and progress observable; the caller resumes by calling
/// again while `report.to < report.target`.
///
/// `[db.ingest] max_advance_blocks`; the built-in default is the 2,000 this
/// was a constant at.
fn max_advance_blocks() -> usize {
  tuning::max_advance_blocks()
}

/// Deepest reorg the pipeline will unwind. Must not exceed
/// [`undo_keep_depth`], which `HyDbIngestCfg::validate` now enforces at load
/// rather than leaving to a doc comment.
///
/// `[db.ingest] max_reorg_depth`.
fn max_reorg_depth() -> i64 {
  tuning::max_reorg_depth()
}

/// How far back the fork walk may look: the reorg bound plus one advance
/// segment, since the canonical tip may itself be a segment behind the node.
fn walk_bound() -> i64 {
  max_reorg_depth() + max_advance_blocks() as i64
}

/// How many blocks of undo pre-images to retain below the tip. Anything
/// deeper than the reorg bound can never be replayed, so it is pruned after
/// each successful advance to keep the undo tables bounded.
///
/// This is a *floor*, not an exact figure: the prune drops whole height
/// partitions rather than deleting rows (see [`chain::prune_undo`]), so up to
/// one `undo_span` of already-unreplayable undo survives below it.
/// Raising the depth can only retain more — the drop rule is keyed on the
/// cutoff this produces.
///
/// `[db.ingest] undo_keep_depth`.
fn undo_keep_depth() -> i32 {
  tuning::undo_keep_depth()
}

/// Store a block's raw bytes and parent link in `ser_blocks` (the absorb stage).
/// Thin spine entry point over [`chain::absorb`].
pub(super) async fn absorb(
  pool: &PgPool,
  data_src: &dyn HyDbDataSrc,
  raw: &[u8],
) -> HyDbResult<HyDbBlockSeen> {
  chain::absorb(pool, data_src, raw).await
}

// ===========================================================================
// The two roles: read-only derivation, and reversible per-slice writes
// ===========================================================================

/// Turns an already-decoded block into the derived facts a connect needs,
/// writing nothing. Runs inside the write transaction (so it sees the
/// transaction's own prior connects during a reorg); a disconnect never calls
/// it (every mutation's inverse lives in an [`IngestStage`]).
///
/// The *decode* is not part of this: it is a pure function of the block's
/// bytes, so A4 moved it out to [`prefetch`], ahead of the writer. What is
/// left here is exactly the part that has to see the transaction's own view —
/// the parent's chainwork and the UTXO set.
#[async_trait]
pub(super) trait IngestDerive: Send + Sync {
  async fn derive(
    &self,
    tx: &mut PgTx<'_>,
    block: HyDbBlock,
    scratch: &mut Scratch,
  ) -> Result<(), HyDbDeriveError>;
}

/// A slice of the projection with both directions of its mutation. Stages run in
/// order for a connect and in reverse for a disconnect; each is self-contained
/// (its `disconnect` reverses exactly what its `connect` wrote).
///
/// The reverse order is not cosmetic. A stage may reverse itself by
/// recomputing from base facts instead of from a stored pre-image — the
/// balance stage does exactly that, and it is why `undo_address_stats` carries
/// two columns rather than seven — which is sound precisely because
/// `BaseStage`, being first on connect, is last on disconnect and so has not
/// yet dropped the block's rows. Reordering the set, or reversing it in
/// connect order, breaks that.
#[async_trait]
pub(super) trait IngestStage: Send + Sync {
  /// A short identifier this stage stamps into the typed errors it raises.
  const NAME: &'static str;

  /// Apply this block to the stage's slice of the projection.
  async fn connect(&self, tx: &mut PgTx<'_>, scratch: &Scratch) -> Result<(), HyDbStageError>;

  /// Reverse a previously-connected block at `height`, from the undo rows the
  /// connect captured and — where a column is recomputable — the block's own
  /// base facts, which are still in place (see the note on ordering above).
  async fn disconnect(&self, tx: &mut PgTx<'_>, height: i32) -> Result<(), HyDbStageError>;
}

// The stage set is a fixed, ordered quartet dispatched statically (the required
// `IngestStage::NAME` associated const makes the trait not dyn-compatible, and a
// fixed set needs no `dyn`): base facts first (the balance stage reads them back
// on the way out, and every other accumulation reads from them conceptually),
// then balances, summaries/metrics, and the validator registry. Connects run
// them in this order; disconnects run them in reverse.

// ===========================================================================
// Canonicalize, then connect/disconnect toward the node tip
// ===========================================================================

/// What the planner decided an advance should do.
enum Plan {
  /// Nothing to do (tip already canonical, or an ancestor is missing).
  Noop,
  /// Pure forward extension of the canonical chain.
  Forward(Vec<(HyDbHash, i32)>),
  /// Disconnect above `fork_height`, then connect `segment` — atomically.
  Reorg {
    fork_height: Option<i32>,
    segment: Vec<(HyDbHash, i32)>,
  },
}

/// Advance the canonical projection toward `node_tip`, connecting at most
/// [`max_advance_blocks`] blocks. Publishes per-block progress on `progress`.
pub(super) async fn advance(
  pool: &PgPool,
  data_src: &Arc<dyn HyDbDataSrc>,
  node_tip: HyDbHash,
  progress: &watch::Sender<HyDbProgress>,
  stepped: &SteppedSync,
) -> HyDbResult<HyDbTipReport> {
  let from_tip = chain::canonical_tip(pool).await?;
  let from = from_tip
    .map(|(h, _)| HyDbHeight::try_from_i32(h))
    .transpose()?;

  let Some(node_height) = chain::ser_height(pool, &node_tip).await? else {
    // The tip must already be absorbed; nothing to do until the caller feeds
    // it (and any missing ancestors) via `on_block`.
    return Ok(HyDbTipReport {
      from,
      to: from,
      target: None,
      reorg: None,
      mode: stepped.mode(),
    });
  };
  let target = Some(HyDbHeight::try_from_i32(node_height)?);

  // One observation per advance: this is the "N consecutive advances" the
  // hysteresis counts, and the point at which the gap to the node is known
  // for certain.
  if let Some(transition) = stepped.observe(from_tip.map(|(h, _)| h), node_height) {
    stepped.apply(transition).await?;
    tuning::apply_storage_for(
      stepped.admin(),
      tuning::resolved(),
      transition.to == HyDbSyncMode::Stepped,
    )
    .await?;
  }

  let plan = plan_advance(pool, &from_tip, &node_tip, node_height).await?;

  match plan {
    Plan::Noop => Ok(HyDbTipReport {
      from,
      to: from,
      target,
      reorg: None,
      mode: stepped.mode(),
    }),
    Plan::Forward(segment) => {
      let publish = |height: i32, stage: HyDbStage| {
        let _ = progress.send(HyDbProgress {
          stage,
          height: HyDbHeight::try_from_i32(height).ok(),
          target,
          mode: stepped.mode(),
        });
      };
      if let (Some(first), Some(last)) = (segment.first(), segment.last()) {
        chain::ensure_partitions_for_range(pool, first.1, last.1).await?;
        // Reconcile before writing a block into the partition, so a freshly
        // created one is never written to under the wrong regime.
        stepped.ensure_settled(first.1).await?;
      }
      // Read and decode run ahead of the writer from here; the writer below
      // still opens one transaction per block and applies them in the order
      // the queue hands them over, which is the order they were planned in.
      // Dropping the queue at any exit from this arm discards the look-ahead —
      // which is how a branch that turns out to be wrong is abandoned.
      let mut queue = DecodeQueue::spawn(
        pool.clone(),
        Arc::clone(data_src),
        segment.clone(),
        PrefetchCfg::resolved(),
      );
      let mut connected_to: Option<i32> = None;
      while let Some(item) = queue.next().await {
        let stumble = |e: PipelineError, height: i32| {
          // Everything up to the previous block is committed and visible;
          // publish what completed before surfacing the failure.
          if let Some(done) = connected_to {
            publish(done, HyDbStage::Idle);
            tracing::warn!(
              connected_through = done,
              failed_height = height,
              "advance stopped mid-segment; committed progress is preserved"
            );
          }
          flatten(e)
        };
        // A read or decode that failed arrives here at its own height, so a
        // hole in the look-ahead stops the advance instead of silently
        // shortening it.
        let block = match item {
          Ok(block) => block,
          Err(e) => return Err(stumble(e.into(), connected_to.map_or(-1, |h| h + 1))),
        };
        let height = block.height;
        publish(height, HyDbStage::Derive);
        if let Err(e) = connect_prefetched(pool, block).await {
          return Err(stumble(e, height));
        }
        connected_to = Some(height);
        publish(height, HyDbStage::Accumulate);
      }
      // ...and again after, which is where a partition the tip has just moved
      // past gets its deferred indexes built.
      if let Some(reached) = connected_to {
        stepped.ensure_settled(reached).await?;
      }
      finish_advance(pool, from, target, stepped).await
    }
    Plan::Reorg {
      fork_height,
      segment,
    } => {
      let from_height = from_tip.map(|(h, _)| h).unwrap_or(-1);
      let floor = fork_height.unwrap_or(-1);
      let max_reorg = max_reorg_depth();
      if i64::from(from_height - floor) > max_reorg {
        return Err(HyDbError::Derive(format!(
          "reorg from height {from_height} to fork {floor} exceeds the supported depth ({max_reorg})"
        )));
      }
      // A disconnect runs the UTXO-restore join on `idx_tx_outputs_outpoint`,
      // which stepped mode has taken off the open partition. Stepped mode is
      // predicated on a reversal being unreachable from this far behind the
      // tip; a reorg says that premise no longer holds, so leave the mode and
      // put the index back before unwinding anything.
      if stepped.force_realtime("a reorg was planned").is_some() {
        tuning::apply_storage_for(stepped.admin(), tuning::resolved(), false).await?;
      }
      stepped.ensure_settled(from_height.max(0)).await?;

      let segment: Vec<(HyDbHash, i32)> = segment.into_iter().take(max_advance_blocks()).collect();
      if let Some(last) = segment.last() {
        chain::ensure_partitions_for_range(pool, floor.max(0), last.1).await?;
      }

      // One transaction for the whole span: disconnect tip-first down to the
      // fork, then derive + connect the replacement branch. If anything fails,
      // the old canonical chain remains untouched.
      let mut tx = pool
        .begin()
        .await
        .map_err(|e| HyDbError::pg("reorg: begin", e))?;
      let mut disconnected: Vec<HyDbHash> = Vec::new();
      let mut h = from_height;
      while h > floor {
        let hash = disconnect_block_in(&mut tx, h).await?;
        disconnected.push(hash);
        h -= 1;
      }
      for (hash, height) in &segment {
        connect_block_in(&mut tx, data_src.as_ref(), *height, hash)
          .await
          .map_err(flatten)?;
      }
      tx.commit()
        .await
        .map_err(|e| HyDbError::pg("reorg: commit", e))?;

      let reorg = (!disconnected.is_empty()).then(|| HyDbReorg {
        fork_height: HyDbHeight(floor.max(0) as u32),
        disconnected,
        full_rederive: false,
      });
      let mut report = finish_advance(pool, from, target, stepped).await?;
      report.reorg = reorg;
      Ok(report)
    }
  }
}

/// Operator-initiated rewind: disconnect canonical blocks, tip-first, until
/// the tip is at `target`.
pub(super) async fn rewind(
  pool: &PgPool,
  target: HyDbHeight,
  stepped: &SteppedSync,
) -> HyDbResult<HyDbTipReport> {
  let from_tip = chain::canonical_tip(pool).await?;
  let from = from_tip
    .map(|(h, _)| HyDbHeight::try_from_i32(h))
    .transpose()?;
  let floor = target.0 as i32;

  // Same argument as the reorg path: a rewind disconnects, and a disconnect
  // needs the index stepped mode is holding back.
  if stepped
    .force_realtime("an operator rewind was requested")
    .is_some()
  {
    tuning::apply_storage_for(stepped.admin(), tuning::resolved(), false).await?;
  }
  if let Some((tip, _)) = from_tip {
    stepped.ensure_settled(tip).await?;
  }

  let mut disconnected: Vec<HyDbHash> = Vec::new();
  let mut h = from_tip.map(|(h, _)| h).unwrap_or(-1);
  while h > floor {
    let mut tx = pool
      .begin()
      .await
      .map_err(|e| HyDbError::pg("rewind: begin", e))?;
    let hash = disconnect_block_in(&mut tx, h).await?;
    tx.commit()
      .await
      .map_err(|e| HyDbError::pg("rewind: commit", e))?;
    disconnected.push(hash);
    h -= 1;
  }

  let to_tip = chain::canonical_tip(pool).await?;
  let to = to_tip
    .map(|(h, _)| HyDbHeight::try_from_i32(h))
    .transpose()?;
  let reorg = (!disconnected.is_empty()).then(|| HyDbReorg {
    fork_height: HyDbHeight(floor.max(0) as u32),
    disconnected,
    full_rederive: false,
  });
  Ok(HyDbTipReport {
    from,
    to,
    target: Some(target),
    reorg,
    mode: stepped.mode(),
  })
}

/// Decide what an advance toward `node_tip` must do.
async fn plan_advance(
  pool: &PgPool,
  from_tip: &Option<(i32, HyDbHash)>,
  node_tip: &HyDbHash,
  node_height: i32,
) -> HyDbResult<Plan> {
  let t = from_tip.as_ref().map(|(h, _)| *h).unwrap_or(-1);

  // Already at (or matching) the tip?
  if node_height <= t {
    if let Some(hash) = chain::canonical_hash_at(pool, node_height).await? {
      if hash == *node_tip {
        // The node's tip is on our canonical chain. If it is *below* our tip
        // the node itself reorged shorter — fall through to the walk, which
        // will disconnect down to it. Same-height match means nothing to do.
        if node_height == t {
          return Ok(Plan::Noop);
        }
      }
    }
    return walk_plan(pool, node_tip).await;
  }

  // Forward fast path: the segment (t, min(node_height, t + MAX_SEGMENT)] has
  // exactly one absorbed candidate per height and links hash-to-hash down to
  // our canonical tip. One range read, no per-block walk. Any ambiguity or
  // linkage failure (including a same-height fork hiding below) falls back to
  // the bounded SQL walk from the node tip.
  let seg_hi = node_height.min(t.saturating_add(max_advance_blocks() as i32));
  let rows = chain::ser_rows_in_range(pool, t + 1, seg_hi).await?;
  let expected_len = (seg_hi - t) as usize;
  let mut ok = rows.len() == expected_len;
  if ok {
    let mut prev_hash: [u8; 32] = match from_tip {
      Some((_, hash)) => hash.0,
      None => chain::GENESIS_PREV,
    };
    for (i, row) in rows.iter().enumerate() {
      if row.height != t + 1 + i as i32 || (t >= 0 || i > 0) && row.prev_hash != prev_hash {
        ok = false;
        break;
      }
      // For an empty projection (t == -1) the first row must be genesis.
      if t < 0 && i == 0 && (row.height != 0 || row.prev_hash != chain::GENESIS_PREV) {
        ok = false;
        break;
      }
      prev_hash = row.hash.0;
    }
    if ok {
      return Ok(Plan::Forward(
        rows.into_iter().map(|r| (r.hash, r.height)).collect(),
      ));
    }
  }
  walk_plan(pool, node_tip).await
}

/// The general path: bounded parent walk from the node tip down to the fork.
async fn walk_plan(pool: &PgPool, node_tip: &HyDbHash) -> HyDbResult<Plan> {
  match chain::walk_to_fork(pool, node_tip, walk_bound()).await? {
    chain::WalkOutcome::AlreadyCanonical | chain::WalkOutcome::MissingAncestor => Ok(Plan::Noop),
    chain::WalkOutcome::DepthExceeded => Err(HyDbError::Derive(format!(
      "fork point deeper than the supported walk bound ({} + {})",
      max_reorg_depth(),
      max_advance_blocks()
    ))),
    chain::WalkOutcome::Segment {
      fork_height,
      segment,
    } => Ok(Plan::Reorg {
      fork_height,
      segment,
    }),
  }
}

/// Read the post-advance tip, prune stale undo rows, and build the report.
async fn finish_advance(
  pool: &PgPool,
  from: Option<HyDbHeight>,
  target: Option<HyDbHeight>,
  stepped: &SteppedSync,
) -> HyDbResult<HyDbTipReport> {
  let to_tip = chain::canonical_tip(pool).await?;
  let to = to_tip
    .map(|(h, _)| HyDbHeight::try_from_i32(h))
    .transpose()?;
  if let Some((tip, _)) = to_tip {
    chain::prune_undo(pool, tip - undo_keep_depth()).await?;
  }
  Ok(HyDbTipReport {
    from,
    to,
    target,
    reorg: None,
    mode: stepped.mode(),
  })
}

/// Route a connect failure before flattening to a public error: transient
/// datastore faults are worth the caller's retry (warn), a `MissingPrevout`
/// means the UTXO projection is inconsistent and needs operator attention
/// (error) — the deep-rederive recovery that could repair it automatically is
/// deliberately not implemented, so it surfaces as a fatal derive error.
fn flatten(e: PipelineError) -> HyDbError {
  if e.recoverable() {
    tracing::error!(
      "block connect hit a missing prevout — the UTXO projection is inconsistent; \
       rewind past the affected range or re-derive from ser_blocks"
    );
  } else if e.transient() {
    tracing::warn!("block connect hit a transient datastore fault; the caller should retry");
  }
  e.into()
}

// ===========================================================================
// Connect one block: derive read-only, then run the stages forward
// ===========================================================================

/// Connect one prefetched block in its own transaction (the forward path).
///
/// The transaction opens with the block already decoded, which is the whole
/// point of A4: the only thing between `BEGIN` and `COMMIT` is work that needs
/// the transaction.
async fn connect_prefetched(pool: &PgPool, block: Prefetched) -> Result<(), PipelineError> {
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| HyDbError::pg("connect: begin", e))?;
  connect_decoded_in(&mut tx, block.height, block.raw_len, block.block).await?;
  tx.commit()
    .await
    .map_err(|e| HyDbError::pg("connect: commit", e))?;
  Ok(())
}

/// Read + decode + connect inside a caller-owned transaction. The reorg path's
/// entry point: a reorg replaces a branch inside ONE transaction, so its
/// blocks cannot be read ahead of a writer that has not started yet, and
/// decoding them inline is the correct trade for a rare, atomic operation.
async fn connect_block_in(
  tx: &mut PgTx<'_>,
  data_src: &dyn HyDbDataSrc,
  height: i32,
  hash: &HyDbHash,
) -> Result<(), PipelineError> {
  let raw = chain::raw_bytes(&mut **tx, hash).await?;
  let raw_len = raw.len() as i64;
  let block = data_src
    .decode(&raw)
    .map_err(|e| HyDbDeriveError::Decode(e.to_string()))?;
  connect_decoded_in(tx, height, raw_len, block).await
}

/// **The** write path for a connect. Both the forward look-ahead and the reorg
/// replay land here, so there is exactly one place that applies a block —
/// they differ only in where the decoded block came from.
async fn connect_decoded_in(
  tx: &mut PgTx<'_>,
  height: i32,
  raw_len: i64,
  block: HyDbBlock,
) -> Result<(), PipelineError> {
  let mut scratch = Scratch::new(height, raw_len);
  PgDeriver.derive(tx, block, &mut scratch).await?;

  base::BaseStage.connect(tx, &scratch).await?;
  balances::BalancesStage.connect(tx, &scratch).await?;
  stats::StatsStage.connect(tx, &scratch).await?;
  validators::ValidatorsStage.connect(tx, &scratch).await?;
  Ok(())
}

// ===========================================================================
// Disconnect: run the stages in reverse
// ===========================================================================

/// Reverse the connect at `height` inside a caller-owned transaction by
/// replaying the stages in reverse order, each restoring its slice from the
/// undo rows it captured. Returns the disconnected block's hash.
async fn disconnect_block_in(tx: &mut PgTx<'_>, height: i32) -> HyDbResult<HyDbHash> {
  let hash = chain::canonical_hash_at(&mut **tx, height)
    .await?
    .ok_or_else(|| {
      HyDbError::Derive(format!(
        "no canonical block at height {height} to disconnect"
      ))
    })?;

  // Reverse of the connect order (see the stage-set note above).
  validators::ValidatorsStage.disconnect(tx, height).await?;
  stats::StatsStage.disconnect(tx, height).await?;
  balances::BalancesStage.disconnect(tx, height).await?;
  base::BaseStage.disconnect(tx, height).await?;
  Ok(hash)
}
