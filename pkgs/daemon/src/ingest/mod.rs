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

pub(crate) mod datasrc;

use std::sync::atomic::{AtomicBool, AtomicI64, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::Duration;

use bytes::Bytes;
use hyphen_db::{HyDbBlockSeen, HyDbHash, HyDbHeight, HyDbIngest, HyDbSyncMode, HyphenDb};
use tracing::{error, info, warn};

use crate::error::{DaemonError, Outcome};
use crate::lifecycle::Lifecycle;
use crate::sources::{RestSource, RpcSource};
use crate::utils::{self, HashEndian};
use hyphend::wire::msg::{IngestionActivity, IngestionStateName};

/// Default seconds between tip polls.
pub(crate) const DEFAULT_POLL_SECS: u64 = 5;

/// Blocks absorbed per chunk before the engine is asked to connect them.
const ABSORB_CHUNK_BLOCKS: u32 = 512;

/// Seconds between refreshes of the node's chain-wide transaction count.
const CHAIN_TX_STATS_REFRESH_SECS: u64 = 300;

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
  last_progress_epoch: Arc<AtomicU64>,
  started_epoch: AtomicU64,
  /// Transactions absorbed by this process since it started.
  transactions_ingested_total: AtomicU64,
  /// The node's own chain-wide transaction count, from `getchaintxstats`
  /// (0 = not known: RPC disabled or the call failed).
  network_transactions_total: AtomicU64,
  /// Transactions already in the database when this process started, counted
  /// once (0 until `transactions_baseline_known`).
  transactions_baseline: AtomicU64,
  transactions_baseline_known: AtomicBool,
  /// Smoothed transactions-per-block, in thousandths, over the blocks this
  /// process has absorbed. Chain density rises roughly 6x from genesis to the
  /// mainnet tip, so a projection needs the *recent* rate, not the average.
  txs_per_block_milli: AtomicU64,
  /// Set when a non-transient ingest failure has parked the loop. Ingestion
  /// stays down until an operator command clears it, which is the difference
  /// between "wedged, needs a human" and "briefly busy". Shared as an
  /// `Arc<AtomicBool>` so the IPC command handler can clear it without
  /// reaching into the ingest loop (same idiom as `network_tip`).
  operator_pause_requested: Arc<AtomicBool>,
  /// Consecutive transient failures driving the backoff ladder (0 = healthy).
  retry_attempt: AtomicU32,
  /// Epoch at which the current backoff sleep ends (0 = not backing off).
  backoff_until_epoch: AtomicU64,
}

const STATE_BOOT: u8 = 0;
const STATE_POLLING: u8 = 1;
const STATE_FETCHING: u8 = 2;
const STATE_COMMITTING: u8 = 3;
const STATE_IDLE: u8 = 4;
const STATE_BACKOFF: u8 = 5;
const STATE_PAUSED: u8 = 6;

/// Ceiling on the transient-failure backoff sleep.
///
/// The healthy path sleeps `poll_secs`; a transient failure doubles from there
/// so that a Postgres restart or an exhausted pool is not hammered at poll
/// cadence, and stops doubling here.
const MAX_BACKOFF_SECS: u64 = 300;

/// Transactions in a serialized block, from the CompactSize count that
/// follows the 80-byte header.
///
/// Deliberately not a full decode: the engine decodes the block anyway, and
/// this only has to be cheap and right. `None` for anything too short to hold
/// a count, which the absorb path will reject on its own terms.
fn block_tx_count(raw: &[u8]) -> Option<u64> {
  let (&first, rest) = raw.get(80..)?.split_first()?;
  match first {
    0xFD => rest
      .get(..2)
      .map(|b| u64::from(u16::from_le_bytes([b[0], b[1]]))),
    0xFE => rest
      .get(..4)
      .map(|b| u64::from(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))),
    0xFF => rest
      .get(..8)
      .map(|b| u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])),
    count => Some(u64::from(count)),
  }
}

/// What the ingest loop does about a failed cycle.
///
/// One step removed from [`Outcome`] so the decision is testable without a
/// storage engine, a node, or a running loop behind it — which is what let the
/// original handling be deleted unnoticed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FailureAction {
  /// Unrecoverable: stop ingesting and take the daemon down with it.
  Stop,
  /// Permanent but survivable: park ingestion for an operator, keep the
  /// daemon (and its REST API) serving what is already indexed.
  Pause,
  /// Transient: sleep on the backoff ladder and try the same work again.
  Backoff,
}

