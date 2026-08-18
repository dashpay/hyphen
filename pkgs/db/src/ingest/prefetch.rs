//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Bounded, order-preserving read-and-decode look-ahead for the connect path.
//!
//! The write transaction used to do three things: read the block's raw bytes
//! back out of `ser_blocks`, decode them, and write the projection. Only the
//! third needs a transaction. The first two are pure with respect to the
//! branch — `ser_blocks` rows are content-addressed and never rewritten, and
//! [`HyDbDataSrc::decode`] is required to be a pure function of the bytes — so
//! they can run ahead of the writer, concurrently, while the previous block's
//! transaction is still committing. That is the whole of this module.
//!
//! **Ordering is structural, not checked after the fact.** The scheduler walks
//! the segment in height order and, for each block, pushes a *slot* (a
//! [`oneshot::Receiver`]) onto a bounded channel before spawning the work that
//! will fill it. The consumer awaits slots in the order they were pushed, so a
//! block that finishes decoding early simply sits in its own slot until its
//! turn. There is no reorder buffer to get wrong and no comparison to forget:
//! a worker cannot deliver out of turn because it has nowhere else to deliver
//! to. [`DecodeQueue::next`] additionally refuses a height it did not expect,
//! so a future change that breaks the invariant stops the sync with a named
//! error instead of writing blocks in the wrong order.
//!
//! **The queue is discardable.** Dropping it aborts the scheduler and closes
//! the slot channel; in-flight workers find their oneshot gone and exit. A
//! reorg abandons a branch by dropping the queue built for it — nothing from
//! the old branch can reach the writer afterwards, because the writer only
//! ever reads from the queue it currently holds.
//!
//! **Memory is bounded by admission, in both blocks and bytes.** See
//! [`LeadGauge`]: the scheduler is admitted for block *n* only when the
//! in-flight charge leaves room for a whole block, and the charge is released
//! when the writer is finished with it. Because admission happens in the
//! single scheduler task, in height order, a large block can never be starved
//! by later small ones — the deadlock a fair-but-unordered semaphore would
//! allow.

use std::future::Future;
use std::sync::Arc;

use sqlx::PgPool;
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::task::JoinHandle;

use super::chain;
use crate::error::{HyDbError, HyDbResult};
use crate::lead::{byte_budget, LeadGauge};
use crate::records::{HyDbBlock, HyDbDataSrc};
use crate::types::HyDbHash;

/// Sizing of one look-ahead run, resolved from `[db.ingest]` at the call site
/// so this module stays testable without the process-wide config.
#[derive(Debug, Clone, Copy)]
pub(super) struct PrefetchCfg {
  /// `[db.ingest] prefetch_blocks`: slots the scheduler may run ahead by.
  pub prefetch_blocks: usize,
  /// `[db.ingest] decode_workers`: concurrent decodes.
  pub decode_workers: usize,
  /// `[db.ingest] max_lead_blocks`: hard bound on read-or-decoded blocks that
  /// the writer has not yet consumed.
  pub max_lead_blocks: usize,
}

impl PrefetchCfg {
  /// The sizing in force, from the process-wide `[db.ingest]`.
  pub(super) fn resolved() -> Self {
    Self {
      prefetch_blocks: crate::tuning::prefetch_blocks(),
      decode_workers: crate::tuning::decode_workers(),
      max_lead_blocks: crate::tuning::max_lead_blocks(),
    }
  }

  /// Slots held in the channel. At least one, and never more than the hard
  /// lead bound — `HyDbIngestCfg::validate` already refuses the inverted case,
  /// but this module must not deadlock if it is ever called directly.
  fn slots(&self) -> usize {
    self.prefetch_blocks.max(1).min(self.lead_blocks())
  }

  fn lead_blocks(&self) -> usize {
    self.max_lead_blocks.max(1)
  }

  /// Byte budget for the whole queue.
  fn lead_bytes(&self) -> usize {
    byte_budget(self.lead_blocks())
  }

  fn workers(&self) -> usize {
    self.decode_workers.max(1)
  }
}

// ===========================================================================
// The queue
// ===========================================================================

