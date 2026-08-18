//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block ingestion driver.
//!
//! The storage crate (`hyphen-db`) owns decode-scheduling, spend resolution,
//! accumulation, undo, and reorg recovery internally. This module is only the
//! driver: it polls the node for its tip, fetches new blocks, and forwards
//! them to the engine via [`hyphen_db::HyDbIngest::on_block`]/`on_tip`.
//! Missing ancestors the engine reports are fetched on demand, which is also
//! how a reorg's competing branch gets pulled in.
//!
//! Cold sync is *interleaved*: blocks are absorbed in bounded chunks and each
//! chunk is connected (`on_tip`) before the next is fetched, so the
//! projection — and the operator's progress view — advances continuously
//! instead of only after the entire chain has been absorbed.
//!
//! Within a chunk the fetch runs *ahead* of the absorb ([`prefetch`], task
//! A4): the node is answering the next block's two round trips while the
//! engine is still writing the previous one. The engine runs its own
//! read-and-decode look-ahead in front of the writer, so the two halves of the
//! seam are pipelined the same way and bounded by the same
//! `[db.ingest] max_lead_blocks`.

pub(crate) mod datasrc;
pub(crate) mod prefetch;

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use hyphen_db::{HyDbBlockSeen, HyDbHash, HyDbHeight, HyDbIngest, HyDbSyncMode, HyphenDb};
use tracing::{info, warn};

use crate::error::DaemonError;
use crate::lifecycle::Lifecycle;
use crate::sources::RestSource;
use crate::utils::{self, HashEndian};
use hyphend::wire::msg::{IngestionActivity, IngestionStateName};

/// Default seconds between tip polls.
pub(crate) const DEFAULT_POLL_SECS: u64 = 5;

/// Blocks absorbed per chunk before the engine is asked to connect them.
const ABSORB_CHUNK_BLOCKS: u32 = 512;

/// Ingest activity considered stalled after this many seconds without
/// progress while behind the node tip.
const STALL_AFTER_SECS: u64 = 120;

fn unix_now() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

/// Shared, lock-free view of the ingest loop for the IPC heartbeat.
#[derive(Debug)]
pub(crate) struct IngestStatus {
  state: AtomicU8,
  stalled: AtomicBool,
  network_tip: Arc<AtomicI64>,
  local_tip: AtomicI64,
  /// Index-maintenance regime the storage engine reports (task A3), shared
  /// with the REST status endpoint the same way the node tip is.
  sync_mode: Arc<AtomicU8>,
  blocks_ingested_total: AtomicU64,
  /// Blocks the fetch look-ahead is currently holding for the absorb (task
  /// A4). Surfaced so an operator can see whether the pipeline is running
  /// ahead or starved, rather than inferring it from throughput.
  lead_blocks: AtomicU64,
  last_progress_epoch: Arc<AtomicU64>,
  started_epoch: AtomicU64,
}

const STATE_BOOT: u8 = 0;
const STATE_POLLING: u8 = 1;
const STATE_FETCHING: u8 = 2;
const STATE_COMMITTING: u8 = 3;
const STATE_IDLE: u8 = 4;
const STATE_BACKOFF: u8 = 5;

/// Stepped-sync mode, encoded for the shared atomic. Kept as a `u8` rather
/// than a lock so the status endpoint can read it on any thread without
/// touching the ingest loop.
pub(crate) const SYNC_MODE_REALTIME: u8 = 0;
pub(crate) const SYNC_MODE_STEPPED: u8 = 1;

/// Decode the shared sync-mode atomic. Unknown values read as `realtime`,
/// which is the mode that promises nothing.
pub(crate) fn sync_mode_label(raw: u8) -> &'static str {
  if raw == SYNC_MODE_STEPPED {
    HyDbSyncMode::Stepped.as_str()
  } else {
    HyDbSyncMode::Realtime.as_str()
  }
}