/// Map a failed cycle onto its action.
///
/// The severity itself is *not* decided here: `classify()` delegates to the
/// storage crate, which owns the SQLSTATE policy. This is only the translation
/// from severity to loop behaviour.
fn failure_action(error: &DaemonError) -> FailureAction {
  match error.classify() {
    Outcome::Fatal => FailureAction::Stop,
    Outcome::Error => FailureAction::Pause,
    Outcome::Warning => FailureAction::Backoff,
  }
}

/// Backoff sleep for the `attempt`-th consecutive transient failure
/// (1-based): `poll_secs * 2^(attempt-1)`, capped at [`MAX_BACKOFF_SECS`].
fn backoff_delay(poll_secs: u64, attempt: u32) -> Duration {
  let shift = attempt.saturating_sub(1).min(32);
  let scaled = poll_secs.saturating_mul(1u64 << shift);
  Duration::from_secs(scaled.clamp(1, MAX_BACKOFF_SECS))
}

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
      last_progress_epoch: Arc::new(AtomicU64::new(0)),
      started_epoch: AtomicU64::new(unix_now()),
      transactions_ingested_total: AtomicU64::new(0),
      network_transactions_total: AtomicU64::new(0),
      transactions_baseline: AtomicU64::new(0),
      transactions_baseline_known: AtomicBool::new(false),
      txs_per_block_milli: AtomicU64::new(0),
      operator_pause_requested: Arc::new(AtomicBool::new(false)),
      retry_attempt: AtomicU32::new(0),
      backoff_until_epoch: AtomicU64::new(0),
    }
  }

  fn set_state(&self, state: u8) {
    self.state.store(state, Ordering::Relaxed);
  }

  /// Record one absorbed block's transaction count (task F2).
  fn note_block_txs(&self, txs: u64) {
    self
      .transactions_ingested_total
      .fetch_add(txs, Ordering::Relaxed);
    // Exponential moving average in thousandths, alpha = 1/256: long enough to
    // ignore one empty block, short enough to track the density climb.
    let sample = txs.saturating_mul(1_000);
    let previous = self.txs_per_block_milli.load(Ordering::Relaxed);
    let next = if previous == 0 {
      sample
    } else {
      let delta = (sample as i64 - previous as i64) / 256;
      previous.saturating_add_signed(delta)
    };
    self.txs_per_block_milli.store(next, Ordering::Relaxed);
  }

  /// Record the node's chain-wide transaction count.
  fn set_network_transactions_total(&self, total: u64) {
    self
      .network_transactions_total
      .store(total, Ordering::Relaxed);
  }

  /// Record the transaction count already in the database at startup.
  fn set_transactions_baseline(&self, total: u64) {
    self.transactions_baseline.store(total, Ordering::Relaxed);
    self
      .transactions_baseline_known
      .store(true, Ordering::Relaxed);
  }

  /// Transactions absorbed by this process.
  pub(crate) fn transactions_ingested_total(&self) -> u64 {
    self.transactions_ingested_total.load(Ordering::Relaxed)
  }

  /// Smoothed transactions per block over the blocks this process absorbed.
  pub(crate) fn txs_per_block_recent(&self) -> Option<f64> {
    let milli = self.txs_per_block_milli.load(Ordering::Relaxed);
    (milli > 0).then(|| milli as f64 / 1_000.0)
  }

  /// Sync progress by block *and* by transaction (task F2).
  ///
  /// A monitored run read "36% complete" while roughly 6% complete by
  /// transaction, and every projection made from the block figure was wrong:
  /// mainnet is ~2.52M blocks but ~70.5M transactions, and density rises about
  /// sixfold across the chain, so the remaining blocks are the expensive ones.
  /// Both numbers are reported, and the transaction figure is explicitly an
  /// estimate — the node is never asked how many transactions the chain has.
  pub(crate) fn sync_progress(&self) -> SyncProgress {
    let local = self.local_tip.load(Ordering::Relaxed);
    let network = self.network_tip.load(Ordering::Relaxed);
    let block_pct = (network > 0 && local >= 0)
      .then(|| (local as f64 / network as f64 * 100.0).clamp(0.0, 100.0));

    let indexed = self
      .transactions_baseline_known
      .load(Ordering::Relaxed)
      .then(|| {
        self.transactions_baseline.load(Ordering::Relaxed)
          + self.transactions_ingested_total.load(Ordering::Relaxed)
      });

    // The denominator is the node's own count, never an extrapolation from
    // the density observed locally: the blocks already indexed are the sparse
    // ones, so projecting from them reads *higher* than the block percentage
    // and is wrong in the opposite direction. Without the node's count there
    // is no transaction percentage, only the count and the density.
    let network_total = self.network_transactions_total.load(Ordering::Relaxed);
    let estimated_total = (network_total > 0).then_some(network_total);
    let tx_pct = match (indexed, estimated_total) {
      (Some(indexed), Some(total)) if total > 0 => {
        Some((indexed as f64 / total as f64 * 100.0).clamp(0.0, 100.0))
      }
      _ => None,
    };

    SyncProgress {
      block_pct,
      transactions_indexed: indexed,
      transactions_estimated_total: estimated_total,
      tx_pct,
      txs_per_block_recent: self.txs_per_block_recent(),
    }
  }

  /// `true` while a non-transient failure has ingestion parked.
  pub(crate) fn operator_pause_requested(&self) -> bool {
    self.operator_pause_requested.load(Ordering::Relaxed)
  }

  /// Shared handle to the pause flag, for the IPC command handler.
  pub(crate) fn pause_handle(&self) -> Arc<AtomicBool> {
    Arc::clone(&self.operator_pause_requested)
  }

  /// Park ingestion until an operator intervenes.
  fn request_operator_pause(&self) {
    self.operator_pause_requested.store(true, Ordering::Relaxed);
    self.set_state(STATE_PAUSED);
  }

  /// Drop the per-cycle runtime view. A failed cycle released every lease and
  /// dropped its look-ahead queue, so anything describing work in flight is
  /// now a stale reading rather than a small inaccuracy.
  fn clear_runtime(&self) {
    self.backoff_until_epoch.store(0, Ordering::Relaxed);
    self.retry_attempt.store(0, Ordering::Relaxed);
  }

  /// Record that the loop is sleeping off its `attempt`-th transient failure.
  fn note_backoff(&self, attempt: u32, delay: Duration) {
    self.retry_attempt.store(attempt, Ordering::Relaxed);
    self.backoff_until_epoch.store(
      unix_now().saturating_add(delay.as_secs()),
      Ordering::Relaxed,
    );
  }

  /// Consecutive transient failures, when the loop is retrying.
  pub(crate) fn retry_attempt(&self) -> Option<u32> {
    let attempt = self.retry_attempt.load(Ordering::Relaxed);
    (attempt > 0).then_some(attempt)
  }

  /// Epoch the current backoff sleep ends, when one is running.
  pub(crate) fn backoff_until_epoch(&self) -> Option<u64> {
    let epoch = self.backoff_until_epoch.load(Ordering::Relaxed);
    (epoch > 0).then_some(epoch)
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
      STATE_PAUSED => IngestionStateName::Paused,
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

/// Sync progress measured two ways (task F2).
///
/// The block figure is exact and misleading; the transaction figure is an
/// estimate and useful. Neither is offered alone.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub(crate) struct SyncProgress {
  /// Percentage of blocks connected, of the node's tip.
  pub(crate) block_pct: Option<f64>,
  /// Transactions in the database (startup count plus this run's absorbs).
  pub(crate) transactions_indexed: Option<u64>,
  /// Transactions the node reports for the whole chain, when it was asked.
  pub(crate) transactions_estimated_total: Option<u64>,
  /// Percentage of the chain's transactions that are indexed.
  pub(crate) tx_pct: Option<f64>,
  /// Smoothed transactions per block over the blocks absorbed this run.
  pub(crate) txs_per_block_recent: Option<f64>,
}