/// A block read and decoded ahead of the writer. Holds its budget lease, so
/// the look-ahead only opens up again once the writer drops this.
pub(super) struct Prefetched {
  pub height: i32,
  /// Serialized size of the block. The only thing the connect needs the raw
  /// bytes *for* once the block is decoded (`blocks.size`, and the metric
  /// inputs), so the bytes themselves are dropped at the decode worker rather
  /// than carried through the queue.
  pub raw_len: i64,
  pub block: HyDbBlock,
  _lease: crate::lead::Lease,
}

impl std::fmt::Debug for Prefetched {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Prefetched")
      .field("height", &self.height)
      .field("raw_len", &self.raw_len)
      .finish_non_exhaustive()
  }
}

type Slot = oneshot::Receiver<HyDbResult<Prefetched>>;

/// Bounded look-ahead over one planned segment.
pub(super) struct DecodeQueue {
  rx: mpsc::Receiver<Slot>,
  scheduler: JoinHandle<()>,
  #[cfg(test)]
  gauge: Arc<LeadGauge>,
  expect: Option<i32>,
}

impl DecodeQueue {
  /// Start reading and decoding `segment` (in the order given, which the
  /// planner guarantees is ascending height).
  pub(super) fn spawn(
    pool: PgPool,
    data_src: Arc<dyn HyDbDataSrc>,
    segment: Vec<(HyDbHash, i32)>,
    cfg: PrefetchCfg,
  ) -> Self {
    let gauge = LeadGauge::new(cfg.lead_blocks(), cfg.lead_bytes());
    Self::spawn_with(
      pool,
      data_src,
      segment,
      cfg,
      gauge,
      |pool, src, hash| async move { chain::raw_bytes(&pool, &hash).await.map(|raw| (src, raw)) },
    )
  }

  /// The queue over a caller-supplied reader, so tests can drive ordering,
  /// failure and bound behaviour without a database.
  fn spawn_with<R, F>(
    pool: PgPool,
    data_src: Arc<dyn HyDbDataSrc>,
    segment: Vec<(HyDbHash, i32)>,
    cfg: PrefetchCfg,
    gauge: Arc<LeadGauge>,
    read: R,
  ) -> Self
  where
    R: Fn(PgPool, Arc<dyn HyDbDataSrc>, HyDbHash) -> F + Send + Sync + 'static,
    F: Future<Output = HyDbResult<(Arc<dyn HyDbDataSrc>, Vec<u8>)>> + Send + 'static,
  {
    let (tx, rx) = mpsc::channel::<Slot>(cfg.slots());
    let decoding = Arc::new(Semaphore::new(cfg.workers()));
    let expect = segment.first().map(|(_, h)| *h);
    let read = Arc::new(read);
    let sched_gauge = Arc::clone(&gauge);

    let scheduler = tokio::spawn(async move {
      for (hash, height) in segment {
        // Admission first, in height order: this is the memory bound, and
        // taking it here is what makes it deadlock-free.
        let mut lease = sched_gauge.admit().await;
        // Then a slot. `reserve` fails only when the consumer is gone, which
        // is how a discarded queue stops its own scheduler.
        let Ok(slot) = tx.reserve().await else { return };
        let (done, waiting) = oneshot::channel();
        slot.send(waiting);

        let pool = pool.clone();
        let src = Arc::clone(&data_src);
        let read = Arc::clone(&read);
        let decoding = Arc::clone(&decoding);
        tokio::spawn(async move {
          let outcome = async {
            let (src, raw) = read(pool, src, hash).await?;
            let raw_len = raw.len();
            // The reservation was an over-estimate; hand the surplus back so
            // the queue can run further ahead.
            lease.correct_to(raw_len);
            // Decode is synchronous CPU work; `decode_workers` caps how much
            // of it runs at once so the look-ahead cannot swamp the runtime
            // the writer also needs.
            let _permit = decoding
              .acquire()
              .await
              .map_err(|_| HyDbError::Derive("prefetch: decode pool closed".to_owned()))?;
            let block = tokio::task::spawn_blocking(move || src.decode(&raw))
              .await
              .map_err(|e| HyDbError::Derive(format!("prefetch: decode task failed: {e}")))??;
            Ok(Prefetched {
              height,
              raw_len: raw_len as i64,
              block,
              _lease: lease,
            })
          }
          .await;
          // A closed receiver means the queue was discarded; the result is
          // simply dropped, which also drops the lease.
          let _ = done.send(outcome);
        });
      }
    });

    Self {
      rx,
      scheduler,
      #[cfg(test)]
      gauge,
      expect,
    }
  }