impl IngestStatus {
  fn new() -> Self {
    Self {
      state: AtomicU8::new(STATE_BOOT),
      stalled: AtomicBool::new(false),
      network_tip: Arc::new(AtomicI64::new(-1)),
      local_tip: AtomicI64::new(-1),
      sync_mode: Arc::new(AtomicU8::new(SYNC_MODE_REALTIME)),
      blocks_ingested_total: AtomicU64::new(0),
      lead_blocks: AtomicU64::new(0),
      last_progress_epoch: Arc::new(AtomicU64::new(0)),
      started_epoch: AtomicU64::new(unix_now()),
    }
  }

  fn set_state(&self, state: u8) {
    self.state.store(state, Ordering::Relaxed);
  }

  fn mark_progress(&self) {
    self
      .last_progress_epoch
      .store(unix_now(), Ordering::Relaxed);
    self.stalled.store(false, Ordering::Relaxed);
  }

  fn update_stall(&self) {
    let behind = self.local_tip.load(Ordering::Relaxed) < self.network_tip.load(Ordering::Relaxed);
    let last = self.last_progress_epoch.load(Ordering::Relaxed);
    let reference = if last == 0 {
      self.started_epoch.load(Ordering::Relaxed)
    } else {
      last
    };
    let stalled = behind && unix_now().saturating_sub(reference) >= STALL_AFTER_SECS;
    self.stalled.store(stalled, Ordering::Relaxed);
  }

  pub(crate) fn state_name(&self) -> IngestionStateName {
    match self.state.load(Ordering::Relaxed) {
      STATE_POLLING => IngestionStateName::PollingTip,
      STATE_FETCHING => IngestionStateName::Fetching,
      STATE_COMMITTING => IngestionStateName::Committing,
      STATE_IDLE => IngestionStateName::Idle,
      STATE_BACKOFF => IngestionStateName::Backoff,
      _ => IngestionStateName::Boot,
    }
  }

  pub(crate) fn activity(&self) -> IngestionActivity {
    match self.state.load(Ordering::Relaxed) {
      STATE_FETCHING | STATE_COMMITTING => IngestionActivity::Active,
      STATE_IDLE => IngestionActivity::Idle,
      _ => IngestionActivity::Sleeping,
    }
  }

  pub(crate) fn stalled(&self) -> bool {
    self.stalled.load(Ordering::Relaxed)
  }

  pub(crate) fn network_tip(&self) -> Option<u32> {
    u32::try_from(self.network_tip.load(Ordering::Relaxed)).ok()
  }

  pub(crate) fn local_tip(&self) -> Option<u32> {
    u32::try_from(self.local_tip.load(Ordering::Relaxed)).ok()
  }

  pub(crate) fn blocks_ingested_total(&self) -> u64 {
    self.blocks_ingested_total.load(Ordering::Relaxed)
  }

  /// Blocks the fetch look-ahead is holding right now.
  pub(crate) fn lead_blocks(&self) -> u32 {
    u32::try_from(self.lead_blocks.load(Ordering::Relaxed)).unwrap_or(u32::MAX)
  }

  /// The configured ceiling on that lead, `[db.ingest] max_lead_blocks`.
  pub(crate) fn max_lead_blocks(&self) -> u32 {
    u32::try_from(hyphen_db::tuning::max_lead_blocks()).unwrap_or(u32::MAX)
  }

  pub(crate) fn last_progress_epoch(&self) -> Option<u64> {
    let epoch = self.last_progress_epoch.load(Ordering::Relaxed);
    (epoch > 0).then_some(epoch)
  }