/// Owns the background ingest loop.
pub(crate) struct IngestVisor {
  db: HyphenDb,
  ingest: Arc<HyDbIngest>,
  source: Arc<dyn RestSource>,
  /// Optional JSON-RPC handle, used only to ask the node how many
  /// transactions the chain has (task F2). Ingest itself needs REST alone, so
  /// this stays optional and its absence costs the transaction percentage and
  /// nothing else.
  rpc: Option<Arc<dyn RpcSource>>,
  lifecycle: Arc<Lifecycle>,
  status: Arc<IngestStatus>,
  poll_secs: u64,
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
    rpc: Option<Arc<dyn RpcSource>>,
    lifecycle: Arc<Lifecycle>,
    poll_secs: u64,
  ) -> Self {
    Self {
      db,
      ingest: Arc::new(ingest),
      source,
      rpc,
      lifecycle,
      status: Arc::new(IngestStatus::new()),
      poll_secs: poll_secs.max(1),
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
    let rpc = self.rpc.clone();
    let lifecycle = Arc::clone(&self.lifecycle);
    let status = Arc::clone(&self.status);
    let poll_secs = self.poll_secs;
    let poll = Duration::from_secs(poll_secs);
    self.task = Some(tokio::spawn(async move {
      info!("ingest loop started");
      // One count, once, so progress can be reported by transaction as well as
      // by block (task F2). Everything after this is incremented in memory.
      count_transactions_baseline(&db, &status).await;
      // Refreshed on a slow cadence: the chain's transaction count moves by a
      // few hundred a minute and the answer is only a denominator.
      let mut chain_stats_refreshed_at = 0u64;
      // Consecutive transient failures. Reset by any successful cycle, so a
      // blip that resolves does not carry its ladder into the next hour.
      let mut attempt: u32 = 0;
      while lifecycle.running() {
        if status.operator_pause_requested() {
          // Parked by a non-transient failure. Nothing here can clear it: an
          // operator command (rewind, disconnect-tip, force-reindex) does,
          // because those are the acts that change the situation.
          status.set_state(STATE_PAUSED);
          status.update_stall();
          lifecycle.sleep(poll).await;
          continue;
        }
        if let Some(rpc) = rpc.as_ref() {
          let now = unix_now();
          if now.saturating_sub(chain_stats_refreshed_at) >= CHAIN_TX_STATS_REFRESH_SECS {
            chain_stats_refreshed_at = now;
            refresh_chain_tx_stats(rpc.as_ref(), &status).await;
          }
        }
        match ingest_once(&db, &ingest, source.as_ref(), &status, &lifecycle).await {
          Ok(()) => {
            attempt = 0;
            status.clear_runtime();
            status.set_state(STATE_IDLE);
          }
          // The severity ladder is the storage crate's, delegated through
          // `DaemonError::classify()`. The loop's only job is to act on it —
          // a Fatal error must not be retried at poll cadence forever, and a
          // permanent one must be visibly distinct from a busy database.
          Err(error) => match failure_action(&error) {
            FailureAction::Stop => {
              error!(%error, outcome = "fatal", "ingest cycle failed unrecoverably; shutting down");
              status.clear_runtime();
              status.request_operator_pause();
              lifecycle.interrupt();
              break;
            }
            FailureAction::Pause => {
              error!(
                %error,
                outcome = "error",
                "ingest cycle failed permanently; pausing ingestion until an operator intervenes"
              );
              status.clear_runtime();
              status.request_operator_pause();
              status.update_stall();
              continue;
            }
            FailureAction::Backoff => {
              attempt = attempt.saturating_add(1);
              let delay = backoff_delay(poll_secs, attempt);
              warn!(
                %error,
                outcome = "warning",
                attempt,
                backoff_secs = delay.as_secs(),
                "ingest cycle failed transiently; backing off"
              );
              status.note_backoff(attempt, delay);
              status.set_state(STATE_BACKOFF);
              status.update_stall();
              lifecycle.sleep(delay).await;
              continue;
            }
          },
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

/// Ask the node for its chain-wide transaction count (task F2).
///
/// Not an ingest failure when it fails: it costs the transaction percentage,
/// so it is logged at debug and dropped. A node that has RPC disabled never
/// gets here at all.
async fn refresh_chain_tx_stats(rpc: &dyn RpcSource, status: &IngestStatus) {
  match rpc.get_chain_tx_stats().await {
    Ok(stats) => status.set_network_transactions_total(stats.tx_count),
    Err(error) => tracing::debug!(
      %error,
      "getchaintxstats failed; transaction-side progress stays a count without a percentage"
    ),
  }
}

/// Count the transactions already in the database, once, at ingest start.
///
/// A failure here is not an ingest failure: it costs the transaction-side
/// progress report and nothing else, so it is logged and dropped rather than
/// classified.
async fn count_transactions_baseline(db: &HyphenDb, status: &IngestStatus) {
  let started = std::time::Instant::now();
  match db.txs().total().await {
    Ok(total) => {
      status.set_transactions_baseline(total);
      info!(
        transactions = total,
        elapsed_ms = started.elapsed().as_millis(),
        "counted the indexed transactions for transaction-side progress"
      );
    }
    Err(error) => warn!(
      %error,
      "could not count indexed transactions; progress will be reported by block only"
    ),
  }
}

/// Advance the storage engine toward the node's current tip once,
/// interleaving bounded absorb chunks with engine connects.
async fn ingest_once(
  db: &HyphenDb,
  ingest: &HyDbIngest,
  source: &dyn RestSource,
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
    absorb_with_ancestors(ingest, source, raw).await?;
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
    let mut chunk_tip: Option<HyDbHash> = None;
    for h in height..=chunk_end {
      if !lifecycle.running() {
        return Ok(());
      }
      let hash = source.block_hash(h).await?;
      let raw = source.raw_block(&hash).await?;
      if h == chunk_end {
        chunk_tip = Some(block_hash(&raw)?);
      }
      // Transaction-side progress (task F2): counted from the block's own
      // CompactSize as it is absorbed, so the figure tracks the linear
      // catch-up without a second pass over what was just written.
      if let Some(txs) = block_tx_count(&raw) {
        status.note_block_txs(txs);
      }
      absorb_with_ancestors(ingest, source, raw).await?;
    }
    if let Some(tip) = chunk_tip {
      status.set_state(STATE_COMMITTING);
      advance_until_settled(db, ingest, status, tip).await?;
    }
    height = chunk_end.saturating_add(1);
  }
  Ok(())
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
  use crate::error::RestErrorKind;
  use hyphen_db::HyDbError;

  fn pg(code: &str) -> DaemonError {
    DaemonError::storage(
      "ingest",
      HyDbError::Pg {
        message: "irrelevant".to_owned(),
        code: Some(code.to_owned()),
      },
    )
  }

  /// E2.19: the loop must act on the classification, and the three actions
  /// must stay distinguishable. A Fatal error looping at poll cadence and a
  /// deadlock parking the indexer are both regressions this pins.
  #[test]
  fn failure_action_follows_the_classification() {
    // Fatal — nothing about retrying this can help.
    assert_eq!(
      failure_action(&DaemonError::Config("bad".to_owned())),
      FailureAction::Stop
    );
    assert_eq!(
      failure_action(&DaemonError::InsufficientDisk {
        available: 0,
        required: 1,
      }),
      FailureAction::Stop
    );

    // Transient Postgres classes, graded by the storage crate: retry.
    for code in ["08006", "40001", "40P01", "53300", "57P01", "55006"] {
      assert_eq!(
        failure_action(&pg(code)),
        FailureAction::Backoff,
        "sqlstate {code} should back off"
      );
    }

    // Permanent Postgres failures: park for a human, do not spin.
    for code in ["23505", "42P01"] {
      assert_eq!(
        failure_action(&pg(code)),
        FailureAction::Pause,
        "sqlstate {code} should pause"
      );
    }

    // A node that is down or slow is a blip, not an intervention.
    assert_eq!(
      failure_action(&DaemonError::RestSource {
        message: "connect failed".to_owned(),
        kind: RestErrorKind::Connect,
      }),
      FailureAction::Backoff
    );
    assert_eq!(
      failure_action(&DaemonError::RestSource {
        message: "not found".to_owned(),
        kind: RestErrorKind::Other,
      }),
      FailureAction::Pause
    );
  }

  /// The backoff must actually differ from the healthy poll interval — the
  /// deleted-code symptom was a `STATE_BACKOFF` label over an unchanged sleep.
  #[test]
  fn backoff_ladder_doubles_and_caps() {
    assert_eq!(backoff_delay(5, 1), Duration::from_secs(5));
    assert_eq!(backoff_delay(5, 2), Duration::from_secs(10));
    assert_eq!(backoff_delay(5, 3), Duration::from_secs(20));
    assert_eq!(backoff_delay(5, 4), Duration::from_secs(40));
    // Capped, and never zero however the arithmetic saturates.
    assert_eq!(backoff_delay(5, 99), Duration::from_secs(MAX_BACKOFF_SECS));
    assert_eq!(backoff_delay(0, 1), Duration::from_secs(1));
    assert!(backoff_delay(5, 2) > backoff_delay(5, 1));
  }

  /// F2: the transaction count comes from the CompactSize after the header,
  /// in all four of its encodings.
  #[test]
  fn block_tx_count_reads_the_compact_size() {
    let mut raw = vec![0u8; 80];
    raw.push(3);
    assert_eq!(block_tx_count(&raw), Some(3));

    let mut raw = vec![0u8; 80];
    raw.extend_from_slice(&[0xFD, 0x10, 0x27]); // 10_000
    assert_eq!(block_tx_count(&raw), Some(10_000));

    let mut raw = vec![0u8; 80];
    raw.extend_from_slice(&[0xFE, 0x40, 0x42, 0x0F, 0x00]); // 1_000_000
    assert_eq!(block_tx_count(&raw), Some(1_000_000));

    let mut raw = vec![0u8; 80];
    raw.push(0xFF);
    raw.extend_from_slice(&7u64.to_le_bytes());
    assert_eq!(block_tx_count(&raw), Some(7));

    // Too short to hold a count: no guess.
    assert_eq!(block_tx_count(&[0u8; 80]), None);
    assert_eq!(block_tx_count(&[]), None);
  }

  /// F2: a run must report progress by transaction as well as by block, and
  /// the two must disagree while density is still rising ahead of the indexer.
  /// The reference run read 36% by block at ~6% by transaction; the numbers
  /// below are that run's, with the node's own count as the denominator.
  #[test]
  fn progress_is_reported_by_transaction_as_well_as_block() {
    let status = IngestStatus::new();
    status.local_tip.store(907_000, Ordering::Relaxed);
    status.network_tip.store(2_520_000, Ordering::Relaxed);
    status.set_transactions_baseline(4_400_000);
    status.set_network_transactions_total(70_556_609);
    for _ in 0..2_000 {
      status.note_block_txs(3);
    }

    let progress = status.sync_progress();
    let block_pct = progress.block_pct.unwrap_or_default();
    let tx_pct = progress.tx_pct.unwrap_or_default();
    assert!(
      (block_pct - 36.0).abs() < 0.5,
      "block progress should read ~36%, got {block_pct}"
    );
    assert!(
      (tx_pct - 6.2).abs() < 0.5,
      "transaction progress should read ~6%, got {tx_pct}"
    );
    assert!(
      tx_pct < block_pct / 4.0,
      "the two figures must disagree sharply: {tx_pct} vs {block_pct}"
    );
    assert_eq!(progress.transactions_estimated_total, Some(70_556_609));
    assert!(progress.txs_per_block_recent.unwrap_or_default() > 2.0);
  }

  /// Without the node's count there is no percentage — and specifically no
  /// percentage extrapolated from local density, which reads *higher* than the
  /// block figure because the blocks indexed so far are the sparse ones.
  #[test]
  fn no_node_count_means_no_transaction_percentage() {
    let status = IngestStatus::new();
    status.local_tip.store(907_000, Ordering::Relaxed);
    status.network_tip.store(2_520_000, Ordering::Relaxed);
    status.set_transactions_baseline(4_400_000);
    status.note_block_txs(3);

    let progress = status.sync_progress();
    assert_eq!(progress.tx_pct, None);
    assert_eq!(progress.transactions_estimated_total, None);
    // The honest parts still report.
    assert_eq!(progress.transactions_indexed, Some(4_400_003));
    assert!(progress.block_pct.is_some());
    assert!(progress.txs_per_block_recent.is_some());
  }

  /// Without a baseline count there is no transaction figure, and the block
  /// figure must still be reported rather than the whole thing going dark.
  #[test]
  fn progress_degrades_to_blocks_when_the_baseline_is_unknown() {
    let status = IngestStatus::new();
    status.local_tip.store(500, Ordering::Relaxed);
    status.network_tip.store(1_000, Ordering::Relaxed);
    let progress = status.sync_progress();
    assert_eq!(progress.block_pct, Some(50.0));
    assert_eq!(progress.transactions_indexed, None);
    assert_eq!(progress.tx_pct, None);
  }

  /// A pause is visible on the wire and clearable through the shared handle
  /// the IPC command path holds — otherwise the only cure is a restart.
  #[test]
  fn operator_pause_is_visible_and_clearable() {
    let status = IngestStatus::new();
    assert!(!status.operator_pause_requested());

    status.note_backoff(3, Duration::from_secs(40));
    assert_eq!(status.retry_attempt(), Some(3));
    assert!(status.backoff_until_epoch().is_some());

    status.request_operator_pause();
    assert!(status.operator_pause_requested());
    assert_eq!(status.state_name(), IngestionStateName::Paused);
    assert_eq!(status.activity(), IngestionActivity::Sleeping);

    // The IPC command handler clears it through this handle.
    let handle = status.pause_handle();
    assert!(handle.swap(false, Ordering::Relaxed));
    assert!(!status.operator_pause_requested());

    status.clear_runtime();
    assert_eq!(status.retry_attempt(), None);
    assert_eq!(status.backoff_until_epoch(), None);
  }
}
