//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Bounded, order-preserving fetch look-ahead over the node's REST interface.
//!
//! The cold-sync loop used to fetch one block, absorb it, and only then ask
//! the node for the next — two HTTP round trips serialised against three
//! Postgres round trips, per block, forever. Neither side was busy; each was
//! waiting for the other. This runs the fetch ahead so the absorb has a block
//! waiting for it.
//!
//! Ordering is structural, and for the same reason as the engine's
//! read-and-decode look-ahead (`hyphen_db`'s `ingest::prefetch`): the
//! scheduler claims a *slot* for each height, in height order, before spawning
//! the fetch that fills it, and the consumer drains slots in the order they
//! were claimed. A block that arrives early waits in its own slot. That
//! matters here even though the engine assigns heights itself — absorbing a
//! block whose parent has not been absorbed yet costs a `MissingParent` round
//! trip and an ancestor walk, which is precisely the serialisation this exists
//! to remove.
//!
//! Memory is bounded by [`hyphen_db::lead::LeadGauge`], the same gauge the
//! engine's look-ahead uses, so `[db.ingest] max_lead_blocks` means one thing
//! across the whole pipeline.
//!
//! Discarding: dropping the queue aborts the scheduler and closes the channel.
//! The absorb loop drops it whenever the engine reports a missing parent —
//! the seam a reorg arrives through — because every height queued behind that
//! point was fetched from a branch the engine has just said it cannot use.

use std::sync::Arc;

use hyphen_db::lead::{byte_budget, LeadGauge};
use tokio::sync::{mpsc, oneshot, Semaphore};
use tokio::task::JoinHandle;

use crate::error::{DaemonError, RestErrorKind};
use crate::sources::RestSource;

/// A block fetched ahead of the absorb.
#[derive(Debug)]
pub(crate) struct FetchedBlock {
  pub height: u32,
  pub raw: Vec<u8>,
  /// Held until the consumer drops the block, so the look-ahead bound is a
  /// bound on bytes *resident*, not bytes requested.
  _lease: hyphen_db::lead::Lease,
}

type Slot = oneshot::Receiver<Result<FetchedBlock, DaemonError>>;

/// Bounded look-ahead over a contiguous height range.
pub(crate) struct FetchQueue {
  rx: mpsc::Receiver<Slot>,
  scheduler: JoinHandle<()>,
  gauge: Arc<LeadGauge>,
  expect: Option<u32>,
}

impl FetchQueue {
  /// Fetch `lo..=hi` ahead of the caller, `[db.ingest] prefetch_blocks` deep,
  /// with at most `concurrency` requests outstanding against the node.
  ///
  /// Depth and concurrency are separate on purpose. The depth is what hides
  /// the writer's latency and wants to be large; the concurrency is what the
  /// node is asked to do at once and does not — `[core] rest_concurrency`
  /// already exists to say how many REST requests this daemon considers
  /// polite, and 8 of them at 2.24 ms each is two orders of magnitude more
  /// fetch capacity than the writer consumes.
  pub(crate) fn spawn(source: Arc<dyn RestSource>, concurrency: usize, lo: u32, hi: u32) -> Self {
    let prefetch = hyphen_db::tuning::prefetch_blocks().max(1);
    let lead = hyphen_db::tuning::max_lead_blocks().max(1);
    let gauge = LeadGauge::new(lead, byte_budget(lead));
    let (tx, rx) = mpsc::channel::<Slot>(prefetch.min(lead));
    let sched_gauge = Arc::clone(&gauge);
    let inflight = Arc::new(Semaphore::new(concurrency.max(1)));

    let scheduler = tokio::spawn(async move {
      for height in lo..=hi {
        // Admission in height order — see the gauge's own note on why it
        // cannot be taken by the workers.
        let mut lease = sched_gauge.admit().await;
        let Ok(slot) = tx.reserve().await else { return };
        let (done, waiting) = oneshot::channel();
        slot.send(waiting);

        let source = Arc::clone(&source);
        let inflight = Arc::clone(&inflight);
        tokio::spawn(async move {
          let outcome = async {
            // Released when the fetch finishes, never waiting on the consumer,
            // so out-of-order acquisition here cannot stall the ordered
            // admission above.
            let _permit = inflight
              .acquire()
              .await
              .map_err(|_| DaemonError::RestSource {
                message: "prefetch: fetch pool closed".to_owned(),
                kind: RestErrorKind::Other,
              })?;
            let hash = source.block_hash(height).await?;
            let raw = source.raw_block(&hash).await?;
            lease.correct_to(raw.len());
            Ok(FetchedBlock {
              height,
              raw,
              _lease: lease,
            })
          }
          .await;
          let _ = done.send(outcome);
        });
      }
    });

    Self {
      rx,
      scheduler,
      gauge,
      expect: Some(lo),
    }
  }

