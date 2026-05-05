//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Block ingest state machine: REST -> RocksDB.

mod admit;
mod commit;
mod diag;
mod driver;
mod fetch;
mod poll;
mod reconcile;
mod runtime;
mod state;
mod watchdog;

#[cfg(feature = "bench")]
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
#[cfg(feature = "bench")]
use std::sync::OnceLock;

use crate::sql::SqlPool;
use async_trait::async_trait;
use rocksdb::DB;

use crate::index::BlockIndexerWorker;
use crate::lifecycle::Lifecycle;
use crate::sources::RestSource;
use crate::store::manager::SecondaryStoreSet;
use crate::visor::{
  Fenceable, ProgressState, RecoveryDisposition, RunState, ShutdownPhase, Supervisable,
};
use crate::worker::{SupervisedTaskSpec, WorkerRuntimeState, WorkerTask};

pub use diag::build_ingestion_diag;
pub use driver::run;
pub use runtime::{IngestRuntime, IngestRuntimeSnapshot};

pub const DEFAULT_MAX_INGEST_LEAD_BLOCKS: u32 = 10_000;
#[cfg(feature = "bench")]
const BENCH_BATCH_SIZE: u32 = 32;
#[cfg(feature = "bench")]
const BENCH_NETWORK_TIP: u32 = 32;
#[cfg(feature = "bench")]
const BENCH_MAX_LEAD_BLOCKS: u32 = 200_000;
#[cfg(feature = "bench")]
static BENCH_CONTEXT: OnceLock<IngestBenchContext> = OnceLock::new();
#[cfg(feature = "bench")]
static BENCH_COUNTER: AtomicU64 = AtomicU64::new(0);

/// Shared dependencies for the ingest state machine.
#[derive(Clone)]
pub(crate) struct IngestCtx {
  pub db: Arc<DB>,
  pub secondary_stores: Option<Arc<SecondaryStoreSet>>,
  pub pool: SqlPool,
  pub source: Arc<dyn RestSource>,
  pub lifecycle: Arc<Lifecycle>,
  pub runtime: Arc<IngestRuntime>,
  pub block_worker: Arc<BlockIndexerWorker>,
  pub chain_state_lock: Arc<tokio::sync::RwLock<()>>,
  pub batch_size: u32,
  pub max_lead_blocks: u32,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub block_time_secs: f64,
}

impl std::fmt::Debug for IngestCtx {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("IngestCtx").finish_non_exhaustive()
  }
}

#[derive(Clone, Debug)]
pub(crate) struct IngestWorker {
  lifecycle: Arc<Lifecycle>,
  runtime: Arc<WorkerRuntimeState>,
}

impl IngestWorker {
  fn new(lifecycle: Arc<Lifecycle>) -> Self {
    Self {
      lifecycle,
      runtime: WorkerRuntimeState::shared(),
    }
  }

  fn current_shutdown_phase(&self) -> ShutdownPhase {
    if self.lifecycle.terminated() {
      ShutdownPhase::Forcing
    } else if self.lifecycle.interrupted() {
      ShutdownPhase::Interrupting
    } else {
      ShutdownPhase::Running
    }
  }
}

#[async_trait]
impl Supervisable for IngestWorker {
  type RunId = u64;

  fn run_state(&self) -> RunState<Self::RunId> {
    let progress_state = if self.runtime.recovering() {
      ProgressState::Recovering
    } else if self.runtime.finished() || !self.lifecycle.running() {
      ProgressState::Idle
    } else if self.runtime.started() && !self.runtime.progress_started() {
      ProgressState::Starting
    } else if self.runtime.started() {
      ProgressState::Active
    } else {
      ProgressState::Idle
    };
    RunState {
      run_id: self.runtime.run_id(),
      phase: "ingestion",
      subphase: None,
      detail: None,
      progress_state,
      last_activity_epoch: self.runtime.last_activity_epoch(),
      last_progress_epoch: self.runtime.last_progress_epoch(),
      recovery_attempts: self.runtime.recovery_attempts(),
      shutdown_phase: self.current_shutdown_phase(),
    }
  }

  fn interrupt(&self) {
    self.lifecycle.interrupt();
  }

  fn recovery_disposition(&self, _run_id: Option<Self::RunId>) -> RecoveryDisposition {
    match self.current_shutdown_phase() {
      ShutdownPhase::Interrupting => RecoveryDisposition::Blocked("interrupt requested"),
      ShutdownPhase::Forcing => RecoveryDisposition::Blocked("shutdown in progress"),
      ShutdownPhase::Running => RecoveryDisposition::Blocked("worker is not restartable"),
    }
  }

  fn watchdog_disposition(
    &self,
    _state: &RunState<Self::RunId>,
    _now_epoch: u64,
  ) -> RecoveryDisposition {
    match self.current_shutdown_phase() {
      ShutdownPhase::Interrupting => RecoveryDisposition::Blocked("interrupt requested"),
      ShutdownPhase::Forcing => RecoveryDisposition::Blocked("shutdown in progress"),
      ShutdownPhase::Running => RecoveryDisposition::Blocked("worker is not restartable"),
    }
  }

