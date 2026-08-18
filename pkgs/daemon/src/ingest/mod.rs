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

  // One look-ahead for the whole run, not one per chunk. A per-chunk queue
  // would only hide the fetch behind the absorb; this hides it behind the
  // *connect* as well, which is where nearly all the wall time is. While the
  // engine spends seconds writing a chunk, the node is answering the next
  // one's requests. `max_lead_blocks` is what stops that from being unbounded,
  // and is the reason production sets it to 2048 rather than to the chunk.
  let start = u32::try_from(local + 1).unwrap_or(0);
  let mut height = start;
  let mut queue = prefetch::FetchQueue::spawn(Arc::clone(source), concurrency, start, node_height);
  while height <= node_height && lifecycle.running() {
    let chunk_end = height
      .saturating_add(ABSORB_CHUNK_BLOCKS - 1)
      .min(node_height);
    status.set_state(STATE_FETCHING);
    let absorbed = absorb_upto(
      &mut queue,
      ingest,
      source.as_ref(),
      status,
      lifecycle,
      height,
      chunk_end,
    )
    .await?;
    if absorbed.discarded {
      // The engine could not place a block on the chain it has, so every
      // height still queued was fetched against an assumption that no longer
      // holds. Refill from where the ancestor walk left off.
      queue =
        prefetch::FetchQueue::spawn(Arc::clone(source), concurrency, absorbed.next, node_height);
    }
    if let Some(tip) = absorbed.chunk_tip {
      status.set_state(STATE_COMMITTING);
      advance_until_settled(db, ingest, status, tip).await?;
    }
    if absorbed.next <= height {
      break; // the queue ran dry early; nothing more to absorb this cycle
    }
    height = absorbed.next;
  }
  Ok(())
}

/// What one absorb pass through the look-ahead did.
struct Absorbed {
  /// Hash of the block at the chunk's last height, when it was reached — the
  /// tip the engine is then asked to connect toward.
  chunk_tip: Option<HyDbHash>,
  /// The next height still to absorb.
  next: u32,
  /// Whether an ancestor walk ran, which invalidates everything still queued.
  discarded: bool,
}

/// Absorb from the look-ahead up to and including `hi`.
///
/// Stops early — reporting `discarded` — when the engine answers
/// [`HyDbBlockSeen::MissingParent`]. That report is the reorg seam: the engine
/// is saying it cannot place this block on the chain it has, so the heights
/// queued behind it belong to a branch it has just rejected. The caller
/// discards the queue and refills rather than feeding it stale blocks.
async fn absorb_upto(
  queue: &mut prefetch::FetchQueue,
  ingest: &HyDbIngest,
  source: &dyn RestSource,
  status: &IngestStatus,
  lifecycle: &Lifecycle,
  lo: u32,
  hi: u32,
) -> Result<Absorbed, DaemonError> {
  absorb_upto_with(
    queue,
    |raw| absorb_one(ingest, source, raw),
    status,
    lifecycle,
    lo,
    hi,
  )
  .await
}