  /// The next block, in strict height order; `None` when the range is done.
  ///
  /// A fetch failure is delivered at its own height rather than skipped. The
  /// caller stops the chunk: a hole here would become a `MissingParent` and an
  /// ancestor walk at best, and a silently short sync at worst.
  pub(crate) async fn next(&mut self) -> Option<Result<FetchedBlock, DaemonError>> {
    let slot = self.rx.recv().await?;
    let item = match slot.await {
      Ok(item) => item,
      Err(_) => Err(DaemonError::RestSource {
        message: "prefetch: a queued block was abandoned before it was fetched".to_owned(),
        kind: RestErrorKind::Other,
      }),
    };
    Some(item.and_then(|b| self.check_order(b)))
  }

  /// Refuse a block that is not the one the absorb is owed. Unreachable while
  /// the slot channel is the delivery mechanism; kept so that a future change
  /// which breaks the invariant is a named refusal rather than an ancestor
  /// walk per block.
  fn check_order(&mut self, b: FetchedBlock) -> Result<FetchedBlock, DaemonError> {
    if let Some(expected) = self.expect {
      if b.height != expected {
        return Err(DaemonError::RestSource {
          message: format!(
            "prefetch delivered height {} where {expected} was expected — the \
             fetch queue is out of order",
            b.height
          ),
          kind: RestErrorKind::Other,
        });
      }
    }
    self.expect = Some(b.height.saturating_add(1));
    Ok(b)
  }

  /// Blocks currently held by the look-ahead, for the IPC heartbeat.
  pub(crate) fn in_flight(&self) -> usize {
    self.gauge.in_flight_blocks()
  }
}

