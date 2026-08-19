//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The single-writer ingest handle, shaped as an event sink so the daemon can
//! wire it straight into node announcements (ZMQ `rawblock`/`hashblock`, or the
//! REST poller). The daemon forwards events; the crate reacts, owning
//! canonicalization, derivation, spend resolution, accumulation, undo, and
//! reorg recovery internally.

mod balances;
mod base;
mod chain;
mod derive;
mod pipeline;
mod scratch;
mod stats;
mod validators;

use core::fmt;
use std::sync::Arc;

use bytes::Bytes;
use sqlx::PgPool;
use tokio::sync::watch;

use crate::error::HyDbResult;
use crate::records::HyDbDataSrc;
use crate::stepped::{HyDbSyncMode, SteppedSync};
use crate::types::{HyDbHash, HyDbHeight};

/// Outcome of absorbing a single block announcement.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyDbBlockSeen {
  /// A block we had not seen; now stored in `ser_blocks`.
  New,
  /// Already present; no work done.
  Known,
  /// Stored, but its parent is missing — the daemon should fetch `.0` next so
  /// the chain can be connected. The crate owns ordering; the daemon owns node
  /// I/O.
  MissingParent(HyDbHash),
}

/// The single-writer ingest handle. Constructed via
/// [`crate::HyphenDb::ingest`], guarded by a Postgres advisory lock so exactly
/// one exists across all processes. The lock lives on `_lock_conn`, a detached
/// session held for the handle's lifetime; dropping the handle closes the
/// session and releases the lock.
pub struct HyDbIngest {
  data_src: Arc<dyn HyDbDataSrc>,
  write: PgPool,
  progress_tx: watch::Sender<HyDbProgress>,
  progress_rx: watch::Receiver<HyDbProgress>,
  /// Serializes mutating operations within this process (`on_block`/`on_tip`
  /// vs. an operator-initiated `rewind_to`); the advisory lock only guards
  /// cross-process exclusivity.
  op_lock: tokio::sync::Mutex<()>,
  /// Stepped-sync state machine and the per-partition index maintenance it
  /// drives. Owned here rather than kept process-wide because the ingest
  /// advisory lock already makes this handle the single writer, and a mode is
  /// only meaningful relative to one write path.
  stepped: Arc<SteppedSync>,
  _lock_conn: sqlx::PgConnection,
}

impl HyDbIngest {
  /// A block announcement arrived (ZMQ `rawblock`, or fetched by hash). Absorbs
  /// it into `ser_blocks`; the caller drives canonicalization via
  /// [`on_tip`](Self::on_tip). Assigns the block's height as `parent.height +
  /// 1` (genesis is 0); a block whose parent is unknown yields
  /// [`HyDbBlockSeen::MissingParent`] and is not stored — height is mandatory.
  pub async fn on_block(&self, raw: Bytes) -> HyDbResult<HyDbBlockSeen> {
    let _op = self.op_lock.lock().await;
    self.publish_stage(HyDbStage::Absorb);
    pipeline::absorb(&self.write, self.data_src.as_ref(), &raw).await
  }

  /// The node's reported tip changed (ZMQ `hashblock`, or a poll). Selects the
  /// canonical chain ending at `node_tip` and advances the projection toward
  /// it, disconnecting and re-deriving the affected range on a fork. The
  /// advance is *bounded*: at most `[db.ingest] max_advance_blocks` blocks are
  /// connected per call, so a cold sync reports progress continuously and the
  /// caller simply calls again (`report.to < target`) to resume deeper.
  pub async fn on_tip(&self, node_tip: HyDbHash) -> HyDbResult<HyDbTipReport> {
    let _op = self.op_lock.lock().await;
    let report = pipeline::advance(
      &self.write,
      self.data_src.as_ref(),
      node_tip,
      &self.progress_tx,
      &self.stepped,
    )
    .await?;
    self.publish(HyDbProgress {
      stage: HyDbStage::Idle,
      height: report.to,
      target: report.target,
      mode: self.stepped.mode(),
    });
    Ok(report)
  }