/// [`absorb_upto`] over a caller-supplied absorb step, so the discard policy
/// can be tested without a storage engine behind it. The step answers "did an
/// ancestor walk run", which is the only thing this loop reacts to.
async fn absorb_upto_with<F, Fut>(
  queue: &mut prefetch::FetchQueue,
  absorb: F,
  status: &IngestStatus,
  lifecycle: &Lifecycle,
  lo: u32,
  hi: u32,
) -> Result<Absorbed, DaemonError>
where
  F: Fn(Vec<u8>) -> Fut,
  Fut: std::future::Future<Output = Result<bool, DaemonError>>,
{
  // Seeded at `lo`, not at `hi + 1`: a queue that runs dry before the chunk
  // end has absorbed nothing further, and the caller must not step over the
  // heights it did not get.
  let mut out = Absorbed {
    chunk_tip: None,
    next: lo,
    discarded: false,
  };
  while let Some(item) = queue.next().await {
    if !lifecycle.running() {
      out.chunk_tip = None;
      return Ok(out);
    }
    status
      .lead_blocks
      .store(queue.in_flight() as u64, Ordering::Relaxed);
    let block = item?;
    let height = block.height;
    if height == hi {
      out.chunk_tip = Some(block_hash(&block.raw)?);
    }
    out.next = height.saturating_add(1);
    if absorb(block.raw).await? {
      out.discarded = true;
      return Ok(out);
    }
    if height >= hi {
      return Ok(out);
    }
  }
  Ok(out)
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

#[cfg(test)]
mod tests {
  use super::*;
  use async_trait::async_trait;
  use std::sync::atomic::AtomicUsize;
  use std::sync::Mutex;

  /// A REST source serving synthetic blocks, recording the heights it served.
  #[derive(Debug, Default)]
  struct CountingRest {
    served: Mutex<Vec<u32>>,
    calls: AtomicUsize,
  }

  #[async_trait]
  impl RestSource for CountingRest {
    async fn block_count(&self) -> Result<u32, DaemonError> {
      Ok(100_000)
    }

    async fn block_hash(&self, height: u32) -> Result<String, DaemonError> {
      Ok(format!("{height:064x}"))
    }

    async fn raw_block(&self, hash: &str) -> Result<Vec<u8>, DaemonError> {
      let height = u32::from_str_radix(hash.trim_start_matches('0'), 16).unwrap_or(0);
      self.calls.fetch_add(1, Ordering::Relaxed);
      self
        .served
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(height);
      // 80 bytes so `block_hash` on the absorbed bytes is satisfiable.
      let mut raw = vec![0u8; 96];
      raw[..4].copy_from_slice(&height.to_le_bytes());
      Ok(raw)
    }

    async fn fetch_blocks_batch(
      &self,
      heights: &[u32],
    ) -> Result<Vec<(String, Vec<u8>)>, DaemonError> {
      let mut out = Vec::new();
      for h in heights {
        let hash = self.block_hash(*h).await?;
        out.push((hash.clone(), self.raw_block(&hash).await?));
      }
      Ok(out)
    }
  }

  fn height_of(raw: &[u8]) -> u32 {
    u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]])
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn a_missing_parent_stops_the_absorb_and_reports_where_to_refill() -> Result<(), DaemonError>
  {
    let src = Arc::new(CountingRest::default()) as Arc<dyn RestSource>;
    let status = IngestStatus::new();
    let lifecycle = Lifecycle::start();
    let mut queue = prefetch::FetchQueue::spawn(Arc::clone(&src), 8, 1, 5_000);

    // The engine accepts everything up to 40, then reports a missing parent —
    // the seam a competing branch arrives through.
    let seen = Arc::new(Mutex::new(Vec::new()));
    let absorbed = {
      let seen = Arc::clone(&seen);
      absorb_upto_with(
        &mut queue,
        move |raw: Vec<u8>| {
          let seen = Arc::clone(&seen);
          async move {
            let h = height_of(&raw);
            seen.lock().unwrap_or_else(|e| e.into_inner()).push(h);
            Ok(h == 40)
          }
        },
        &status,
        &lifecycle,
        1,
        512,
      )
      .await?
    };

    assert!(absorbed.discarded, "a missing parent must stop the absorb");
    assert_eq!(
      absorbed.next, 41,
      "the caller refills from the height after the one that walked, not from \
       the chunk start and not from the queue's own position"
    );
    assert!(
      absorbed.chunk_tip.is_none(),
      "the chunk end was never reached, so there is no tip to connect toward"
    );
    let seen = seen.lock().unwrap_or_else(|e| e.into_inner()).clone();
    assert_eq!(
      seen,
      (1..=40).collect::<Vec<_>>(),
      "the absorb must have consumed 1..=40 in order and stopped dead at 40"
    );
    Ok(())
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn a_clean_chunk_reports_its_tip_and_the_next_height() -> Result<(), DaemonError> {
    let src = Arc::new(CountingRest::default()) as Arc<dyn RestSource>;
    let status = IngestStatus::new();
    let lifecycle = Lifecycle::start();
    let mut queue = prefetch::FetchQueue::spawn(Arc::clone(&src), 8, 1, 5_000);

    let absorbed = absorb_upto_with(
      &mut queue,
      |_raw| async { Ok(false) },
      &status,
      &lifecycle,
      1,
      512,
    )
    .await?;
    assert!(!absorbed.discarded);
    assert_eq!(absorbed.next, 513);
    assert!(
      absorbed.chunk_tip.is_some(),
      "reaching the chunk end must yield the tip the engine is asked to \
       connect toward"
    );

    // The same queue keeps serving the next chunk — it spans the run, so the
    // fetch carries on while the engine writes.
    let more = absorb_upto_with(
      &mut queue,
      |_raw| async { Ok(false) },
      &status,
      &lifecycle,
      513,
      1024,
    )
    .await?;
    assert_eq!(more.next, 1025);
    assert!(!more.discarded);
    Ok(())
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn a_stopped_lifecycle_absorbs_nothing_further() -> Result<(), DaemonError> {
    let src = Arc::new(CountingRest::default()) as Arc<dyn RestSource>;
    let status = IngestStatus::new();
    let lifecycle = Lifecycle::start();
    lifecycle.stop();
    let mut queue = prefetch::FetchQueue::spawn(Arc::clone(&src), 8, 1, 5_000);
    let absorbed = absorb_upto_with(
      &mut queue,
      |_raw| async { Ok(false) },
      &status,
      &lifecycle,
      1,
      512,
    )
    .await?;
    assert!(
      absorbed.chunk_tip.is_none(),
      "a shutdown must not hand back a tip to connect toward"
    );
    assert_eq!(
      absorbed.next, 1,
      "and must not claim progress the caller can step over"
    );
    Ok(())
  }
}