  /// Shared handle to the node tip observed by polling (-1 = unknown), for
  /// the REST status endpoint.
  pub(crate) fn sync_mode(&self) -> &'static str {
    sync_mode_label(self.sync_mode.load(Ordering::Relaxed))
  }

  /// Shared handle to the index-maintenance regime, for the REST status
  /// endpoint.
  pub(crate) fn sync_mode_handle(&self) -> Arc<AtomicU8> {
    Arc::clone(&self.sync_mode)
  }

  pub(crate) fn network_tip_handle(&self) -> Arc<AtomicI64> {
    Arc::clone(&self.network_tip)
  }

  /// Shared handle to the last-progress epoch (0 = never), for the REST
  /// status endpoint.
  pub(crate) fn progress_epoch_handle(&self) -> Arc<AtomicU64> {
    Arc::clone(&self.last_progress_epoch)
  }
}

/// Owns the background ingest loop.
pub(crate) struct IngestVisor {
  db: HyphenDb,
  ingest: Arc<HyDbIngest>,
  source: Arc<dyn RestSource>,
  lifecycle: Arc<Lifecycle>,
  status: Arc<IngestStatus>,
  poll_secs: u64,
  /// `[core] rest_concurrency`: outstanding REST requests the fetch
  /// look-ahead may have against the node at once.
  rest_concurrency: usize,
  task: Option<tokio::task::JoinHandle<()>>,
}

impl std::fmt::Debug for IngestVisor {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("IngestVisor").finish_non_exhaustive()
  }
}

impl IngestVisor {
  pub(crate) fn new(
    db: HyphenDb,
    ingest: HyDbIngest,
    source: Arc<dyn RestSource>,
    lifecycle: Arc<Lifecycle>,
    poll_secs: u64,
    rest_concurrency: usize,
  ) -> Self {
    Self {
      db,
      ingest: Arc::new(ingest),
      source,
      lifecycle,
      status: Arc::new(IngestStatus::new()),
      poll_secs: poll_secs.max(1),
      rest_concurrency: rest_concurrency.max(1),
      task: None,
    }
  }

  /// Shared status view for the IPC heartbeat.
  pub(crate) fn status(&self) -> Arc<IngestStatus> {
    Arc::clone(&self.status)
  }

  /// Shared ingest handle (rewind verbs go through the same single-writer
  /// handle; operations are serialized inside the engine).
  pub(crate) fn ingest_handle(&self) -> Arc<HyDbIngest> {
    Arc::clone(&self.ingest)
  }

  /// Watch channel of engine-side ingest progress.
  pub(crate) fn progress(&self) -> tokio::sync::watch::Receiver<hyphen_db::HyDbProgress> {
    self.ingest.progress()
  }

  /// Spawn the ingest loop.
  pub(crate) fn start(&mut self) {
    let db = self.db.clone();
    let ingest = Arc::clone(&self.ingest);
    let source = Arc::clone(&self.source);
    let lifecycle = Arc::clone(&self.lifecycle);
    let status = Arc::clone(&self.status);
    let poll = Duration::from_secs(self.poll_secs);
    let concurrency = self.rest_concurrency;
    self.task = Some(tokio::spawn(async move {
      info!("ingest loop started");
      while lifecycle.running() {
        match ingest_once(&db, &ingest, &source, concurrency, &status, &lifecycle).await {
          Ok(()) => status.set_state(STATE_IDLE),
          Err(error) => {
            warn!(%error, "ingest cycle failed");
            status.set_state(STATE_BACKOFF);
          }
        }
        status.update_stall();
        lifecycle.sleep(poll).await;
      }
      info!("ingest loop stopped");
    }));
  }

  /// Abort and join the ingest loop.
  pub(crate) async fn stop(&mut self) {
    if let Some(task) = self.task.take() {
      task.abort();
      let _ = task.await;
    }
  }
}

