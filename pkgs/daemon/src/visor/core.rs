//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL/RocksDB-backed core supervisor.

use std::collections::{BTreeSet, HashMap};
use std::sync::atomic::AtomicBool;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use crate::sql::SqlPool;
use async_trait::async_trait;
use rocksdb::DB;

use crate::config::PerformanceConfig;
use crate::error::DaemonError;
use crate::index;
use crate::index::batch::{
  BatchSizer, BatchSizerRegistry, DEFAULT_COMMIT_BATCH_BLOCKS, DEFAULT_INGESTION_BATCH_SIZE,
  DEFAULT_MAX_BATCH_SIZE, DEFAULT_MIN_BATCH_SIZE, DEFAULT_PREFETCH_BLOCKS, DEFAULT_TARGET_BATCH_MS,
};
use crate::index::spent::cache::SpentQueryCache;
use crate::ingest::{self, IngestRuntime};
use crate::init::Options;
use crate::lifecycle::Lifecycle;
use crate::sources::RestSource;
use crate::store::manager::SecondaryStoreSet;

use super::{
  await_supervisable_task, spawn_supervised_task, Fenceable, InterruptGate, SupervisedHandle, Visor,
};

/// Inputs needed to construct the core supervisor.
pub(crate) struct CoreVisorConfig {
  pub db: Arc<DB>,
  pub secondary_stores: Option<Arc<SecondaryStoreSet>>,
  pub pool: SqlPool,
  pub source: Arc<dyn RestSource>,
  pub event_tx: tokio::sync::broadcast::Sender<hyphend::wire::msg::DaemonMessage>,
  pub lifecycle: Arc<Lifecycle>,
  pub spent_cache: Arc<SpentQueryCache>,
  pub enabled_stages: Arc<Vec<index::Stage>>,
  pub ingestion_batch_size: u32,
  pub max_ingestion_lead_blocks: u32,
  pub idle_timeout: Duration,
  pub prefetch_blocks: u32,
  pub initial_batch: u32,
  pub min_batch: u32,
  pub max_batch: u32,
  pub target_batch_ms: u64,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub block_time_secs: f64,
}

impl std::fmt::Debug for CoreVisorConfig {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("CoreVisorConfig").finish_non_exhaustive()
  }
}

/// Inputs needed to resolve one core supervisor config from startup options.
pub(crate) struct CoreVisorOptionsInput {
  pub db: Arc<DB>,
  pub secondary_stores: Option<Arc<SecondaryStoreSet>>,
  pub pool: SqlPool,
  pub source: Arc<dyn RestSource>,
  pub event_tx: tokio::sync::broadcast::Sender<hyphend::wire::msg::DaemonMessage>,
  pub lifecycle: Arc<Lifecycle>,
  pub spent_cache: Arc<SpentQueryCache>,
  pub idle_timeout: Duration,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub block_time_secs: f64,
}

impl std::fmt::Debug for CoreVisorOptionsInput {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("CoreVisorOptionsInput")
      .finish_non_exhaustive()
  }
}

/// Shared runtime handles exported by the core supervisor.
#[derive(Clone)]
pub(crate) struct CoreHandles {
  pub chain_state_lock: Arc<tokio::sync::RwLock<()>>,
  pub ingest_runtime: Arc<IngestRuntime>,
  pub block_worker: Arc<index::BlockIndexerWorker>,
  pub control_plane_pause_active: Arc<AtomicBool>,
  pub operator_pause_requested: Arc<AtomicBool>,
  scheduler_state: Arc<std::sync::Mutex<index::runner::IndexSchedulerState>>,
  pub batch_registry: BatchSizerRegistry,
  pub controls: Arc<tokio::sync::Mutex<HashMap<index::Stage, index::IndexerControl>>>,
  pub pausing_tx: tokio::sync::watch::Sender<BTreeSet<String>>,
  pub pausing_rx: tokio::sync::watch::Receiver<BTreeSet<String>>,
  pub suspend_tx: tokio::sync::watch::Sender<BTreeSet<String>>,
  pub suspend_rx: tokio::sync::watch::Receiver<BTreeSet<String>>,
  pub revalidate_schema: Arc<AtomicBool>,
}

impl std::fmt::Debug for CoreHandles {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("CoreHandles").finish_non_exhaustive()
  }
}

/// Owned ingest/index runtime supervisor.
pub(crate) struct CoreVisor {
  cfg: CoreVisorConfig,
  handles: CoreHandles,
  block_batch_sizer: Arc<Mutex<BatchSizer>>,
  tx_batch_sizer: Arc<Mutex<BatchSizer>>,
  spent_batch_sizer: Arc<Mutex<BatchSizer>>,
  ingestion_task: Option<tokio::task::JoinHandle<()>>,
  ingestion_worker: Option<SupervisedHandle<crate::ingest::IngestWorker>>,
  block_task: Option<tokio::task::JoinHandle<()>>,
  block_task_worker: Option<SupervisedHandle<index::BlockIndexerWorker>>,
}