  async fn recover(&self, _run_id: Option<Self::RunId>) -> Result<(), crate::error::DaemonError> {
    Err(crate::error::DaemonError::Config(
      "ingestion worker is not restartable".to_owned(),
    ))
  }
}

impl Fenceable for IngestWorker {
  fn fence(&self, _run_id: Option<Self::RunId>) {}
}

#[must_use]
pub(crate) fn ingestion_supervised_task(ctx: IngestCtx) -> SupervisedTaskSpec<IngestWorker> {
  let worker = IngestWorker::new(Arc::clone(&ctx.lifecycle));
  let runtime = Arc::clone(&worker.runtime);
  let initial = WorkerTask::new(worker, {
    let ctx = ctx.clone();
    async move {
      runtime.mark_started();
      let result = run(ctx).await;
      runtime.mark_finished();
      result
    }
  });
  SupervisedTaskSpec::new("ingestion", initial, move || {
    Err(crate::error::DaemonError::Config(
      "ingestion worker is not restartable".to_owned(),
    ))
  })
}

#[cfg(feature = "bench")]
#[derive(Debug)]
struct IngestBenchContext {
  ctx: IngestCtx,
  batch: state::InFlightBatch,
}

#[cfg(feature = "bench")]
#[derive(Debug)]
struct FakeRestSource {
  network_tip: u32,
  blocks: std::collections::BTreeMap<u32, Vec<u8>>,
  hashes: std::collections::BTreeMap<u32, String>,
}

#[cfg(feature = "bench")]
impl FakeRestSource {
  fn new(network_tip: u32) -> Self {
    let blocks: std::collections::BTreeMap<u32, Vec<u8>> = (1..=network_tip)
      .map(|height| (height, minimal_block_bytes(height)))
      .collect();
    let hashes: std::collections::BTreeMap<u32, String> = blocks
      .iter()
      .map(|(&height, raw)| {
        let hash =
          crate::consensus::block_hash(raw).expect("benchmark raw blocks should hash");
        (
          height,
          crate::utils::view_hex(&hash.to_bytes(), crate::utils::HashEndian::Ser),
        )
      })
      .collect();

    Self {
      network_tip,
      blocks,
      hashes,
    }
  }
}

#[cfg(feature = "bench")]
#[async_trait]
impl RestSource for FakeRestSource {
  async fn block_count(&self) -> Result<u32, crate::error::DaemonError> {
    Ok(self.network_tip)
  }

  async fn block_hash(&self, height: u32) -> Result<String, crate::error::DaemonError> {
    self.hashes.get(&height).cloned().ok_or_else(|| {
      crate::error::DaemonError::RestSource(format!("missing benchmark hash for height {height}"))
    })
  }

  async fn raw_block(&self, hash: &str) -> Result<Vec<u8>, crate::error::DaemonError> {
    self
      .hashes
      .iter()
      .find_map(|(height, expected)| {
        if expected == hash {
          self.blocks.get(height).cloned()
        } else {
          None
        }
      })
      .ok_or_else(|| {
        crate::error::DaemonError::RestSource(format!("missing benchmark block for hash {hash}"))
      })
  }

  async fn fetch_blocks_batch(
    &self,
    heights: &[u32],
  ) -> Result<Vec<(String, Vec<u8>)>, crate::error::DaemonError> {
    heights
      .iter()
      .map(|height| {
        let hash = self.hashes.get(height).cloned().ok_or_else(|| {
          crate::error::DaemonError::RestSource(format!(
            "missing benchmark hash for height {height}"
          ))
        })?;
        let raw = self.blocks.get(height).cloned().ok_or_else(|| {
          crate::error::DaemonError::RestSource(format!(
            "missing benchmark block for height {height}"
          ))
        })?;
        Ok((hash, raw))
      })
      .collect()
  }
}

#[cfg(all(test, feature = "bench"))]
mod tests {
  use super::FakeRestSource;
  use crate::sources::RestSource;

  #[tokio::test]
  async fn fake_rest_source_returns_view_endian_block_hashes(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let source = FakeRestSource::new(1);
    let hash = source.block_hash(1).await?;
    let raw = source.raw_block(&hash).await?;
    let computed = crate::consensus::block_hash(&raw)?.to_bytes();
    let expected = crate::utils::view_hex(&computed, crate::utils::HashEndian::Ser);
    assert_eq!(hash, expected);
    Ok(())
  }
}

