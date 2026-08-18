//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The look-ahead memory bound, shared by both halves of the ingest seam.
//!
//! Task A4 runs two look-aheads: the daemon fetches blocks from the node ahead
//! of the engine, and the engine reads and decodes them ahead of the writer.
//! Both hold whole blocks in memory while the writer catches up, and both are
//! therefore one unbounded queue away from an out-of-memory kill on a chain
//! whose blocks reach 2 MB. They bound themselves the same way, with this, so
//! there is one definition of what `[db.ingest] max_lead_blocks` means rather
//! than two that can drift.
//!
//! The rule: a producer is admitted for one block only when the outstanding
//! charge leaves room for the largest block the chain produces, and the charge
//! is released when the *consumer* is finished with the block — not when the
//! producer is. So the gauge measures memory held, not work started.
//!
//! Two properties make it safe to use from an ordered pipeline:
//!
//! * **Admissions must be taken in delivery order**, by a single scheduler.
//!   Taken by the workers instead — fair or not — a later block could hold the
//!   budget the earliest block needs, and a consumer waiting on the earliest
//!   block would wait forever. Every caller here takes its lease in the
//!   scheduler, before spawning the work.
//! * **The charge starts as an over-estimate and is corrected downward.** A
//!   block's size is not knowable before it is fetched or read, and a nominal
//!   charge corrected *upward* would already be holding memory it then
//!   discovers it cannot afford. Reserving the largest block and handing the
//!   surplus back keeps the charge an upper bound on bytes held at every
//!   instant.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use tokio::sync::Notify;

/// The byte budget a `max_lead_blocks` count implies.
///
/// A count is not a memory bound: mainnet blocks reach 2 MB, so the production
/// `max_lead_blocks = 2048` would authorise 4 GB of queued bytes. Granting a
/// nominal budget per slot instead turns a run of large blocks into a shorter
/// queue rather than an out-of-memory kill, while a run of ordinary ~14 KB
/// blocks still fills the queue to its count bound.
#[must_use]
pub fn byte_budget(max_lead_blocks: usize) -> usize {
  max_lead_blocks
    .max(1)
    .saturating_mul(LEAD_BYTES_PER_BLOCK)
    .max(MIN_LEAD_RESERVATIONS * BLOCK_RESERVATION_BYTES)
}
/// Bytes of look-ahead budget granted per `max_lead_blocks`. The count bound
/// alone is not a memory bound: mainnet blocks reach 2 MB, so the production
/// `max_lead_blocks = 2048` would authorise 4 GB of queued bytes. Charging a
/// nominal budget per slot instead turns a run of large blocks into a shorter
/// queue rather than an out-of-memory kill, while a run of ordinary ~14 KB
/// blocks still fills the queue to its count bound.
pub const LEAD_BYTES_PER_BLOCK: usize = 128 * 1024;

/// What a slot is charged before its block's size is known.
///
/// Admission has to charge *something* up front, and the charge has to be an
/// over-estimate for the budget to be a real bound rather than a hope: bytes
/// arrive before their size can be measured, so a nominal charge corrected
/// upward afterwards would already be holding the memory it then discovers it
/// cannot afford. Reserving the largest block the chain produces and
/// correcting *down* keeps the charge an upper bound on bytes held at every
/// instant.
///
/// The cost is that admission can only run `budget / this` blocks ahead while
/// their sizes are unknown. That is not a throughput constraint: sizes become
/// known one local `ser_blocks` read later, and even the floor of four
/// in-flight reads is three orders of magnitude more read capacity than the
/// writer consumes.
pub const BLOCK_RESERVATION_BYTES: usize = 2 * 1024 * 1024;

/// Smallest useful byte budget: enough headroom for several blocks to be in
/// flight with their sizes still unknown. Without a floor, a small
/// `max_lead_blocks` would leave room for exactly one reservation and
/// serialise the look-ahead it was asked to parallelise.
pub const MIN_LEAD_RESERVATIONS: usize = 4;

#[derive(Debug, Default, Clone, Copy)]
struct GaugeState {
  blocks: usize,
  bytes: usize,
}

/// Admission control over the look-ahead, in blocks *and* bytes, with the
/// high-water marks kept so a test can assert the bound actually bounded
/// rather than assert that the arithmetic reads correctly.
#[derive(Debug)]
pub struct LeadGauge {
  state: Mutex<GaugeState>,
  space: Notify,
  max_blocks: usize,
  max_bytes: usize,
  peak_blocks: AtomicU64,
  peak_bytes: AtomicU64,
}

impl LeadGauge {
  /// A gauge admitting at most `max_blocks` blocks and `max_bytes` bytes.
  #[must_use]
  pub fn new(max_blocks: usize, max_bytes: usize) -> Arc<Self> {
    Arc::new(Self {
      state: Mutex::new(GaugeState::default()),
      space: Notify::new(),
      max_blocks: max_blocks.max(1),
      max_bytes: max_bytes.max(MIN_LEAD_RESERVATIONS * BLOCK_RESERVATION_BYTES),
      peak_blocks: AtomicU64::new(0),
      peak_bytes: AtomicU64::new(0),
    })
  }

  /// Highest number of blocks ever charged at once.
  pub fn peak_blocks(&self) -> usize {
    self.peak_blocks.load(Ordering::Relaxed) as usize
  }

  /// Highest byte charge ever outstanding at once.
  pub fn peak_bytes(&self) -> usize {
    self.peak_bytes.load(Ordering::Relaxed) as usize
  }