impl std::fmt::Debug for CoreVisor {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("CoreVisor").finish_non_exhaustive()
  }
}

impl CoreVisor {
  /// Build one resolved core supervisor config from validated startup options.
  pub(crate) fn config_from_options(
    input: CoreVisorOptionsInput,
    options: &Options,
    perf: Option<&PerformanceConfig>,
  ) -> CoreVisorConfig {
    let enabled_stages = Arc::new(index::enabled_stage_order(index::IndexStageFlags {
      address: options.indices.address,
      metrics: options.indices.metrics,
      validator: options.indices.validator,
    }));
    CoreVisorConfig {
      db: input.db,
      secondary_stores: input.secondary_stores,
      pool: input.pool,
      source: input.source,
      event_tx: input.event_tx,
      lifecycle: input.lifecycle,
      spent_cache: input.spent_cache,
      enabled_stages,
      ingestion_batch_size: perf.map_or(DEFAULT_INGESTION_BATCH_SIZE, |perf| {
        perf.ingestion_batch_size
      }),
      max_ingestion_lead_blocks: perf
        .map_or(crate::ingest::DEFAULT_MAX_INGEST_LEAD_BLOCKS, |perf| {
          perf.max_ingestion_lead_blocks
        }),
      idle_timeout: input.idle_timeout,
      prefetch_blocks: perf.map_or(DEFAULT_PREFETCH_BLOCKS, |perf| perf.prefetch_blocks),
      initial_batch: perf.map_or(DEFAULT_COMMIT_BATCH_BLOCKS, |perf| perf.commit_batch_blocks),
      min_batch: perf.map_or(DEFAULT_MIN_BATCH_SIZE, |perf| perf.min_batch_size),
      max_batch: perf.map_or(DEFAULT_MAX_BATCH_SIZE, |perf| perf.max_batch_size),
      target_batch_ms: perf.map_or(DEFAULT_TARGET_BATCH_MS, |perf| perf.target_batch_ms),
      p2pkh_version: input.p2pkh_version,
      p2sh_version: input.p2sh_version,
      block_time_secs: input.block_time_secs,
    }
  }

  pub(crate) fn new(cfg: CoreVisorConfig) -> Self {
    let chain_state_lock = Arc::new(tokio::sync::RwLock::new(()));
    let ingest_runtime = Arc::new(IngestRuntime::default());
    let control_plane_pause_active = Arc::new(AtomicBool::new(false));
    let operator_pause_requested = Arc::new(AtomicBool::new(false));
    let scheduler_state = Arc::new(std::sync::Mutex::new(
      index::runner::IndexSchedulerState::default(),
    ));
    let block_worker = Arc::new(index::BlockIndexerWorker::new(
      Arc::clone(&cfg.db),
      Arc::clone(&ingest_runtime),
      Arc::clone(&cfg.lifecycle),
      Arc::clone(&scheduler_state),
      Arc::clone(&control_plane_pause_active),
    ));
    let batch_registry = BatchSizerRegistry::new();
    let block_batch_sizer = Arc::new(Mutex::new(BatchSizer::new(
      cfg.initial_batch,
      cfg.min_batch,
      cfg.max_batch,
      cfg.target_batch_ms,
    )));
    let tx_batch_sizer = Arc::new(Mutex::new(BatchSizer::new(
      cfg.initial_batch,
      cfg.min_batch,
      cfg.max_batch,
      cfg.target_batch_ms,
    )));
    let spent_batch_sizer = Arc::new(Mutex::new(BatchSizer::new(
      cfg.initial_batch,
      cfg.min_batch,
      cfg.max_batch,
      cfg.target_batch_ms,
    )));
    batch_registry.register(index::Stage::Block.name(), Arc::clone(&block_batch_sizer));
    batch_registry.register(
      index::Stage::Transaction.name(),
      Arc::clone(&tx_batch_sizer),
    );
    batch_registry.register(index::Stage::Spent.name(), Arc::clone(&spent_batch_sizer));
    let (pausing_tx, pausing_rx) = tokio::sync::watch::channel(BTreeSet::new());
    let (suspend_tx, suspend_rx) = tokio::sync::watch::channel(BTreeSet::new());

    Self {
      cfg,
      handles: CoreHandles {
        chain_state_lock,
        ingest_runtime,
        block_worker,
        control_plane_pause_active,
        operator_pause_requested,
        scheduler_state,
        batch_registry,
        controls: Arc::new(tokio::sync::Mutex::new(HashMap::new())),
        pausing_tx,
        pausing_rx,
        suspend_tx,
        suspend_rx,
        revalidate_schema: Arc::new(AtomicBool::new(false)),
      },
      block_batch_sizer,
      tx_batch_sizer,
      spent_batch_sizer,
      ingestion_task: None,
      ingestion_worker: None,
      block_task: None,
      block_task_worker: None,
    }
  }

  pub(crate) fn handles(&self) -> CoreHandles {
    self.handles.clone()
  }
}