  /// Operator-initiated rewind: disconnect canonical blocks until the tip is
  /// at `target` (a no-op if the tip is already at or below it). The blocks'
  /// raw bytes stay in `ser_blocks`, so a later [`on_tip`](Self::on_tip)
  /// reconnects them.
  pub async fn rewind_to(&self, target: HyDbHeight) -> HyDbResult<HyDbTipReport> {
    let _op = self.op_lock.lock().await;
    let report = pipeline::rewind(&self.write, target, &self.stepped).await?;
    self.publish(HyDbProgress {
      stage: HyDbStage::Idle,
      height: report.to,
      target: report.target,
      mode: self.stepped.mode(),
    });
    Ok(report)
  }

  /// Watch channel of ingest progress for the TUI/status endpoints.
  pub fn progress(&self) -> watch::Receiver<HyDbProgress> {
    self.progress_rx.clone()
  }

  /// The index-maintenance regime in force. Surfaced by `/api/v1/status` and
  /// the TUI, so an operator can see that a sync is deferring index builds
  /// rather than inferring it from throughput.
  pub fn sync_mode(&self) -> HyDbSyncMode {
    self.stepped.mode()
  }

  /// Watch the mode, for callers that would otherwise poll it.
  pub fn sync_mode_watch(&self) -> watch::Receiver<HyDbSyncMode> {
    self.stepped.subscribe()
  }

  pub(crate) fn new(
    data_src: Arc<dyn HyDbDataSrc>,
    write: PgPool,
    admin: PgPool,
    lock_conn: sqlx::PgConnection,
  ) -> Self {
    let (progress_tx, progress_rx) = watch::channel(HyDbProgress::default());
    Self {
      data_src,
      write,
      progress_tx,
      progress_rx,
      op_lock: tokio::sync::Mutex::new(()),
      stepped: Arc::new(SteppedSync::from_config(admin)),
      _lock_conn: lock_conn,
    }
  }

  fn publish(&self, progress: HyDbProgress) {
    let _ = self.progress_tx.send(progress);
  }

  fn publish_stage(&self, stage: HyDbStage) {
    let cur = *self.progress_rx.borrow();
    self.publish(HyDbProgress { stage, ..cur });
  }
}

impl fmt::Debug for HyDbIngest {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.debug_struct("HyDbIngest").finish()
  }
}

/// A snapshot of ingest progress.
#[derive(Debug, Clone, Copy)]
pub struct HyDbProgress {
  pub stage: HyDbStage,
  /// Height the projection has reached.
  pub height: Option<HyDbHeight>,
  /// Height the node reports as its tip (the target).
  pub target: Option<HyDbHeight>,
  /// Index-maintenance regime the write path is in.
  pub mode: HyDbSyncMode,
}

impl Default for HyDbProgress {
  fn default() -> Self {
    Self {
      stage: HyDbStage::Idle,
      height: None,
      target: None,
      mode: HyDbSyncMode::Realtime,
    }
  }
}

/// Details of a reorganization performed while advancing.
#[derive(Debug, Clone)]
pub struct HyDbReorg {
  /// Height of the last block common to the old and new chains.
  pub fork_height: HyDbHeight,
  /// Hashes disconnected from the old chain, in disconnection order — tip
  /// first, deepest (closest to the fork point) last.
  pub disconnected: Vec<HyDbHash>,
  /// Whether recovery fell back to full re-derivation (deep fork) rather than
  /// bounded undo replay (shallow fork).
  pub full_rederive: bool,
}

/// Which pipeline stage the ingest is currently working through — surfaced for
/// the TUI/status progress feed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyDbStage {
  Absorb,
  Canonicalize,
  Derive,
  ResolveSpends,
  Accumulate,
  Idle,
}

/// What advancing toward a new tip did.
#[derive(Debug, Clone)]
pub struct HyDbTipReport {
  /// Projection height before this advance.
  pub from: Option<HyDbHeight>,
  /// Projection height after.
  pub to: Option<HyDbHeight>,
  /// Height of the node tip this advance was working toward, when known.
  /// `to < target` means the advance was segment-bounded — call again to
  /// resume deeper.
  pub target: Option<HyDbHeight>,
  /// Present when the advance replaced part of the canonical chain.
  pub reorg: Option<HyDbReorg>,
  /// Index-maintenance regime in force at the end of the advance.
  pub mode: HyDbSyncMode,
}