  /// The next block, in strict height order. `None` once the segment is
  /// exhausted.
  ///
  /// A read or decode failure is delivered here, at the failing height, rather
  /// than skipped — a hole in the middle of a segment would be a hole in the
  /// chain. The caller stops the advance; everything below the failure is
  /// already committed.
  pub(super) async fn next(&mut self) -> Option<HyDbResult<Prefetched>> {
    let slot = self.rx.recv().await?;
    let item = match slot.await {
      Ok(item) => item,
      // The worker was dropped without sending: only reachable if the runtime
      // tore the task down. Name it rather than ending the segment quietly,
      // which would look like a successful short advance.
      Err(_) => Err(HyDbError::Derive(
        "prefetch: a queued block was abandoned before it was decoded".to_owned(),
      )),
    };
    Some(item.and_then(|p| self.check_order(p)))
  }

  /// Refuse a block that is not the one the writer is owed. The slot channel
  /// makes this unreachable; it is here so that if it ever becomes reachable
  /// the sync stops with a named error instead of applying blocks in the
  /// wrong order and calling the result a chain.
  fn check_order(&mut self, p: Prefetched) -> HyDbResult<Prefetched> {
    if let Some(expected) = self.expect {
      if p.height != expected {
        return Err(HyDbError::Derive(format!(
          "prefetch delivered height {} where {expected} was expected — the \
           look-ahead queue is out of order and the segment cannot be applied",
          p.height
        )));
      }
    }
    self.expect = Some(p.height + 1);
    Ok(p)
  }

  /// The admission gauge, for assertions about the bound.
  #[cfg(test)]
  pub(super) fn gauge(&self) -> &Arc<LeadGauge> {
    &self.gauge
  }
}

