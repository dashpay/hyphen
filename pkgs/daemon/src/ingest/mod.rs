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
use hyphen_db::{
  HyDbBlockSeen, HyDbHash, HyDbHeight, HyDbIngest, HyDbSyncMode, HyDbTipTarget, HyphenDb,
};
use tracing::{error, info, warn};

use crate::error::{DaemonError, Outcome, RestErrorKind};
use crate::lifecycle::Lifecycle;
use crate::sources::{RestSource, RpcSource};
use crate::utils::{self, HashEndian};
use hyphend::wire::msg::{IngestionActivity, IngestionStateName};

/// Default seconds between tip polls.
pub(crate) const DEFAULT_POLL_SECS: u64 = 5;

/// Blocks absorbed per chunk before the engine is asked to connect them.
const ABSORB_CHUNK_BLOCKS: u32 = 512;

/// How long after a reorg the recovery indicator keeps reporting it.
///
/// A reorg is an event, not a state, and the engine exposes no "recovering"
/// flag to poll. Holding the last one for a window is what makes it visible to
/// an operator who was not watching the log at the moment it happened.
const RECOVERY_STICKY_SECS: u64 = 300;

/// Seconds between refreshes of the node's chain-wide transaction count.
const CHAIN_TX_STATS_REFRESH_SECS: u64 = 300;

/// Ingest activity considered stalled after this many seconds without
/// progress while behind the node tip.
const STALL_AFTER_SECS: u64 = 120;

/// How far behind the node the projection may be before the live subsystems
/// that read it are considered unable to answer. See
/// [`IngestStatus::projection_near_tip`].
const LIVE_TIP_MARGIN_BLOCKS: u32 = 4;

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
  /// Index-maintenance regime the storage engine reports, shared
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
  /// Epoch of the last reorg this process applied (0 = none).
  last_reorg_epoch: AtomicU64,
  /// Fork height of that reorg (-1 = none).
  last_reorg_fork_height: AtomicI64,
  /// Whether that reorg fell back to full re-derivation rather than bounded
  /// undo replay. The distinction is the whole point of the indicator: one is
  /// routine, the other means the undo coverage was not enough.
  last_reorg_full_rederive: AtomicBool,
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
  /// Epoch the current park began (0 = not parked). Set once, the first time
  /// [`Self::request_operator_pause`] fires, and cleared the next time the
  /// loop observes the pause lifted — never touched while the pause is still
  /// in effect, so it measures the whole park, not just its latest cycle.
  /// This is what the bounded-park timeout in the ingest loop measures
  /// against, and what an operator's IPC command implicitly resets by
  /// clearing the pause flag itself.
  paused_since_epoch: AtomicU64,
  /// Epoch the "still paused" heartbeat was last logged (0 = never). Distinct
  /// from `paused_since_epoch`, which never moves for the life of one park:
  /// this is what makes the periodic log line periodic instead of once.
  last_pause_log_epoch: AtomicU64,
  /// Epoch of the ingest loop's last iteration, updated once per pass through
  /// its `while` loop regardless of outcome (healthy, backing off, or
  /// parked). Shared with the REST health endpoint as an `Arc<AtomicU64>` so
  /// a liveness probe can tell "the loop is asleep on a legitimate backoff or
  /// poll interval" from "the loop's task is wedged and has stopped
  /// iterating at all" without ever touching a connection pool itself — the
  /// health endpoint's entire reason for existing.
  loop_heartbeat_epoch: Arc<AtomicU64>,
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

/// How long a failure-induced park may sit waiting for an operator before the
/// ingest loop gives up and takes the whole daemon down with it.
///
/// The rule this exists to enforce: Hyphen must either wait around and
/// actually resume, or exit — never sit wedged, because a wedge defeats
/// liveness checking. A park that never ends is worse than either genuine
/// outcome, so it is bounded: past this, the daemon exits non-zero (see
/// [`crate::lifecycle::Lifecycle::interrupt_fatal`]) so a supervisor (Docker,
/// systemd) restarts it, which is strictly better than sitting parked for 26
/// hours the way the mainnet incident did.
const PARK_TIMEOUT_SECS: u64 = 30 * 60;