impl Drop for FetchQueue {
  fn drop(&mut self) {
    self.scheduler.abort();
    self.rx.close();
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::sources::RestSource;
  use async_trait::async_trait;
  use std::collections::HashMap;
  use std::sync::atomic::{AtomicUsize, Ordering};
  use std::sync::Mutex;
  use std::time::Duration;

  /// A REST source that serves synthetic blocks, can be told to be slow in an
  /// order-hostile way, and can be told to fail at one height.
  #[derive(Debug)]
  struct FakeRest {
    tip: u32,
    /// Heights that fail with a 404-shaped error.
    missing: Vec<u32>,
    /// Later heights answer faster, so completion order fights height order.
    reverse_latency: bool,
    /// Bytes each block reports.
    size: usize,
    fetched: AtomicUsize,
    order: Mutex<Vec<u32>>,
  }

  impl FakeRest {
    fn new(tip: u32) -> Self {
      Self {
        tip,
        missing: Vec::new(),
        reverse_latency: false,
        size: 256,
        fetched: AtomicUsize::new(0),
        order: Mutex::new(Vec::new()),
      }
    }
  }

  #[async_trait]
  impl RestSource for FakeRest {
    async fn block_count(&self) -> Result<u32, DaemonError> {
      Ok(self.tip)
    }

    async fn block_hash(&self, height: u32) -> Result<String, DaemonError> {
      if self.missing.contains(&height) {
        return Err(DaemonError::RestSource {
          message: format!("rest: 404 for blockhashbyheight/{height}"),
          kind: RestErrorKind::Http(404),
        });
      }
      Ok(format!("{height:064x}"))
    }

    async fn raw_block(&self, hash: &str) -> Result<Vec<u8>, DaemonError> {
      let height = u32::from_str_radix(hash.trim_start_matches('0'), 16).unwrap_or(0);
      if self.reverse_latency {
        let wait = 40u64.saturating_sub(u64::from(height) * 4);
        tokio::time::sleep(Duration::from_millis(wait)).await;
      }
      self.fetched.fetch_add(1, Ordering::Relaxed);
      self
        .order
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .push(height);
      let mut raw = vec![0u8; self.size];
      raw[0] = height as u8;
      Ok(raw)
    }

    async fn fetch_blocks_batch(
      &self,
      heights: &[u32],
    ) -> Result<Vec<(String, Vec<u8>)>, DaemonError> {
      let mut out = Vec::new();
      for h in heights {
        let hash = self.block_hash(*h).await?;
        let raw = self.raw_block(&hash).await?;
        out.push((hash, raw));
      }
      Ok(out)
    }
  }

  type T = Result<(), DaemonError>;

  fn locked<X: Clone>(m: &Mutex<Vec<X>>) -> Vec<X> {
    m.lock().unwrap_or_else(|e| e.into_inner()).clone()
  }

  /// Drain a queue into the heights it delivered, stopping at the first
  /// failure and handing it back.
  async fn drain(q: &mut FetchQueue) -> (Vec<u32>, Option<DaemonError>) {
    let mut seen = Vec::new();
    while let Some(item) = q.next().await {
      match item {
        Ok(b) => seen.push(b.height),
        Err(e) => return (seen, Some(e)),
      }
    }
    (seen, None)
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn concurrent_fetch_still_arrives_in_height_order() -> T {
    let src = Arc::new(FakeRest {
      reverse_latency: true,
      ..FakeRest::new(64)
    });
    let mut q = FetchQueue::spawn(Arc::clone(&src) as Arc<dyn RestSource>, 8, 1, 8);
    let (seen, failure) = drain(&mut q).await;
    if let Some(e) = failure {
      return Err(e);
    }
    assert_eq!(seen, (1..=8).collect::<Vec<_>>());
    assert_ne!(
      locked(&src.order),
      (1..=8).collect::<Vec<_>>(),
      "the fake must actually complete out of order, or this proves nothing"
    );
    Ok(())
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn a_fetch_failure_surfaces_at_its_height_instead_of_stalling() -> T {
    let src = Arc::new(FakeRest {
      missing: vec![5],
      ..FakeRest::new(64)
    });
    let mut q = FetchQueue::spawn(src as Arc<dyn RestSource>, 8, 1, 10);
    let (seen, failure) = drain(&mut q).await;
    assert_eq!(
      seen,
      vec![1, 2, 3, 4],
      "everything below the missing height must still be delivered, in order"
    );
    let Some(err) = failure else {
      return Err(DaemonError::RestSource {
        message: "height 5 is missing and must have been refused".to_owned(),
        kind: RestErrorKind::Other,
      });
    };
    assert!(
      err.to_string().contains("404"),
      "the refusal must name the failure rather than stall: {err}"
    );
    Ok(())
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn discarding_the_queue_stops_fetching() {
    let src = Arc::new(FakeRest {
      reverse_latency: true,
      ..FakeRest::new(100_000)
    });
    let q = FetchQueue::spawn(Arc::clone(&src) as Arc<dyn RestSource>, 8, 1, 100_000);
    tokio::time::sleep(Duration::from_millis(60)).await;
    drop(q);
    let after_drop = src.fetched.load(Ordering::Relaxed);
    tokio::time::sleep(Duration::from_millis(200)).await;
    let settled = src.fetched.load(Ordering::Relaxed);
    assert!(
      settled - after_drop <= 64,
      "a discarded queue kept fetching: {after_drop} -> {settled} of 100,000"
    );
    assert!(
      settled < 100_000,
      "the whole range was fetched despite the discard"
    );
  }

  #[tokio::test(flavor = "multi_thread", worker_threads = 4)]
  async fn the_lead_bound_bounds_the_fetch_queue() -> T {
    // 1 MiB blocks: the count bound alone would authorise the whole range.
    let src = Arc::new(FakeRest {
      size: 1024 * 1024,
      ..FakeRest::new(4_000)
    });
    let mut q = FetchQueue::spawn(Arc::clone(&src) as Arc<dyn RestSource>, 8, 1, 400);
    let gauge = Arc::clone(&q.gauge);
    let (seen, failure) = drain(&mut q).await;
    if let Some(e) = failure {
      return Err(e);
    }
    assert_eq!(seen.len(), 400);

    let budget = byte_budget(hyphen_db::tuning::max_lead_blocks().max(1));
    assert!(
      gauge.peak_bytes() <= budget,
      "the fetch look-ahead charged {} bytes against a budget of {budget}",
      gauge.peak_bytes()
    );
    assert_eq!(
      gauge.in_flight_blocks(),
      0,
      "every lease must be released once the absorb has consumed the block"
    );

    let mut counts: HashMap<u32, usize> = HashMap::new();
    for h in locked(&src.order) {
      *counts.entry(h).or_default() += 1;
    }
    assert!(
      counts.values().all(|c| *c == 1),
      "the look-ahead must fetch each height exactly once"
    );
    Ok(())
  }
}
