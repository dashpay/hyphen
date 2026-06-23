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
mod consensus;
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
use init::args::IndexTarget;
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
    StartupMode::DocsOnly => {
      rt.block_on(run_docs_only(&options))?;
      info!("docs-only request completed");
      drop(rt);
      lock.release()?;
      return Ok(());
    }
    StartupMode::RebuildIndex(_) => {
      rt.block_on(run_rebuild_index(&cfg, Arc::clone(&db), &options, &dir))?;
      info!("rebuild-index request completed");
      drop(db);
      drop(rt);
      lock.release()?;
      return Ok(());
    }
    StartupMode::VerifyIndex(_) => {
      rt.block_on(run_verify_index(&cfg, Arc::clone(&db), &options))?;
      drop(db);
      drop(rt);
      lock.release()?;
      return Ok(());
    }
    StartupMode::RewindAll { .. } => {
      rt.block_on(run_rewind_all(&cfg, Arc::clone(&db), &options, &dir))?;
      info!("rewind-all request completed");
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
    StartupMode::CompactStore(_) => {
      run_compact_store(&cfg, &options, &dir)?;
      info!("compact-store request completed");
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

async fn run_docs_only(options: &Options) -> Result<(), DaemonError> {
  let bind = options
    .api
    .bind
    .as_deref()
    .unwrap_or(rest::DEFAULT_API_BIND)
    .to_owned();
  let cors_origin = options.api.cors_origin.clone();
  let (shutdown_tx, shutdown_rx) = tokio::sync::watch::channel(false);
  let server = rest::serve_docs_only(&bind, shutdown_rx, cors_origin.as_deref());
  tokio::pin!(server);

  tokio::select! {
    result = &mut server => result,
    _ = tokio::signal::ctrl_c() => {
      info!("received SIGINT, shutting down docs-only server");
      let _ = shutdown_tx.send(true);
      tokio::time::timeout(
        Duration::from_secs(GRACEFUL_SHUTDOWN_TIMEOUT_SECS),
        &mut server,
      )
      .await
      .map_err(|_| DaemonError::Config("timed out shutting down docs-only server".to_owned()))?
    }
    _ = async {
      let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
        .map_err(|e| DaemonError::Io {
          context: "sigterm handler",
          source: e,
        })?;
      let _ = sigterm.recv().await;
      Result::<(), DaemonError>::Ok(())
    } => {
      info!("received SIGTERM, shutting down docs-only server");
      let _ = shutdown_tx.send(true);
      tokio::time::timeout(
        Duration::from_secs(GRACEFUL_SHUTDOWN_TIMEOUT_SECS),
        &mut server,
      )
      .await
      .map_err(|_| DaemonError::Config("timed out shutting down docs-only server".to_owned()))?
    }
  }
}

async fn run_rebuild_index(
  cfg: &config::Config,
  db: Arc<rocksdb::DB>,
  options: &Options,
  datadir: &std::path::Path,
) -> Result<(), DaemonError> {
  info!(url = %cfg.postgres.url, "connecting to postgres for rebuild-index");
  let pg_pools = sql::create_pools(&cfg.postgres).await?;
  sql::bootstrap::bootstrap(&pg_pools.write).await?;
  let secondary_stores = Arc::new(store::manager::SecondaryStoreSet::open(
    datadir,
    &cfg.chain,
    cfg.rocksdb.as_ref(),
  )?);

  match options.startup_mode {
    StartupMode::RebuildIndex(IndexTarget::All) => {
      warn!("running --rebuild-index all: resetting the entire PostgreSQL schema");
      let enabled_stages = index::enabled_stage_order(index::IndexStageFlags {
        address: options.indices.address,
        metrics: options.indices.metrics,
        validator: options.indices.validator,
      });
      let message = sql::verbs::reindex::force_reindex_all(
        &pg_pools.write,
        Arc::clone(&db),
        Some(Arc::clone(&secondary_stores)),
        &enabled_stages,
        cfg.chain.p2pkh_version,
        cfg.chain.p2sh_version,
        cfg.chain.block_time_secs,
      )
      .await?;
      info!(%message, "rebuild-index complete");
    }
    StartupMode::RebuildIndex(target) => {
      let stage = target
        .as_stage_name()
        .ok_or_else(|| DaemonError::Index("IndexTarget::All should not reach single-stage path".into()))?;
      warn!(stage, "running --rebuild-index");
      let message = sql::verbs::reindex::force_reindex_stage(
        &pg_pools.write,
        Arc::clone(&db),
        Some(Arc::clone(&secondary_stores)),
        stage,
        cfg.chain.p2pkh_version,
        cfg.chain.p2sh_version,
        cfg.chain.block_time_secs,
      )
      .await?;
      info!(%message, "rebuild-index complete");
    }
    _ => {}
  }
  Ok(())
}

async fn run_verify_index(
  cfg: &config::Config,
  db: Arc<rocksdb::DB>,
  options: &Options,
) -> Result<(), DaemonError> {
  info!(url = %cfg.postgres.url, "connecting to postgres for verify-index");
  let pg_pools = sql::create_pools(&cfg.postgres).await?;
  // No bootstrap() — verify is read-only and must not mutate the schema.

  let stage_filter = match options.startup_mode {
    StartupMode::VerifyIndex(IndexTarget::All) => None,
    StartupMode::VerifyIndex(target) => target.as_stage_name(),
    _ => None,
  };

  let report =
    sql::verbs::verify::verify_index(&pg_pools.write, &db, stage_filter).await?;

  if report.is_healthy() {
    info!("verify-index: all checks passed");
  } else {
    return Err(DaemonError::Index(
      "verify-index: health check failed, see log output above".to_owned(),
    ));
  }
  Ok(())
}

async fn run_rewind_all(
  cfg: &config::Config,
  db: Arc<rocksdb::DB>,
  options: &Options,
  datadir: &std::path::Path,
) -> Result<(), DaemonError> {
  let target_height = match options.startup_mode {
    StartupMode::RewindAll { target_height } => target_height,
    _ => return Ok(()),
  };

  info!(url = %cfg.postgres.url, "connecting to postgres for rewind-all");
  let pg_pools = sql::create_pools(&cfg.postgres).await?;
  sql::bootstrap::bootstrap(&pg_pools.write).await?;
  let secondary_stores = Arc::new(store::manager::SecondaryStoreSet::open(
    datadir,
    &cfg.chain,
    cfg.rocksdb.as_ref(),
  )?);

  let rocks_tip = db::get_tip_height(&db);
  let pg_height =
    sql::query::indexed_stage_height(&pg_pools.write, index::Stage::Block).await?;

  let effective_target = match target_height {
    Some(h) => h,
    None => {
      // Auto-detect: rewind to one below the lowest tip.
      let min_tip = match (pg_height >= 0, rocks_tip) {
        (true, Some(r)) => std::cmp::min(pg_height as u32, r),
        (true, None) => pg_height as u32,
        (false, Some(r)) => r,
        (false, None) => {
          return Err(DaemonError::Index("nothing to rewind".to_owned()));
        }
      };
      let auto = min_tip.saturating_sub(1);
      warn!(
        pg_height,
        rocks_tip = ?rocks_tip,
        auto_target = auto,
        "auto-detected rewind target from lowest database tip"
      );
      auto
    }
  };

  let max_allowed = match (pg_height >= 0, rocks_tip) {
    (true, Some(r)) => std::cmp::min(pg_height as u32, r),
    (true, None) => pg_height as u32,
    (false, Some(r)) => r,
    (false, None) => {
      return Err(DaemonError::Index("nothing to rewind".to_owned()));
    }
  };
  if effective_target >= max_allowed {
    return Err(DaemonError::Config(format!(
      "rewind target {effective_target} must be below min(pg={pg_height}, rocks={rocks_tip:?}) = {max_allowed}"
    )));
  }

  warn!(target_height = effective_target, "running --rewind-all");
  let message = sql::verbs::rewind::rewind_to_height(
    &pg_pools.write,
    &db,
    Some(Arc::clone(&secondary_stores)),
    effective_target,
    cfg.chain.p2pkh_version,
    cfg.chain.p2sh_version,
    cfg.chain.block_time_secs,
  )
  .await?;
  info!(%message, "rewind-all complete");

  // If any secondary store tip exceeds the effective target and the raw
  // blocks covering the rewind range are not fully present in RocksDB, the
  // store disconnects were skipped. Delete the store directories so they
  // auto-rebuild from RocksDB on next normal startup.
  let mut stores_need_reset = false;
  for id in store::StoreId::ALL {
    if let Ok(meta) = secondary_stores.meta(id) {
      if meta.tip_height.is_some_and(|tip| tip > effective_target) {
        stores_need_reset = true;
        break;
      }
    }
  }
  if stores_need_reset {
    // Must drop the open store handles before deleting the directories.
    drop(secondary_stores);
    for id in store::StoreId::ALL {
      store::reset_store_dir(datadir, id)?;
    }
    info!("secondary store directories reset; stores will auto-rebuild on next startup");
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
    _ => {}
  }
  Ok(())
}

fn run_compact_store(
  cfg: &config::Config,
  options: &Options,
  datadir: &std::path::Path,
) -> Result<(), DaemonError> {
  let StartupMode::CompactStore(target) = options.startup_mode else {
    return Ok(());
  };
  warn!(target = ?target, "running --compact-store");
  store::manager::compact_target(datadir, &cfg.chain, cfg.rocksdb.as_ref(), target)
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