/// How often the ingest loop logs a heartbeat while parked.
///
/// Before this existed, a park produced exactly one log line and then total
/// silence for as long as the park lasted — 24 hours of it, in the mainnet
/// incident — which is indistinguishable from a deadlock. A periodic "still
/// parked" line makes silence itself diagnostic: if this stops appearing, the
/// loop's task has actually wedged, which is a different and worse problem
/// than a park (and is what [`IngestStatus::loop_heartbeat_epoch`] and the
/// REST health endpoint are for).
const PAUSE_HEARTBEAT_LOG_SECS: u64 = 5 * 60;

/// How long the ingest loop's heartbeat may go unrefreshed before the REST
/// health endpoint calls it wedged rather than merely asleep.
///
/// Set comfortably above [`MAX_BACKOFF_SECS`] (the longest legitimate gap
/// between heartbeat touches from a single backoff sleep) so a healthy loop
/// riding out its worst-case backoff never trips it, while still catching a
/// loop whose task has stopped iterating altogether — the exact shape of the
/// wedge this whole task is about.
const HEALTH_HEARTBEAT_STALE_SECS: u64 = 2 * MAX_BACKOFF_SECS;

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

/// `true` once `heartbeat_epoch` has gone unrefreshed for longer than
/// [`HEALTH_HEARTBEAT_STALE_SECS`] as of `now`. Split out from
/// [`IngestStatus::loop_heartbeat_stale`] so the health-endpoint decision is
/// testable without a running loop behind it.
fn loop_heartbeat_is_stale(heartbeat_epoch: u64, now: u64) -> bool {
  now.saturating_sub(heartbeat_epoch) > HEALTH_HEARTBEAT_STALE_SECS
}