impl InterruptGate for CoreVisor {
  fn interrupt(&self) {
    self.cfg.lifecycle.interrupt();
  }

  fn interrupt_point(&self) -> bool {
    self.cfg.lifecycle.interruption_point()
  }
}

#[async_trait]
impl Visor for CoreVisor {
  async fn start(&mut self) -> Result<(), DaemonError> {
    if self.ingestion_task.is_some() || self.block_task.is_some() {
      return Ok(());
    }
    if !self.interrupt_point() {
      return Ok(());
    }

    let ingest_spec = ingest::ingestion_supervised_task(ingest::IngestCtx {
      db: Arc::clone(&self.cfg.db),
      secondary_stores: self.cfg.secondary_stores.clone(),
      pool: self.cfg.pool.clone(),
      source: Arc::clone(&self.cfg.source),
      lifecycle: Arc::clone(&self.cfg.lifecycle),
      runtime: Arc::clone(&self.handles.ingest_runtime),
      block_worker: Arc::clone(&self.handles.block_worker),
      chain_state_lock: Arc::clone(&self.handles.chain_state_lock),
      batch_size: self.cfg.ingestion_batch_size,
      max_lead_blocks: self.cfg.max_ingestion_lead_blocks,
      p2pkh_version: self.cfg.p2pkh_version,
      p2sh_version: self.cfg.p2sh_version,
      block_time_secs: self.cfg.block_time_secs,
    });
    let ingest_handle = SupervisedHandle::new(ingest_spec.initial.worker.clone());
    self.ingestion_worker = Some(ingest_handle.clone());
    self.ingestion_task = Some(spawn_supervised_task(
      "core",
      ingest_spec.name,
      Arc::clone(&self.cfg.lifecycle),
      ingest_handle,
      ingest_spec.initial,
      move || (ingest_spec.rebuild)(),
    ));

    let (block_control, block_handle) = index::indexer_channel(self.cfg.idle_timeout);
    self
      .handles
      .controls
      .lock()
      .await
      .insert(index::Stage::Block, block_control);

    let block_spec = index::block_indexer_supervised_task(
      index::IndexerLoopCtx {
        db: Arc::clone(&self.cfg.db),
        secondary_stores: self.cfg.secondary_stores.clone(),
        pool: self.cfg.pool.clone(),
        block_worker: Arc::clone(&self.handles.block_worker),
        event_tx: self.cfg.event_tx.clone(),
        lifecycle: Arc::clone(&self.cfg.lifecycle),
        enabled_stages: Arc::clone(&self.cfg.enabled_stages),
        chain_state_lock: Arc::clone(&self.handles.chain_state_lock),
        scheduler_state: Arc::clone(&self.handles.scheduler_state),
        spent_cache: Arc::clone(&self.cfg.spent_cache),
        history_partition_cache: Arc::new(std::sync::Mutex::new(None)),
        canonical_partition_cache: Arc::new(std::sync::Mutex::new(None)),
        tx_batch_sizer: Arc::clone(&self.tx_batch_sizer),
        spent_batch_sizer: Arc::clone(&self.spent_batch_sizer),
        operator_pause_requested: Arc::clone(&self.handles.operator_pause_requested),
        p2pkh_version: self.cfg.p2pkh_version,
        p2sh_version: self.cfg.p2sh_version,
        block_time_secs: self.cfg.block_time_secs,
      },
      Arc::clone(&self.block_batch_sizer),
      self.cfg.prefetch_blocks,
      block_handle,
    );
    let block_handle = SupervisedHandle::new(block_spec.initial.worker.clone());
    self.block_task_worker = Some(block_handle.clone());
    self.block_task = Some(spawn_supervised_task(
      "core",
      block_spec.name,
      Arc::clone(&self.cfg.lifecycle),
      block_handle,
      block_spec.initial,
      move || (block_spec.rebuild)(),
    ));

    Ok(())
  }

  async fn stop(
    &mut self,
    sigterm: &mut tokio::signal::unix::Signal,
    graceful_deadline: Option<tokio::time::Instant>,
  ) -> Result<(), DaemonError> {
    if let (Some(task), Some(worker)) = (self.ingestion_task.take(), self.ingestion_worker.take()) {
      await_supervisable_task(
        "core",
        "ingestion",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    if let (Some(task), Some(worker)) = (self.block_task.take(), self.block_task_worker.take()) {
      await_supervisable_task(
        "core",
        "block indexer",
        task,
        &worker,
        &self.cfg.lifecycle,
        sigterm,
        graceful_deadline,
      )
      .await;
    }
    self.handles.block_worker.fence(None);
    self.handles.ingest_runtime.set_paused(false);
    self.handles.controls.lock().await.clear();
    let _ = self.handles.pausing_tx.send(BTreeSet::new());
    let _ = self.handles.suspend_tx.send(BTreeSet::new());
    self
      .handles
      .revalidate_schema
      .store(false, std::sync::atomic::Ordering::Relaxed);
    Ok(())
  }
}