/// Advance the storage engine toward the node's current tip once,
/// interleaving bounded absorb chunks with engine connects.
async fn ingest_once(
  db: &HyphenDb,
  ingest: &HyDbIngest,
  source: &Arc<dyn RestSource>,
  concurrency: usize,
  status: &IngestStatus,
  lifecycle: &Lifecycle,
) -> Result<(), DaemonError> {
  status.set_state(STATE_POLLING);
  let node_height = source.block_count().await?;
  status
    .network_tip
    .store(i64::from(node_height), Ordering::Relaxed);

  let local = read_local_tip(db).await?;
  status.local_tip.store(local, Ordering::Relaxed);
  if i64::from(node_height) <= local {
    // Same-height / shorter reorgs are invisible to a pure height compare:
    // hand the node's tip hash to the engine, which compares hashes at the
    // fork walk and reorgs when they differ.
    let node_tip_hex = source.block_hash(node_height).await?;
    let raw = source.raw_block(&node_tip_hex).await?;
    let tip = block_hash(&raw)?;
    status.set_state(STATE_FETCHING);
    absorb_with_ancestors(ingest, source.as_ref(), raw).await?;
    status.set_state(STATE_COMMITTING);
    advance_until_settled(db, ingest, status, tip).await?;
    return Ok(());
  }

  let start = u32::try_from(local + 1).unwrap_or(0);
  let mut height = start;
  while height <= node_height && lifecycle.running() {
    let chunk_end = height
      .saturating_add(ABSORB_CHUNK_BLOCKS - 1)
      .min(node_height);
    status.set_state(STATE_FETCHING);
    let chunk_tip = absorb_chunk(
      ingest,
      source,
      concurrency,
      status,
      lifecycle,
      height,
      chunk_end,
    )
    .await?;
    if let Some(tip) = chunk_tip {
      status.set_state(STATE_COMMITTING);
      advance_until_settled(db, ingest, status, tip).await?;
    }
    height = chunk_end.saturating_add(1);
  }
  Ok(())
}

/// Absorb `[lo, hi]` with the fetch running ahead of the absorb, and return
/// the chunk tip's hash.
///
/// The queue is dropped — and with it every block still queued behind the
/// current one — the moment the engine reports a missing parent. That report
/// is the reorg seam: it means the engine cannot place this block on the chain
/// it has, so the heights queued behind it were fetched against an assumption
/// that no longer holds. They are refetched after the ancestor walk, from
/// wherever the walk left the chain, rather than absorbed from a stale queue.
async fn absorb_chunk(
  ingest: &HyDbIngest,
  source: &Arc<dyn RestSource>,
  concurrency: usize,
  status: &IngestStatus,
  lifecycle: &Lifecycle,
  lo: u32,
  hi: u32,
) -> Result<Option<HyDbHash>, DaemonError> {
  let mut next = lo;
  let mut chunk_tip: Option<HyDbHash> = None;
  while next <= hi {
    let mut queue = prefetch::FetchQueue::spawn(Arc::clone(source), concurrency, next, hi);
    let mut discarded = false;
    while let Some(item) = queue.next().await {
      if !lifecycle.running() {
        return Ok(None);
      }
      status
        .lead_blocks
        .store(queue.in_flight() as u64, Ordering::Relaxed);
      let block = item?;
      let height = block.height;
      if height == hi {
        chunk_tip = Some(block_hash(&block.raw)?);
      }
      next = height.saturating_add(1);
      if absorb_one(ingest, source.as_ref(), block.raw).await? {
        // An ancestor walk ran. Everything still queued was fetched on the
        // superseded assumption; discard and refill from here.
        discarded = true;
        break;
      }
    }
    if !discarded {
      break;
    }
  }
  status.lead_blocks.store(0, Ordering::Relaxed);
  Ok(chunk_tip)
}

/// Absorb one block, walking to fetch any ancestor the engine reports missing.
/// Returns whether a walk was needed — the caller treats that as a signal to
/// discard its look-ahead.
async fn absorb_one(
  ingest: &HyDbIngest,
  source: &dyn RestSource,
  raw: Vec<u8>,
) -> Result<bool, DaemonError> {
  match ingest
    .on_block(Bytes::from(raw.clone()))
    .await
    .map_err(|e| DaemonError::storage("absorb block", e))?
  {
    HyDbBlockSeen::New | HyDbBlockSeen::Known => Ok(false),
    HyDbBlockSeen::MissingParent(_) => {
      absorb_with_ancestors(ingest, source, raw).await?;
      Ok(true)
    }
  }
}