  /// Blocks currently charged (zero once a run has fully drained).
  pub fn in_flight_blocks(&self) -> usize {
    self.state.lock().unwrap_or_else(|e| e.into_inner()).blocks
  }

  /// Wait until one more block fits, then charge a full reservation for it.
  ///
  /// Only the scheduler task calls this, and it calls it in height order, so
  /// admissions cannot invert: the block the writer is waiting for is always
  /// the one admitted first. A semaphore acquired by the workers instead —
  /// fair or not — would let later blocks hold the budget the earliest block
  /// needs, and the writer would wait forever for a block that can never be
  /// funded.
  pub async fn admit(self: &Arc<Self>) -> Lease {
    loop {
      // Subscribe before testing, so a release between the test and the wait
      // cannot be missed.
      let waiting = self.space.notified();
      {
        let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
        let fits = st.blocks < self.max_blocks
          && st.bytes.saturating_add(BLOCK_RESERVATION_BYTES) <= self.max_bytes;
        if fits {
          st.blocks += 1;
          st.bytes += BLOCK_RESERVATION_BYTES;
          self.record_peaks(&st);
          return Lease {
            gauge: Arc::clone(self),
            charged: BLOCK_RESERVATION_BYTES,
          };
        }
      }
      waiting.await;
    }
  }

  fn record_peaks(&self, st: &GaugeState) {
    self
      .peak_blocks
      .fetch_max(st.blocks as u64, Ordering::Relaxed);
    self
      .peak_bytes
      .fetch_max(st.bytes as u64, Ordering::Relaxed);
  }

  fn recharge(&self, from: usize, to: usize) {
    {
      let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
      st.bytes = st.bytes.saturating_sub(from).saturating_add(to);
      self.record_peaks(&st);
    }
    self.space.notify_waiters();
  }

  fn release(&self, charged: usize) {
    {
      let mut st = self.state.lock().unwrap_or_else(|e| e.into_inner());
      st.blocks = st.blocks.saturating_sub(1);
      st.bytes = st.bytes.saturating_sub(charged);
    }
    self.space.notify_waiters();
  }
}

/// One block's claim on the look-ahead budget. Held from admission until the
/// writer has finished with the block, which is what makes the bound a bound
/// on *memory held* rather than on blocks started.
#[derive(Debug)]
pub struct Lease {
  gauge: Arc<LeadGauge>,
  charged: usize,
}

impl Lease {
  /// Replace the reservation with the block's real size, once known. Only
  /// ever a reduction in practice — the reservation is the largest block the
  /// chain produces — and a block bigger than that is still admitted rather
  /// than refused for being large.
  pub fn correct_to(&mut self, actual: usize) {
    self.gauge.recharge(self.charged, actual);
    self.charged = actual;
  }
}

impl Drop for Lease {
  fn drop(&mut self) {
    self.gauge.release(self.charged);
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  #[tokio::test]
  async fn a_lease_is_charged_the_reservation_until_it_is_corrected() {
    let g = LeadGauge::new(8, byte_budget(8));
    let mut lease = g.admit().await;
    assert_eq!(g.peak_bytes(), BLOCK_RESERVATION_BYTES);
    lease.correct_to(1024);
    assert_eq!(g.in_flight_blocks(), 1);
    drop(lease);
    assert_eq!(g.in_flight_blocks(), 0);
  }

  #[tokio::test]
  async fn admission_blocks_until_the_consumer_releases() {
    // Budget for exactly the floor: four reservations.
    let g = LeadGauge::new(64, MIN_LEAD_RESERVATIONS * BLOCK_RESERVATION_BYTES);
    let mut held = Vec::new();
    for _ in 0..MIN_LEAD_RESERVATIONS {
      held.push(g.admit().await);
    }
    assert_eq!(g.peak_blocks(), MIN_LEAD_RESERVATIONS);
    // A fifth cannot be admitted while the first four hold full reservations.
    let g2 = Arc::clone(&g);
    let fifth = tokio::spawn(async move { g2.admit().await });
    tokio::time::sleep(std::time::Duration::from_millis(30)).await;
    assert!(!fifth.is_finished(), "admission must wait for space");
    held.pop();
    fifth.await.expect("admitted once space freed");
  }

  #[tokio::test]
  async fn correcting_a_charge_down_makes_room_without_a_release() {
    let g = LeadGauge::new(64, MIN_LEAD_RESERVATIONS * BLOCK_RESERVATION_BYTES);
    let mut held: Vec<Lease> = Vec::new();
    for _ in 0..MIN_LEAD_RESERVATIONS {
      held.push(g.admit().await);
    }
    for lease in &mut held {
      lease.correct_to(4096);
    }
    // Four blocks of 4 KiB leave room for many more reservations.
    for _ in 0..MIN_LEAD_RESERVATIONS {
      held.push(g.admit().await);
    }
    assert!(g.peak_blocks() >= 2 * MIN_LEAD_RESERVATIONS);
  }

  #[test]
  fn the_budget_never_falls_below_a_few_whole_blocks() {
    assert_eq!(
      byte_budget(1),
      MIN_LEAD_RESERVATIONS * BLOCK_RESERVATION_BYTES,
      "a tiny lead bound must still leave room to run ahead at all"
    );
    assert_eq!(byte_budget(2048), 2048 * LEAD_BYTES_PER_BLOCK);
  }
}
