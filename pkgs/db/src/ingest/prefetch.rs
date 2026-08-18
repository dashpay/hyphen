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
  /// `[db.ingest] decode_workers`: how many blocks may be read and decoded at
  /// once. It caps connection use as well as CPU: the reads run on the write
  /// pool, and a look-ahead free to open a connection per queued block would
  /// take the pool the writer needs to begin its next transaction.
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
    let workers = Arc::new(Semaphore::new(cfg.workers()));
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
        let workers = Arc::clone(&workers);
        tokio::spawn(async move {
          let outcome = async {
            // One permit covers the read and the decode. It is released when
            // the work finishes and never waits on the writer, so workers
            // taking it out of height order cannot deadlock the ordered
            // admission above.
            let _permit = workers
              .acquire()
              .await
              .map_err(|_| HyDbError::Derive("prefetch: worker pool closed".to_owned()))?;
            let (src, raw) = read(pool, src, hash).await?;
            let raw_len = raw.len();
            // The reservation was an over-estimate; hand the surplus back so
            // the queue can run further ahead.
            lease.correct_to(raw_len);
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
  fn lazy_pool() -> HyDbResult<PgPool> {
    PgPool::connect_lazy("postgresql://postgres@127.0.0.1:5432/postgres")
      .map_err(|e| HyDbError::pg("test: lazy pool", e))
  }

  fn queue_over(
    segment: Vec<(HyDbHash, i32)>,
    cfg: PrefetchCfg,
    src: Arc<dyn HyDbDataSrc>,
    hold: Option<Arc<Semaphore>>,
  ) -> HyDbResult<DecodeQueue> {
    let gauge = LeadGauge::new(cfg.lead_blocks(), cfg.lead_bytes());
    Ok(DecodeQueue::spawn_with(
      lazy_pool()?,
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
    ))
  }

  fn cfg(prefetch: usize, workers: usize, lead: usize) -> PrefetchCfg {
    PrefetchCfg {
      prefetch_blocks: prefetch,
      decode_workers: workers,
      max_lead_blocks: lead,
    }
  }

  fn slow_src(delay_by_tag: bool) -> Arc<dyn HyDbDataSrc> {
    Arc::new(SlowSrc {
      delay_by_tag,
      decodes: AtomicUsize::new(0),
    })
  }

  /// Drain a queue, returning the heights delivered in the order they were
  /// delivered. Stops at the first failure and returns it.
  async fn drain(q: &mut DecodeQueue, pause_every: usize) -> HyDbResult<Vec<i32>> {
    let mut seen = Vec::new();
    while let Some(item) = q.next().await {
      seen.push(item?.height);
      if pause_every > 0 && seen.len() % pause_every == 0 {
        tokio::time::sleep(Duration::from_millis(5)).await;
      }
    }
    Ok(seen)
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn concurrent_decode_still_arrives_in_height_order() -> HyDbResult<()> {
    // Later heights are deliberately made to finish first.
    let mut q = queue_over(segment(1, 8), cfg(8, 8, 8), slow_src(true), None)?;
    assert_eq!(
      drain(&mut q, 0).await?,
      (1..=8).collect::<Vec<_>>(),
      "eight blocks decoded concurrently, with later heights finishing first, \
       must still be delivered 1..=8"
    );
    Ok(())
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn the_lead_bound_bounds_blocks_and_bytes() -> HyDbResult<()> {
    // 200 blocks offered, a lead bound of 3, and a writer that dawdles.
    let sizing = cfg(64, 8, 3);
    let mut q = queue_over(segment(1, 200), sizing, slow_src(false), None)?;
    let gauge = Arc::clone(q.gauge());

    assert_eq!(drain(&mut q, 20).await?.len(), 200);
    assert!(
      gauge.peak_blocks() <= 3,
      "look-ahead peaked at {} blocks against a bound of 3",
      gauge.peak_blocks()
    );
    assert!(
      gauge.peak_bytes() <= sizing.lead_bytes(),
      "look-ahead peaked at {} bytes against a budget of {}",
      gauge.peak_bytes(),
      sizing.lead_bytes()
    );
    assert_eq!(
      gauge.in_flight_blocks(),
      0,
      "every lease must be released once the writer has consumed the block"
    );
    Ok(())
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn big_blocks_shorten_the_queue_instead_of_filling_memory() -> HyDbResult<()> {
    // The count bound is generous; the blocks are not. If only the count were
    // enforced this queue would be entitled to 512 x 1 MiB resident at once.
    let sizing = cfg(512, 4, 512);
    let budget = sizing.lead_bytes();
    let gauge = LeadGauge::new(sizing.lead_blocks(), budget);
    let mut q = DecodeQueue::spawn_with(
      lazy_pool()?,
      slow_src(false),
      segment(1, 250),
      sizing,
      Arc::clone(&gauge),
      |_pool, src, hash| async move {
        let mut raw = vec![0u8; 1024 * 1024];
        raw[0] = hash.0[0];
        raw[32] = hash.0[0].saturating_sub(1);
        Ok((src, raw))
      },
    );

    assert_eq!(drain(&mut q, 25).await?.len(), 250);
    assert!(
      gauge.peak_bytes() <= budget,
      "charged {} bytes against a budget of {budget}",
      gauge.peak_bytes()
    );
    // The point of the byte bound: it, not the count, is what stopped the
    // queue. A count-only bound would have allowed all 512.
    assert!(
      gauge.peak_blocks() < sizing.lead_blocks(),
      "the byte budget should have bound before the count bound of {}, but the \
       queue reached {} blocks",
      sizing.lead_blocks(),
      gauge.peak_blocks()
    );
    assert!(
      gauge.peak_blocks() * 1024 * 1024 <= budget,
      "{} blocks of 1 MiB exceeds the {budget}-byte budget",
      gauge.peak_blocks()
    );
    Ok(())
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn a_read_failure_surfaces_at_its_height_instead_of_stalling() -> HyDbResult<()> {
    let gauge = LeadGauge::new(16, byte_budget(16));
    let mut q = DecodeQueue::spawn_with(
      lazy_pool()?,
      slow_src(false),
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

    let mut delivered = Vec::new();
    let mut failure = None;
    while let Some(item) = q.next().await {
      match item {
        Ok(block) => delivered.push(block.height),
        Err(e) => {
          failure = Some(e);
          break;
        }
      }
    }
    assert_eq!(
      delivered,
      vec![1, 2, 3],
      "everything below the failure must still be delivered, in order"
    );
    let failure = failure.ok_or_else(|| HyDbError::Derive("height 4 must fail".to_owned()))?;
    assert!(
      failure.to_string().contains("ser_blocks read refused"),
      "the failure must name itself rather than degrade into a stall: {failure}"
    );
    Ok(())
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn discarding_the_queue_releases_every_lease() -> HyDbResult<()> {
    let hold = Arc::new(Semaphore::new(0));
    let q = queue_over(
      segment(1, 64),
      cfg(16, 4, 16),
      slow_src(false),
      Some(Arc::clone(&hold)),
    )?;
    let gauge = Arc::clone(q.gauge());
    // Let the scheduler fill its admission window against blocked reads.
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(gauge.in_flight_blocks() > 0, "the queue should have filled");

    drop(q);
    hold.close();
    // The abandoned workers release their leases as they unwind.
    let mut charged = gauge.in_flight_blocks();
    for _ in 0..200 {
      if charged == 0 {
        break;
      }
      tokio::time::sleep(Duration::from_millis(5)).await;
      charged = gauge.in_flight_blocks();
    }
    assert_eq!(charged, 0, "discarding the queue left blocks charged");
    Ok(())
  }

  #[tokio::test]
  async fn an_out_of_order_delivery_is_refused_by_name() -> HyDbResult<()> {
    // The slot channel makes this unreachable in the real queue, so the guard
    // is exercised directly: it is the backstop that turns a future ordering
    // bug into a refusal rather than a misapplied chain.
    let gauge = LeadGauge::new(4, byte_budget(4));
    let (_tx, rx) = mpsc::channel::<Slot>(1);
    let mut q = DecodeQueue {
      rx,
      scheduler: tokio::spawn(async {}),
      gauge: Arc::clone(&gauge),
      expect: Some(100),
    };
    let out_of_turn = Prefetched {
      height: 101,
      raw_len: 0,
      block: crate::tests::mock::MockDataSrc.decode(&raw_for(101))?,
      _lease: gauge.admit().await,
    };
    let refused = q.check_order(out_of_turn);
    let err = match refused {
      Ok(_) => return Err(HyDbError::Derive("101 was accepted for 100".to_owned())),
      Err(e) => e,
    };
    assert!(err.to_string().contains("out of order"), "{err}");
    Ok(())
  }
}