#[cfg(feature = "bench")]
fn bench_context() -> &'static IngestBenchContext {
  BENCH_CONTEXT.get_or_init(|| {
    let pool = hyphend::bench::runtime().pool().clone();
    let timestamp = std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .map(|duration| duration.as_nanos())
      .unwrap_or_default();
    let counter = BENCH_COUNTER.fetch_add(1, Ordering::Relaxed);
    let db_path = std::env::temp_dir().join(format!("hyphen-ingest-bench-{timestamp}-{counter}"));
    let db = crate::db::open(&db_path, None).expect("ingest benchmark rocksdb should open");
    let _ = std::fs::remove_dir_all(&db_path);
    let db = Arc::new(db);
    let runtime = Arc::new(IngestRuntime::default());
    let lifecycle = crate::lifecycle::Lifecycle::start();
    let scheduler_state = Arc::new(std::sync::Mutex::new(
      crate::index::runner::IndexSchedulerState::default(),
    ));
    let block_worker = Arc::new(BlockIndexerWorker::new(
      Arc::clone(&db),
      Arc::clone(&runtime),
      Arc::clone(&lifecycle),
      Arc::clone(&scheduler_state),
      Arc::new(std::sync::atomic::AtomicBool::new(false)),
    ));
    let source = Arc::new(FakeRestSource::new(BENCH_NETWORK_TIP));
    let ctx = IngestCtx {
      db,
      secondary_stores: None,
      pool,
      source,
      lifecycle,
      runtime,
      block_worker,
      chain_state_lock: Arc::new(tokio::sync::RwLock::new(())),
      batch_size: BENCH_BATCH_SIZE,
      max_lead_blocks: BENCH_MAX_LEAD_BLOCKS,
      p2pkh_version: 76,
      p2sh_version: 16,
      block_time_secs: 150.0,
    };

    IngestBenchContext {
      ctx,
      batch: state::InFlightBatch::new(1, BENCH_BATCH_SIZE, 1),
    }
  })
}

#[cfg(feature = "bench")]
fn minimal_block_bytes(nonce: u32) -> Vec<u8> {
  let mut buf = Vec::new();
  buf.extend_from_slice(&4_i32.to_le_bytes());
  buf.extend_from_slice(&[0xAA; 32]);
  buf.extend_from_slice(&[0xBB; 32]);
  buf.extend_from_slice(&1_700_000_000_u32.to_le_bytes());
  buf.extend_from_slice(&0x1A0F_FFFF_u32.to_le_bytes());
  buf.extend_from_slice(&nonce.to_le_bytes());
  buf.push(0x01);
  buf.extend_from_slice(&1_i32.to_le_bytes());
  buf.push(0x01);
  buf.extend_from_slice(&[0x00; 32]);
  buf.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
  buf.push(0x04);
  buf.extend_from_slice(&[nonce as u8, 0x02, 0x03, 0x04]);
  buf.extend_from_slice(&0xFFFF_FFFF_u32.to_le_bytes());
  buf.push(0x01);
  buf.extend_from_slice(&50_0000_0000_i64.to_le_bytes());
  buf.push(0x00);
  buf.extend_from_slice(&0_u32.to_le_bytes());
  buf
}

#[cfg(feature = "bench")]
/// Run one real ingest admission decision through the daemon-owned ingest seam.
pub fn bench_admit_batch() -> Result<usize, crate::error::DaemonError> {
  hyphend::bench::runtime().run_async(async {
    let ctx = &bench_context().ctx;
    let decision = admit::admit_batch(ctx, 1, BENCH_NETWORK_TIP).await?;
    Ok(match decision {
      admit::AdmissionDecision::Admit(batch) => {
        (batch.end_height - batch.start_height + 1) as usize
      }
      admit::AdmissionDecision::Idle | admit::AdmissionDecision::Throttle(_) => 0,
    })
  })
}

#[cfg(feature = "bench")]
/// Run one real ingest fetch through the daemon-owned ingest seam.
pub fn bench_fetch_batch() -> Result<usize, crate::error::DaemonError> {
  hyphend::bench::runtime().run_async(async {
    let shared = bench_context();
    let rows = fetch::fetch_batch(&shared.ctx, &shared.batch).await?;
    Ok(rows.len())
  })
}

#[cfg(feature = "bench")]
/// Run one real ingest commit through the daemon-owned ingest seam.
pub fn bench_commit_batch() -> Result<usize, crate::error::DaemonError> {
  hyphend::bench::runtime().run_async(async {
    let shared = bench_context();
    let rows = fetch::fetch_batch(&shared.ctx, &shared.batch).await?;
    commit::commit_batch(&shared.ctx, &shared.batch, &rows)?;
    Ok(rows.len())
  })
}

#[cfg(feature = "bench")]
/// Run one real ingest admit-fetch-commit turn through the daemon-owned ingest seam.
pub fn bench_turn_admit_fetch_commit() -> Result<usize, crate::error::DaemonError> {
  hyphend::bench::runtime().run_async(async {
    let shared = bench_context();
    let admit::AdmissionDecision::Admit(batch) =
      admit::admit_batch(&shared.ctx, 1, BENCH_NETWORK_TIP).await?
    else {
      return Err(crate::error::DaemonError::Config(
        "ingest benchmark expected admission without throttling".to_owned(),
      ));
    };
    let rows = fetch::fetch_batch(&shared.ctx, &batch).await?;
    commit::commit_batch(&shared.ctx, &batch, &rows)?;
    Ok(rows.len())
  })
}