impl Drop for DecodeQueue {
  fn drop(&mut self) {
    // Discard: stop scheduling, and close the channel so in-flight workers
    // find nowhere to deliver and release their leases.
    self.scheduler.abort();
    self.rx.close();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::lead::LEAD_BYTES_PER_BLOCK;
  use std::sync::atomic::{AtomicUsize, Ordering};
  use std::time::Duration;

  /// A data source that decodes the synthetic `hash(32) || prev(32)`
  /// encoding and can be told to take its time, so completion order and
  /// height order genuinely differ.
  #[derive(Debug)]
  struct SlowSrc {
    /// Milliseconds to burn per decode, keyed by the block's first byte.
    delay_by_tag: bool,
    decodes: AtomicUsize,
  }

  impl HyDbDataSrc for SlowSrc {
    fn identify(&self, raw: &[u8]) -> HyDbResult<crate::records::HyDbBlockId> {
      crate::tests::mock::MockDataSrc.identify(raw)
    }

    fn decode(&self, raw: &[u8]) -> HyDbResult<HyDbBlock> {
      if self.delay_by_tag {
        // Later blocks decode faster, so completion order is the reverse of
        // height order unless the queue enforces it.
        let tag = u64::from(raw[0]);
        std::thread::sleep(Duration::from_millis(40u64.saturating_sub(tag * 4)));
      }
      self.decodes.fetch_add(1, Ordering::Relaxed);
      crate::tests::mock::MockDataSrc.decode(raw)
    }
  }

  fn raw_for(height: i32) -> Vec<u8> {
    let mut raw = vec![0u8; 72];
    raw[0] = height as u8;
    raw[32] = height.saturating_sub(1) as u8;
    raw
  }

  fn segment(lo: i32, hi: i32) -> Vec<(HyDbHash, i32)> {
    (lo..=hi)
      .map(|h| {
        let mut b = [0u8; 32];
        b[0] = h as u8;
        (HyDbHash(b), h)
      })
      .collect()
  }

  /// A pool value is needed for the real reader's signature but never
  /// connected to in these tests; `PgPool::connect_lazy` does no I/O.
  fn lazy_pool() -> PgPool {
    PgPool::connect_lazy("postgresql://postgres@127.0.0.1:5432/postgres").expect("lazy pool")
  }

  fn queue_over(
    segment: Vec<(HyDbHash, i32)>,
    cfg: PrefetchCfg,
    src: Arc<dyn HyDbDataSrc>,
    hold: Option<Arc<Semaphore>>,
  ) -> DecodeQueue {
    let gauge = LeadGauge::new(cfg.lead_blocks(), cfg.lead_bytes());
    DecodeQueue::spawn_with(
      lazy_pool(),
      src,
      segment,
      cfg,
      gauge,
      move |_pool, src, hash| {
        let hold = hold.clone();
        async move {
          if let Some(hold) = hold {
            // Zero permits: every read parks here until the gate is closed,
            // and closing it releases all of them, now and later.
            let _ = hold.acquire().await;
          }
          Ok((src, raw_for(i32::from(hash.0[0]))))
        }
      },
    )
  }

  fn cfg(prefetch: usize, workers: usize, lead: usize) -> PrefetchCfg {
    PrefetchCfg {
      prefetch_blocks: prefetch,
      decode_workers: workers,
      max_lead_blocks: lead,
    }
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn concurrent_decode_still_arrives_in_height_order() {
    let src = Arc::new(SlowSrc {
      delay_by_tag: true,
      decodes: AtomicUsize::new(0),
    });
    let mut q = queue_over(segment(1, 8), cfg(8, 8, 8), src, None);

    let mut seen = Vec::new();
    while let Some(item) = q.next().await {
      seen.push(item.expect("decoded").height);
    }
    assert_eq!(
      seen,
      (1..=8).collect::<Vec<_>>(),
      "eight blocks decoded concurrently, with later heights deliberately \
       finishing first, must still be delivered 1..=8"
    );
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn the_lead_bound_bounds_blocks_and_bytes() {
    let src = Arc::new(SlowSrc {
      delay_by_tag: false,
      decodes: AtomicUsize::new(0),
    });
    // 200 blocks offered, a lead bound of 3. The writer is deliberately slow.
    let mut q = queue_over(segment(1, 200), cfg(64, 8, 3), src, None);
    let gauge = Arc::clone(q.gauge());

    let mut count = 0;
    while let Some(item) = q.next().await {
      item.expect("decoded");
      count += 1;
      if count % 20 == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
      }
    }
    assert_eq!(count, 200);
    assert!(
      gauge.peak_blocks() <= 3,
      "look-ahead peaked at {} blocks against a bound of 3",
      gauge.peak_blocks()
    );
    assert!(
      gauge.peak_bytes() <= cfg(64, 8, 3).lead_bytes(),
      "look-ahead peaked at {} bytes against a budget of {}",
      gauge.peak_bytes(),
      cfg(64, 8, 3).lead_bytes()
    );
    assert_eq!(
      gauge.in_flight_blocks(),
      0,
      "every lease must be released once the writer has consumed the block"
    );
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn a_read_failure_surfaces_at_its_height_instead_of_stalling() {
    let src: Arc<dyn HyDbDataSrc> = Arc::new(SlowSrc {
      delay_by_tag: false,
      decodes: AtomicUsize::new(0),
    });
    let gauge = LeadGauge::new(16, 16 * LEAD_BYTES_PER_BLOCK);
    let mut q = DecodeQueue::spawn_with(
      lazy_pool(),
      src,
      segment(1, 6),
      cfg(6, 4, 16),
      gauge,
      |_pool, src, hash| async move {
        if hash.0[0] == 4 {
          return Err(HyDbError::Derive("ser_blocks read refused".to_owned()));
        }
        Ok((src, raw_for(i32::from(hash.0[0]))))
      },
    );

    assert_eq!(q.next().await.unwrap().unwrap().height, 1);
    assert_eq!(q.next().await.unwrap().unwrap().height, 2);
    assert_eq!(q.next().await.unwrap().unwrap().height, 3);
    let err = q.next().await.unwrap().expect_err("height 4 must fail");
    assert!(
      err.to_string().contains("ser_blocks read refused"),
      "the failure must name itself, not degrade into a stall: {err}"
    );
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn big_blocks_shorten_the_queue_instead_of_filling_memory() {
    // The count bound is generous; the blocks are not. If only the count were
    // enforced this queue would be entitled to 512 x 1 MiB in memory at once.
    let cfg = cfg(512, 4, 512);
    let budget = cfg.lead_bytes();
    let src: Arc<dyn HyDbDataSrc> = Arc::new(SlowSrc {
      delay_by_tag: false,
      decodes: AtomicUsize::new(0),
    });
    let gauge = LeadGauge::new(cfg.lead_blocks(), budget);
    let mut q = DecodeQueue::spawn_with(
      lazy_pool(),
      src,
      segment(1, 250),
      cfg,
      Arc::clone(&gauge),
      |_pool, src, hash| async move {
        let mut raw = vec![0u8; 1024 * 1024];
        raw[0] = hash.0[0];
        raw[32] = hash.0[0].saturating_sub(1);
        Ok((src, raw))
      },
    );

    let mut count = 0;
    while let Some(item) = q.next().await {
      item.expect("decoded");
      count += 1;
      if count % 25 == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
      }
    }
    assert_eq!(count, 250, "every block must still be delivered");
    assert!(
      gauge.peak_bytes() <= budget,
      "charged {} bytes against a budget of {budget}",
      gauge.peak_bytes()
    );
    // The point of the byte bound: it, not the count, is what stopped the
    // queue. A count-only bound would have allowed all 512.
    assert!(
      gauge.peak_blocks() < cfg.lead_blocks(),
      "the byte budget should have bound before the count bound of {}, but \
       the queue reached {} blocks",
      cfg.lead_blocks(),
      gauge.peak_blocks()
    );
    assert!(
      gauge.peak_blocks() as u64 * 1024 * 1024 <= budget as u64,
      "{} blocks of 1 MiB exceeds the {budget}-byte budget",
      gauge.peak_blocks()
    );
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn discarding_the_queue_releases_every_lease() {
    let src = Arc::new(SlowSrc {
      delay_by_tag: false,
      decodes: AtomicUsize::new(0),
    });
    let hold = Arc::new(Semaphore::new(0));
    let q = queue_over(segment(1, 64), cfg(16, 4, 16), src, Some(Arc::clone(&hold)));
    let gauge = Arc::clone(q.gauge());
    // Let the scheduler fill its admission window against blocked reads.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(gauge.in_flight_blocks() > 0, "the queue should have filled");

    drop(q);
    hold.close();
    // The abandoned workers release their leases as they unwind.
    for _ in 0..200 {
      if gauge.in_flight_blocks() == 0 {
        return;
      }
      tokio::time::sleep(Duration::from_millis(5)).await;
    }
    panic!(
      "discarding the queue left {} blocks charged",
      gauge.in_flight_blocks()
    );
  }

  #[tokio::test]
  async fn an_out_of_order_delivery_is_refused_by_name() {
    // The slot channel makes this unreachable in the real queue, so the guard
    // is exercised directly: it is the backstop that turns a future ordering
    // bug into a refusal rather than a misapplied chain.
    let gauge = LeadGauge::new(4, 4 * LEAD_BYTES_PER_BLOCK);
    let (_tx, rx) = mpsc::channel::<Slot>(1);
    let mut q = DecodeQueue {
      rx,
      scheduler: tokio::spawn(async {}),
      gauge: Arc::clone(&gauge),
      expect: Some(100),
    };
    let lease = gauge.admit().await;
    let out_of_turn = Prefetched {
      height: 101,
      raw_len: 0,
      block: crate::tests::mock::MockDataSrc
        .decode(&raw_for(101))
        .expect("mock decode"),
      _lease: lease,
    };
    let err = q.check_order(out_of_turn).expect_err("101 is not 100");
    assert!(err.to_string().contains("out of order"), "{err}");
  }
}