/// Drive `on_tip` until the projection reaches the engine's reported target
/// (each call connects a bounded segment).
async fn advance_until_settled(
  db: &HyphenDb,
  ingest: &HyDbIngest,
  status: &IngestStatus,
  tip: HyDbHash,
) -> Result<(), DaemonError> {
  loop {
    let report = ingest
      .on_tip(tip)
      .await
      .map_err(|e| DaemonError::storage("advance to tip", e))?;
    if let Some(reorg) = &report.reorg {
      info!(
        fork_height = reorg.fork_height.0,
        disconnected = reorg.disconnected.len(),
        "chain reorganization applied"
      );
    }
    let connected = blocks_connected(&report.from, &report.to);
    if connected > 0 {
      status
        .blocks_ingested_total
        .fetch_add(connected, Ordering::Relaxed);
      status.mark_progress();
    }
    if let Some(to) = report.to {
      status.local_tip.store(i64::from(to.0), Ordering::Relaxed);
    }
    status.sync_mode.store(
      match report.mode {
        HyDbSyncMode::Stepped => SYNC_MODE_STEPPED,
        HyDbSyncMode::Realtime => SYNC_MODE_REALTIME,
      },
      Ordering::Relaxed,
    );
    let done = match (report.to, report.target) {
      (Some(to), Some(target)) => to >= target,
      _ => true,
    };
    if done || connected == 0 {
      // Also stop when the engine made no progress, so a permanently
      // unconnectable tip cannot spin this loop.
      break;
    }
  }
  let _ = read_local_tip(db).await; // keep the read path warm; tip already tracked
  Ok(())
}

fn blocks_connected(from: &Option<HyDbHeight>, to: &Option<HyDbHeight>) -> u64 {
  match (from, to) {
    (Some(from), Some(to)) if to.0 > from.0 => u64::from(to.0 - from.0),
    (None, Some(to)) => u64::from(to.0) + 1,
    _ => 0,
  }
}

async fn read_local_tip(db: &HyphenDb) -> Result<i64, DaemonError> {
  Ok(
    db.status()
      .tip()
      .await
      .map_err(|e| DaemonError::storage("read canonical tip", e))?
      .map_or(-1i64, |tip| i64::from(tip.height.0)),
  )
}

/// Absorb a block, fetching (and absorbing) any ancestor the engine reports
/// missing before retrying — the seam that also pulls in a reorg's branch.
async fn absorb_with_ancestors(
  ingest: &HyDbIngest,
  source: &dyn RestSource,
  raw: Vec<u8>,
) -> Result<(), DaemonError> {
  let mut stack = vec![raw];
  while let Some(raw) = stack.pop() {
    match ingest
      .on_block(Bytes::from(raw.clone()))
      .await
      .map_err(|e| DaemonError::storage("absorb block", e))?
    {
      HyDbBlockSeen::MissingParent(parent) => {
        let parent_hex = utils::view_hex(parent.as_bytes(), HashEndian::Ser);
        let parent_raw = source.raw_block(&parent_hex).await?;
        stack.push(raw);
        stack.push(parent_raw);
      }
      HyDbBlockSeen::New | HyDbBlockSeen::Known => {}
    }
  }
  Ok(())
}

fn block_hash(raw: &[u8]) -> Result<HyDbHash, DaemonError> {
  // The inner error is already a `DaemonError`; re-formatting it into a new
  // one would flatten a typed failure into a sentence for no gain.
  crate::consensus::block_hash(raw).map(|h| HyDbHash(h.to_bytes()))
}
