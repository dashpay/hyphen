//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Daemon binary for the Hyphen knowledge explorer.
//!
//! This binary owns ingestion, indexing, live data refresh, IPC snapshots, and
//! the public REST and Streaming HTTP surfaces.

#![expect(
  dead_code,
  reason = "the daemon binary compiles helper modules whose entrypoints are exercised indirectly through runtime ownership and tests"
)]

mod api_metrics;
#[cfg(test)]
mod architecture;
mod cache;
mod config;
mod datadir;
mod db;
mod error;
mod index;
mod ingest;
mod init;
mod ipc;
mod lifecycle;
mod live;
mod lockfile;
mod policy;
mod readiness;
mod recovery;
mod rest;
mod sources;
mod sql;
mod store;
#[cfg(any(test, feature = "tests"))]
mod tests;
mod tx;
mod utils;
mod visor;
mod worker;

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use error::DaemonError;
use init::{Args, Options, StartupMode};
use sources::RestSource;
use tracing::{info, warn};
use visor::runtime::ServeRuntime;

const GRACEFUL_SHUTDOWN_TIMEOUT_SECS: u64 = 5;

fn run() -> Result<(), DaemonError> {
  let args = Args::parse_args();
  let dir = datadir::resolve(&args.datadir)?;
  datadir::ensure_exists(&dir)?;
  let open_file_limit = init::raise_open_file_limit();

  init::init_logs(&dir, args.loglevel, args.logconsole)?;

  info!(version = env!("CARGO_PKG_VERSION"), "hyphend starting");
  info!(path = %dir.display(), "datadir");
  info!(open_file_limit, "process file-descriptor limit");

  let cfg = init::load_config(&dir)?;
  let options = Options::resolve(&args, &cfg)?;

  datadir::check_free_space(&dir)?;
  info!("disk space OK");

  let lock = lockfile::acquire(&dir)?;
  info!("lockfile acquired");

  let db = Arc::new(db::open(&dir, cfg.rocksdb.as_ref())?);
  info!("RocksDB opened");
  db::verify_fixed_constants(&db, &cfg.chain)?;
  info!("RocksDB persisted constants verified");

  let rt = tokio::runtime::Runtime::new().map_err(|e| DaemonError::Io {
    context: "tokio runtime",
    source: e,
  })?;

  match options.startup_mode {
    StartupMode::ReindexAll => {
      rt.block_on(run_reindex(&cfg, &db, &options))?;
      info!("reindex request completed");
      drop(db);
      drop(rt);
      lock.release()?;
      return Ok(());
    }
    StartupMode::Reindex(_) => {
      rt.block_on(run_reindex(&cfg, &db, &options))?;
      info!("reindex request completed");
      drop(db);
      drop(rt);
      lock.release()?;
      return Ok(());
    }
    StartupMode::Serve => {}
    StartupMode::RebuildStore(_) | StartupMode::VerifyStore(_) => {
      run_store_maintenance(&cfg, &db, &options, &dir)?;
      info!("store maintenance request completed");
      drop(db);
      drop(rt);
      lock.release()?;
      return Ok(());
    }
  }

  info!(url = %cfg.core.url, "connecting to Core REST");
  let source: Arc<dyn RestSource> = Arc::new(sources::rest::CoreRestClient::new(&cfg.core)?);
  info!("REST client created");

  rt.block_on(run_async(
    cfg,
    options,
    Arc::clone(&db),
    Arc::clone(&source),
    dir,
  ))?;

  if let Err(e) = db.flush_wal(true) {
    warn!("failed to flush RocksDB WAL during shutdown: {e}");
  }
  if let Err(e) = db.flush() {
    warn!("failed to flush RocksDB memtables during shutdown: {e}");
  }
  drop(source);
  drop(db);
  drop(rt);
  info!("shutdown complete");
  lock.release()?;
  Ok(())
}