/// `true` once a park that began at `paused_since_epoch` has run past
/// [`PARK_TIMEOUT_SECS`] as of `now`, meaning the ingest loop must stop
/// waiting for an operator and take the daemon down so a supervisor restarts
/// it. Split out from the loop body for the same reason as
/// [`loop_heartbeat_is_stale`]: testable without a storage engine, a node, or
/// a running loop.
fn park_should_exit(paused_since_epoch: u64, now: u64) -> bool {
  paused_since_epoch > 0 && now.saturating_sub(paused_since_epoch) >= PARK_TIMEOUT_SECS
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
  pub(crate) fn new() -> Self {
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
      last_reorg_epoch: AtomicU64::new(0),
      last_reorg_fork_height: AtomicI64::new(-1),
      last_reorg_full_rederive: AtomicBool::new(false),
      network_transactions_total: AtomicU64::new(0),
      transactions_baseline: AtomicU64::new(0),
      transactions_baseline_known: AtomicBool::new(false),
      txs_per_block_milli: AtomicU64::new(0),
      operator_pause_requested: Arc::new(AtomicBool::new(false)),
      retry_attempt: AtomicU32::new(0),
      backoff_until_epoch: AtomicU64::new(0),
      paused_since_epoch: AtomicU64::new(0),
      last_pause_log_epoch: AtomicU64::new(0),
      // Starts fresh rather than at 0/unset: a brand-new loop has not failed
      // to iterate, it just has not iterated yet, and a health probe hit in
      // the gap between process start and the loop's first pass must not
      // read that as a wedge.
      loop_heartbeat_epoch: Arc::new(AtomicU64::new(unix_now())),
    }
  }

  fn set_state(&self, state: u8) {
    self.state.store(state, Ordering::Relaxed);
  }

  /// Record one absorbed block's transaction count.
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

  /// Record a reorg the engine reported.
  fn note_reorg(&self, fork_height: u32, full_rederive: bool) {
    self.last_reorg_epoch.store(unix_now(), Ordering::Relaxed);
    self
      .last_reorg_fork_height
      .store(i64::from(fork_height), Ordering::Relaxed);
    self
      .last_reorg_full_rederive
      .store(full_rederive, Ordering::Relaxed);
  }

  /// Recovery state for the IPC snapshot: what happened, and where.
  ///
  /// `("clean", None)` when this process has applied no reorg, or the last one
  /// has aged out of [`RECOVERY_STICKY_SECS`]. The console's recovery
  /// indicator reads exactly this pair, and before E2.5 nothing wrote it — it
  /// was hardcoded `"clean"`, so the panel was structurally unable to fire.
  pub(crate) fn recovery_state(&self) -> (&'static str, Option<u32>) {
    let epoch = self.last_reorg_epoch.load(Ordering::Relaxed);
    if epoch == 0 || unix_now().saturating_sub(epoch) > RECOVERY_STICKY_SECS {
      return ("clean", None);
    }
    let label = if self.last_reorg_full_rederive.load(Ordering::Relaxed) {
      "rederive"
    } else {
      "undo-replay"
    };
    let height = u32::try_from(self.last_reorg_fork_height.load(Ordering::Relaxed)).ok();
    (label, height)
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

  /// Sync progress by block *and* by transaction.
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
  pub(crate) fn request_operator_pause(&self) {
    self.operator_pause_requested.store(true, Ordering::Relaxed);
    self.set_state(STATE_PAUSED);
    // Only stamped the first time: a park that fails its `Pause` classifier
    // again on a later cycle (rare, but the flag stays set until an operator
    // command lifts it) must not have its clock reset, or the bounded-park
    // timeout below could never fire.
    self
      .paused_since_epoch
      .compare_exchange(0, unix_now(), Ordering::Relaxed, Ordering::Relaxed)
      .ok();
  }

  /// Seconds the current park has lasted, when one is in effect.
  pub(crate) fn pause_elapsed_secs(&self) -> Option<u64> {
    let since = self.paused_since_epoch.load(Ordering::Relaxed);
    (since > 0).then(|| unix_now().saturating_sub(since))
  }

  /// Reset the park clock once the pause is observed lifted. A no-op while
  /// still paused; called on every iteration where the loop finds itself not
  /// parked, which is the only signal it has that an operator's command
  /// cleared the flag since the last check.
  fn clear_pause_started(&self) {
    self.paused_since_epoch.store(0, Ordering::Relaxed);
    self.last_pause_log_epoch.store(0, Ordering::Relaxed);
  }

  /// Log the "still parked" heartbeat if [`PAUSE_HEARTBEAT_LOG_SECS`] have
  /// passed since the last one. Returns the park's age for the caller's own
  /// bounded-timeout check, so both read `unix_now()` against the same
  /// `paused_since_epoch` rather than risking two slightly different clocks.
  fn log_pause_heartbeat_if_due(&self, attempt_context: &str) -> u64 {
    let elapsed = self.pause_elapsed_secs().unwrap_or(0);
    let now = unix_now();
    let last = self.last_pause_log_epoch.load(Ordering::Relaxed);
    if now.saturating_sub(last) >= PAUSE_HEARTBEAT_LOG_SECS {
      self.last_pause_log_epoch.store(now, Ordering::Relaxed);
      warn!(
        elapsed_secs = elapsed,
        park_timeout_secs = PARK_TIMEOUT_SECS,
        "{attempt_context}: ingestion is still parked awaiting operator intervention"
      );
    }
    elapsed
  }

  /// Record that the ingest loop is still iterating, whatever the outcome.
  fn note_loop_heartbeat(&self) {
    self
      .loop_heartbeat_epoch
      .store(unix_now(), Ordering::Relaxed);
  }

  /// Shared handle to the loop heartbeat, for the REST health endpoint.
  pub(crate) fn loop_heartbeat_handle(&self) -> Arc<AtomicU64> {
    Arc::clone(&self.loop_heartbeat_epoch)
  }

  /// `true` once the loop heartbeat has gone unrefreshed longer than
  /// [`HEALTH_HEARTBEAT_STALE_SECS`] — the loop's task has stopped iterating,
  /// as opposed to merely sleeping through a legitimate backoff or poll
  /// interval.
  pub(crate) fn loop_heartbeat_stale(&self) -> bool {
    loop_heartbeat_is_stale(
      self.loop_heartbeat_epoch.load(Ordering::Relaxed),
      unix_now(),
    )
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

  /// Force the stall flag for a test in another module (e.g. the REST health
  /// endpoint's), which needs a stalled `IngestStatus` but has no access to
  /// the private `local_tip`/`network_tip`/progress bookkeeping that
  /// [`Self::update_stall`] derives it from.
  #[cfg(test)]
  pub(crate) fn force_stalled_for_test(&self) {
    self.stalled.store(true, Ordering::Relaxed);
  }

  pub(crate) fn network_tip(&self) -> Option<u32> {
    u32::try_from(self.network_tip.load(Ordering::Relaxed)).ok()
  }

  pub(crate) fn local_tip(&self) -> Option<u32> {
    u32::try_from(self.local_tip.load(Ordering::Relaxed)).ok()
  }

  /// Set both tips, for tests of the live subsystems that wait on them.
  #[cfg(test)]
  pub(crate) fn set_tips_for_test(&self, local: u32, node: u32) {
    self.local_tip.store(i64::from(local), Ordering::Relaxed);
    self.network_tip.store(i64::from(node), Ordering::Relaxed);
  }

  /// Whether the projection is close enough to the node for the live
  /// subsystems that read it to get sensible answers.
  ///
  /// A mempool transaction spends outputs that already exist, very often ones
  /// created within the last few blocks. Resolving it needs those outputs in
  /// the projection, so while the projection is behind, hydration cannot
  /// succeed — it is not merely stale, it is *unanswerable*. During the
  /// mainnet sync this was not free: every refresh issued txid lookups that
  /// seq-scanned the open partition, whose txid index stepped sync had
  /// deliberately dropped, and then failed anyway. The work competed for the
  /// same disk as the ingest that would eventually have made it possible.
  ///
  /// The margin is small on purpose. Being a block or two behind is ordinary
  /// while following the tip and should not switch the mempool off, but tens
  /// of blocks behind means recent outputs are missing and hydration starts
  /// failing again.
  pub(crate) fn projection_near_tip(&self) -> bool {
    match (self.local_tip(), self.network_tip()) {
      (Some(local), Some(node)) => node.saturating_sub(local) <= LIVE_TIP_MARGIN_BLOCKS,
      // Nothing ingested yet, or the node's height is not known: not near.
      _ => false,
    }
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

/// Sync progress measured two ways.
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
  /// transactions the chain has. Ingest itself needs REST alone, so
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

  /// Watch channel of the deferred index build in flight. Read alongside
  /// `progress`, which it explains: a build holds the write path, so progress
  /// stops moving while one runs.
  pub(crate) fn index_build(
    &self,
  ) -> tokio::sync::watch::Receiver<Option<hyphen_db::HyDbIndexBuild>> {
    self.ingest.index_build_watch()
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
      // by block. Everything after this is incremented in memory.
      count_transactions_baseline(&db, &status).await;
      // Refreshed on a slow cadence: the chain's transaction count moves by a
      // few hundred a minute and the answer is only a denominator.
      let mut chain_stats_refreshed_at = 0u64;
      // Consecutive transient failures. Reset by any successful cycle, so a
      // blip that resolves does not carry its ladder into the next hour.
      let mut attempt: u32 = 0;
      while lifecycle.running() {
        // Proves the loop's task is still iterating at all, independent of
        // whether this pass finds work to do. Read by the REST health
        // endpoint, which otherwise has no way to tell "asleep on a
        // legitimate backoff" from "wedged mid-cycle and never coming back" —
        // and deliberately cannot ask a connection pool, which is the thing
        // that was wedged in the incident this exists to catch.
        status.note_loop_heartbeat();
        if status.operator_pause_requested() {
          // Parked by a non-transient failure. Nothing here can clear it: an
          // operator command (rewind, disconnect-tip, force-reindex) does,
          // because those are the acts that change the situation.
          status.set_state(STATE_PAUSED);
          status.update_stall();
          let elapsed = status.log_pause_heartbeat_if_due("ingest loop");
          if elapsed >= PARK_TIMEOUT_SECS {
            // The rule this loop exists to enforce: either wait around and
            // actually resume, or exit — never sit wedged. A human has had
            // `PARK_TIMEOUT_SECS` to intervene; past that, staying parked
            // forever is worse than exiting, because it defeats liveness
            // checking. Exit non-zero so a supervisor restarts the daemon,
            // which is what should have happened within minutes of the
            // mainnet incident instead of after 26 hours and a manual kill.
            error!(
              elapsed_secs = elapsed,
              park_timeout_secs = PARK_TIMEOUT_SECS,
              "ingestion has been parked past its bounded timeout with no \
               operator intervention; exiting so a supervisor restarts the daemon"
            );
            lifecycle.interrupt_fatal();
            break;
          }
          lifecycle.sleep(poll).await;
          continue;
        }
        // Reached only when not parked: either healthy, or an operator
        // command just lifted a pause. Either way the park clock (and its
        // heartbeat cadence) must not carry into whatever park comes next.
        status.clear_pause_started();
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
              // Fatal, not a park: no amount of waiting fixes this, so the
              // daemon exits (non-zero) immediately rather than sitting
              // parked. See `interrupt_fatal`'s doc comment for why plain
              // `interrupt()` alone left the process alive-but-unresponsive.
              lifecycle.interrupt_fatal();
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

/// Ask the node for its chain-wide transaction count.
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
    absorb_with_ancestors(ingest, source, status, raw).await?;
    status.set_state(STATE_COMMITTING);
    advance_until_settled(db, ingest, status, tip, node_height).await?;
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
      absorb_with_ancestors(ingest, source, status, raw).await?;
    }
    if let Some(tip) = chunk_tip {
      status.set_state(STATE_COMMITTING);
      advance_until_settled(db, ingest, status, tip, node_height).await?;
    }
    height = chunk_end.saturating_add(1);
  }
  Ok(())
}

/// Drive `on_tip` until the projection reaches the engine's reported target
/// (each call connects a bounded segment).
///
/// `network_height` is carried separately from `tip` because the two mean
/// different things during a cold sync: `tip` ends the chunk being connected
/// now, while `network_height` is how far the node has got. Stepped sync
/// needs the second one.
async fn advance_until_settled(
  db: &HyphenDb,
  ingest: &HyDbIngest,
  status: &IngestStatus,
  tip: HyDbHash,
  network_height: u32,
) -> Result<(), DaemonError> {
  loop {
    let report = ingest
      .on_tip(HyDbTipTarget::Chunk {
        tip,
        network_height: HyDbHeight::new(network_height),
      })
      .await
      .map_err(|e| DaemonError::storage("advance to tip", e))?;
    if let Some(reorg) = &report.reorg {
      info!(
        fork_height = reorg.fork_height.get(),
        disconnected = reorg.disconnected.len(),
        full_rederive = reorg.full_rederive,
        "chain reorganization applied"
      );
      status.note_reorg(reorg.fork_height.get(), reorg.full_rederive);
    }
    let connected = blocks_connected(&report.from, &report.to);
    if connected > 0 {
      status
        .blocks_ingested_total
        .fetch_add(connected, Ordering::Relaxed);
      status.mark_progress();
    }
    if let Some(to) = report.to {
      status
        .local_tip
        .store(i64::from(to.get()), Ordering::Relaxed);
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
    (Some(from), Some(to)) if to.get() > from.get() => u64::from(to.get() - from.get()),
    (None, Some(to)) => u64::from(to.get()) + 1,
    _ => 0,
  }
}

async fn read_local_tip(db: &HyphenDb) -> Result<i64, DaemonError> {
  Ok(
    db.status()
      .tip()
      .await
      .map_err(|e| DaemonError::storage("read canonical tip", e))?
      .map_or(-1i64, |tip| i64::from(tip.height.get())),
  )
}

/// Record an ancestor about to be fetched, refusing a walk that cannot end.
///
/// Split out from the loop so both refusals are reachable without a storage
/// engine or a node behind them.
///
/// # Errors
///
/// `RestSource` when the walk has already reached `max_ancestors`, or when this
/// ancestor has been requested before, which means the reported chain loops.
fn admit_ancestor_request(
  parent: &HyDbHash,
  requested: &mut std::collections::HashSet<[u8; 32]>,
  max_ancestors: usize,
) -> Result<(), DaemonError> {
  let parent_hex = utils::view_hex(parent.as_bytes(), HashEndian::Ser);
  if requested.len() >= max_ancestors {
    return Err(DaemonError::RestSource {
      message: format!(
        "ancestor walk reached {max_ancestors} blocks without finding an \
         absorbed parent, which is the deepest reorg this projection can \
         reconnect; stopped at {parent_hex}"
      ),
      kind: RestErrorKind::Other,
    });
  }
  if !requested.insert(*parent.as_bytes()) {
    return Err(DaemonError::RestSource {
      message: format!(
        "ancestor {parent_hex} was requested twice, so the chain the source \
         reports contains a cycle"
      ),
      kind: RestErrorKind::Other,
    });
  }
  Ok(())
}

/// Refuse a fetched ancestor that is not the block that was asked for.
///
/// # Errors
///
/// `RestSource` when the hashes differ. A source that echoes the child back
/// would otherwise have it absorbed under a hash it does not have, and the walk
/// would never reach the parent it is looking for.
fn verify_ancestor_response(wanted: &HyDbHash, got: &HyDbHash) -> Result<(), DaemonError> {
  if got == wanted {
    return Ok(());
  }
  Err(DaemonError::RestSource {
    message: format!(
      "asked for block {} and got {}",
      utils::view_hex(wanted.as_bytes(), HashEndian::Ser),
      utils::view_hex(got.as_bytes(), HashEndian::Ser)
    ),
    kind: RestErrorKind::Other,
  })
}

/// Absorb a block, fetching (and absorbing) any ancestor the engine reports
/// missing before retrying — the seam that also pulls in a reorg's branch.
///
/// Each `MissingParent` removes one entry from the stack and adds two, so the
/// walk only terminates because the ancestry does. Three things make that a
/// property of this function rather than of the endpoint's good behaviour: the
/// fetched block must be the one that was asked for, an ancestor may not be
/// requested twice, and the walk may not exceed the depth a reorg can
/// reconnect. Without them a source that answers every parent request with the
/// original block grows the stack until the process is out of memory.
async fn absorb_with_ancestors(
  ingest: &HyDbIngest,
  source: &dyn RestSource,
  status: &IngestStatus,
  raw: Vec<u8>,
) -> Result<(), DaemonError> {
  // Fetching past the retained undo window cannot pay off: a fork deeper than
  // `max_reorg_depth` is refused by the disconnect path anyway.
  let max_ancestors = usize::try_from(hyphen_db::tuning::max_reorg_depth().max(0)).unwrap_or(0);
  let mut requested: std::collections::HashSet<[u8; 32]> = std::collections::HashSet::new();
  // `Bytes` from the start, so handing the block to the engine is a refcount
  // bump rather than a copy of the whole thing. The clone exists only because
  // the missing-parent arm pushes the block back to retry after its ancestor,
  // and that arm is the rare one — copying every block to serve it was paying
  // a full memcpy per block for a path almost none of them take.
  let mut stack = vec![Bytes::from(raw)];
  while let Some(raw) = stack.pop() {
    // Touched per block, not only once per outer loop pass: a large cold-sync
    // chunk can legitimately run longer than the health endpoint's staleness
    // threshold, and without this its own progress would be indistinguishable
    // from the loop having wedged mid-chunk.
    status.note_loop_heartbeat();
    match ingest
      .on_block(raw.clone())
      .await
      .map_err(|e| DaemonError::storage("absorb block", e))?
    {
      HyDbBlockSeen::MissingParent(parent) => {
        admit_ancestor_request(&parent, &mut requested, max_ancestors)?;
        let parent_hex = utils::view_hex(parent.as_bytes(), HashEndian::Ser);
        let parent_raw = source.raw_block(&parent_hex).await?;
        verify_ancestor_response(&parent, &block_hash(&parent_raw)?)?;
        stack.push(raw);
        stack.push(Bytes::from(parent_raw));
      }
      // Counted here rather than on the way in, because a `MissingParent`
      // block is pushed back and seen a second time.
      //
      // `New` only. `Known` means the engine already holds this block, so its
      // transactions were counted when it was first absorbed. Counting them
      // again made every re-delivery — a poll that returns the same tip, a
      // reconnect, a retry — add to a total that has no compensating
      // decrement, so reported progress climbed past 100% and stayed there.
      HyDbBlockSeen::New => {
        if let Some(txs) = block_tx_count(&raw) {
          status.note_block_txs(txs);
        }
      }
      HyDbBlockSeen::Known => {}
    }
  }
  Ok(())
}

fn block_hash(raw: &[u8]) -> Result<HyDbHash, DaemonError> {
  // The inner error is already a `DaemonError`; re-formatting it into a new
  // one would flatten a typed failure into a sentence for no gain.
  crate::consensus::block_hash(raw).map(|h| HyDbHash(*h.as_bytes()))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::error::RestErrorKind;
  use hyphen_db::HyDbError;

  fn hash(byte: u8) -> HyDbHash {
    HyDbHash([byte; 32])
  }

  /// Each `MissingParent` removes one stack entry and adds two, so without a
  /// bound a source that never produces an absorbed parent grows the stack
  /// until the process dies. This is that bound.
  #[test]
  fn the_ancestor_walk_stops_at_the_deepest_reconnectable_reorg() {
    let mut requested = std::collections::HashSet::new();
    for byte in 0..3u8 {
      assert!(admit_ancestor_request(&hash(byte), &mut requested, 3).is_ok());
    }
    let err = admit_ancestor_request(&hash(9), &mut requested, 3)
      .err()
      .map(|e| e.to_string())
      .unwrap_or_default();
    assert!(err.contains("reached 3 blocks"), "got {err}");
  }

  /// A source that reports a parent it has already been asked for is
  /// describing a loop, not an ancestry.
  #[test]
  fn a_repeated_ancestor_request_is_refused_as_a_cycle() {
    let mut requested = std::collections::HashSet::new();
    assert!(admit_ancestor_request(&hash(1), &mut requested, 100).is_ok());
    let err = admit_ancestor_request(&hash(1), &mut requested, 100)
      .err()
      .map(|e| e.to_string())
      .unwrap_or_default();
    assert!(err.contains("cycle"), "got {err}");
  }

  /// The endpoint answering with something other than the requested block is
  /// the shape that made the walk unbounded: the child comes back, is pushed
  /// again, and reports the same missing parent forever.
  #[test]
  fn an_ancestor_that_is_not_the_requested_block_is_refused() {
    assert!(verify_ancestor_response(&hash(1), &hash(1)).is_ok());
    let err = verify_ancestor_response(&hash(1), &hash(2))
      .err()
      .map(|e| e.to_string())
      .unwrap_or_default();
    assert!(err.contains("asked for block"), "got {err}");
  }

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
  /// The live subsystems must stay idle while the projection is behind, and
  /// must not switch off for the block-or-two lag that is ordinary at the tip.
  /// Both halves matter: gating too eagerly makes the mempool flap during
  /// normal operation, and not gating at all is what had it issuing 47
  /// consecutive unanswerable queries against a partition whose index stepped
  /// sync had dropped.
  #[test]
  fn live_reads_wait_for_the_projection_to_reach_the_node() {
    let status = IngestStatus::new();
    // Nothing ingested yet, and no node height: not near the tip.
    assert!(!status.projection_near_tip());

    // Mid-sync, a million blocks behind.
    status.local_tip.store(1_204_010, Ordering::Relaxed);
    status.network_tip.store(2_525_888, Ordering::Relaxed);
    assert!(!status.projection_near_tip());

    // Just outside the margin.
    status.local_tip.store(
      2_525_888 - i64::from(LIVE_TIP_MARGIN_BLOCKS) - 1,
      Ordering::Relaxed,
    );
    assert!(!status.projection_near_tip());

    // Exactly at the margin, and fully caught up: both are near.
    status.local_tip.store(
      2_525_888 - i64::from(LIVE_TIP_MARGIN_BLOCKS),
      Ordering::Relaxed,
    );
    assert!(status.projection_near_tip());
    status.local_tip.store(2_525_888, Ordering::Relaxed);
    assert!(status.projection_near_tip());

    // Ahead of the node — briefly possible between a connect and the next
    // poll — must not read as "behind" through an underflow.
    status.local_tip.store(2_525_890, Ordering::Relaxed);
    assert!(status.projection_near_tip());
  }

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

  /// E2.5: the recovery indicator must be able to fire. It was hardcoded to
  /// `"clean"` while the console rendered it, so a reorg — routine or a full
  /// re-derivation — was invisible on the panel built to show it.
  #[test]
  fn a_reorg_shows_up_in_the_recovery_indicator() {
    let status = IngestStatus::new();
    assert_eq!(status.recovery_state(), ("clean", None));

    status.note_reorg(1_234_567, false);
    assert_eq!(status.recovery_state(), ("undo-replay", Some(1_234_567)));

    // A deep fork that needed full re-derivation must read differently: one is
    // routine, the other says the undo coverage was not enough.
    status.note_reorg(1_234_000, true);
    assert_eq!(status.recovery_state(), ("rederive", Some(1_234_000)));

    // It ages out rather than staying on forever.
    status
      .last_reorg_epoch
      .store(unix_now() - RECOVERY_STICKY_SECS - 1, Ordering::Relaxed);
    assert_eq!(status.recovery_state(), ("clean", None));
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

  /// The bounded-park decision, pinned directly: a park must not trip the
  /// exit before `PARK_TIMEOUT_SECS`, must trip at or past it, and "not
  /// currently parked" (`paused_since_epoch == 0`) must never trip it no
  /// matter how large `now` is — otherwise a freshly booted, never-parked
  /// daemon could read as needing to exit.
  #[test]
  fn park_should_exit_is_bounded_by_the_park_timeout() {
    let started = 1_000_000u64;
    assert!(!park_should_exit(started, started));
    assert!(!park_should_exit(started, started + PARK_TIMEOUT_SECS - 1));
    assert!(park_should_exit(started, started + PARK_TIMEOUT_SECS));
    assert!(park_should_exit(started, started + PARK_TIMEOUT_SECS + 100));
    assert!(!park_should_exit(0, started + PARK_TIMEOUT_SECS + 100));
  }

  /// The loop-heartbeat staleness decision, pinned directly: fresh reads
  /// alive, and it only flips at strictly more than
  /// `HEALTH_HEARTBEAT_STALE_SECS` of silence — comfortably past the longest
  /// single legitimate gap (one backoff sleep, capped at `MAX_BACKOFF_SECS`),
  /// which is the whole point of the margin.
  #[test]
  fn loop_heartbeat_staleness_has_margin_over_the_worst_case_backoff() {
    let now = 1_000_000u64;
    assert!(!loop_heartbeat_is_stale(now, now));
    assert!(!loop_heartbeat_is_stale(now - MAX_BACKOFF_SECS, now));
    assert!(!loop_heartbeat_is_stale(
      now - HEALTH_HEARTBEAT_STALE_SECS,
      now
    ));
    assert!(loop_heartbeat_is_stale(
      now - HEALTH_HEARTBEAT_STALE_SECS - 1,
      now
    ));
  }

  /// The whole point of `paused_since_epoch`: it survives across repeated
  /// `Pause`-classified failures (the clock must not reset each cycle, or the
  /// bounded-park timeout could never fire on a park that keeps re-failing),
  /// but resets the moment the loop observes the pause lifted, so the next
  /// park starts its own clock rather than inheriting the last one's age.
  #[test]
  fn pause_clock_survives_repeats_and_resets_on_clear() {
    let status = IngestStatus::new();
    assert_eq!(status.pause_elapsed_secs(), None);

    status.request_operator_pause();
    let first = status.paused_since_epoch.load(Ordering::Relaxed);
    assert!(first > 0);

    // A second failure while still parked must not move the clock.
    status.request_operator_pause();
    assert_eq!(status.paused_since_epoch.load(Ordering::Relaxed), first);
    assert!(status.pause_elapsed_secs().is_some());

    // An operator command lifts the flag; the loop observes that and clears
    // the clock (this is what `clear_pause_started` is called for, on the
    // non-paused path through the loop).
    status.pause_handle().store(false, Ordering::Relaxed);
    status.clear_pause_started();
    assert_eq!(status.pause_elapsed_secs(), None);

    // A fresh park after the clear starts its own clock, not the old one's.
    status.request_operator_pause();
    assert!(status.paused_since_epoch.load(Ordering::Relaxed) >= first);
  }
}