async fn run_reindex(
  cfg: &config::Config,
  db: &rocksdb::DB,
  options: &Options,
) -> Result<(), DaemonError> {
  info!(url = %cfg.postgres.url, "connecting to postgres for reindex");
  let pg_pools = sql::create_pools(&cfg.postgres).await?;
  sql::bootstrap::bootstrap(&pg_pools.write).await?;

  match options.startup_mode {
    StartupMode::ReindexAll => {
      warn!("running --reindex-all: resetting the entire PostgreSQL schema");
      sql::bootstrap::reset(&pg_pools.write).await?;
    }
    StartupMode::Reindex(target) => {
      let stage = target.as_stage_name();
      warn!(stage, "running --reindex");
      let message = sql::verbs::reindex::force_reindex_stage(&pg_pools.write, db, stage).await?;
      info!(%message, "reindex complete");
    }
    StartupMode::Serve => {}
    StartupMode::RebuildStore(_) | StartupMode::VerifyStore(_) => {}
  }
  Ok(())
}

fn run_store_maintenance(
  cfg: &config::Config,
  db: &rocksdb::DB,
  options: &Options,
  datadir: &std::path::Path,
) -> Result<(), DaemonError> {
  match options.startup_mode {
    StartupMode::RebuildStore(target) => {
      warn!(target = ?target, "running --rebuild-store");
      let _ =
        store::manager::rebuild_target(db, datadir, &cfg.chain, cfg.rocksdb.as_ref(), target)?;
    }
    StartupMode::VerifyStore(target) => {
      warn!(target = ?target, "running --verify-store");
      store::manager::verify_target(db, datadir, &cfg.chain, cfg.rocksdb.as_ref(), target)?;
    }
    StartupMode::Serve | StartupMode::Reindex(_) | StartupMode::ReindexAll => {}
  }
  Ok(())
}

async fn run_async(
  cfg: config::Config,
  options: Options,
  db: Arc<rocksdb::DB>,
  source: Arc<dyn RestSource>,
  datadir: PathBuf,
) -> Result<(), DaemonError> {
  let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    .map_err(|e| DaemonError::Io {
      context: "sigterm handler",
      source: e,
    })?;

  let mut runtime = tokio::select! {
    runtime = ServeRuntime::build(&cfg, &options, db, source, &datadir) => runtime?,
    _ = tokio::signal::ctrl_c() => {
      info!("received SIGINT during startup, aborting before subsystems started");
      return Ok(());
    }
    _ = sigterm.recv() => {
      info!("received SIGTERM during startup, aborting before subsystems started");
      return Ok(());
    }
  };

  let startup_graceful = tokio::select! {
    start = runtime.start_all() => {
      start?;
      None
    }
    _ = tokio::signal::ctrl_c() => {
      info!("received SIGINT during startup, shutting down partially started subsystems");
      runtime.lifecycle().interrupt();
      Some(true)
    }
    _ = sigterm.recv() => {
      info!("received SIGTERM during startup, shutting down partially started subsystems");
      runtime.lifecycle().stop();
      Some(false)
    }
  };

  if let Some(graceful_shutdown_started) = startup_graceful {
    let graceful_deadline = graceful_shutdown_started
      .then(|| tokio::time::Instant::now() + Duration::from_secs(GRACEFUL_SHUTDOWN_TIMEOUT_SECS));
    runtime.stop_all(&mut sigterm, graceful_deadline).await?;
    return Ok(());
  }

  let lifecycle = runtime.lifecycle();

  // ── Heartbeat / signal loop ───────────────────────────────────────────
  info!("all subsystems started");

  let graceful_shutdown_started = tokio::select! {
    _ = tokio::signal::ctrl_c() => {
      info!("received SIGINT, shutting down");
      lifecycle.interrupt();
      true
    }
    _ = sigterm.recv() => {
      info!("received SIGTERM, shutting down");
      lifecycle.stop();
      false
    }
  };
  let graceful_deadline = graceful_shutdown_started
    .then(|| tokio::time::Instant::now() + Duration::from_secs(GRACEFUL_SHUTDOWN_TIMEOUT_SECS));

  runtime.stop_all(&mut sigterm, graceful_deadline).await?;

  Ok(())
}

fn main() {
  if let Err(e) = run() {
    eprintln!("error: {e}");
    std::process::exit(1);
  }
}
