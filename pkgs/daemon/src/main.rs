//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Daemon binary for the Hyphen knowledge explorer.
//!
//! This binary owns ingestion, live data refresh, and the public REST and
//! Streaming HTTP surfaces. All chain storage is delegated to the `hyphen-db`
//! engine.

#![expect(
  dead_code,
  reason = "the daemon binary compiles helper modules whose entrypoints are exercised indirectly through runtime ownership and tests"
)]

mod api_metrics;
mod config;
mod consensus;
mod datadir;
mod error;
mod index;
mod ingest;
mod init;
mod ipc;
mod lifecycle;
mod live;
mod lockfile;
mod policy;
mod rest;
mod sources;
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
use tracing::info;
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

  // Before anything that takes a lock or migrates: `--show-layout` is a
  // read-only diagnostic and must work against a database a running daemon
  // already owns.
  if args.show_layout {
    return show_layout(&cfg);
  }

  let options = Options::resolve(&args, &cfg)?;
  options.log_capabilities();

  // The number, not a reassurance: "disk space OK" once survived the check it
  // was reporting on.
  let free_bytes = datadir::check_free_space(&dir)?;
  info!(free_bytes, "datadir free space");

  let lock = lockfile::acquire(&dir)?;
  info!("lockfile acquired");

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
    StartupMode::Serve => {}
  }

  info!(url = %cfg.core.url, "connecting to Core REST");
  let source: Arc<dyn RestSource> = Arc::new(sources::rest::CoreRestClient::new(&cfg.core)?);
  info!("REST client created");

  rt.block_on(run_async(cfg, options, Arc::clone(&source), dir))?;

  drop(source);
  drop(rt);
  info!("shutdown complete");
  lock.release()?;
  Ok(())
}

/// `--show-layout`: print the recorded storage layout beside the configured
/// one and exit. Read-only, lock-free, and migration-free by construction —
/// its whole purpose is to explain a startup refusal, so it must not be able
/// to hit the same refusal itself.
fn show_layout(cfg: &config::Config) -> Result<(), DaemonError> {
  let rt = tokio::runtime::Runtime::new().map_err(|e| DaemonError::Io {
    context: "tokio runtime",
    source: e,
  })?;
  let db_cfg = cfg.hydb_config();
  let rendered = rt
    .block_on(hyphen_db::show_layout(&db_cfg))
    .map_err(|e| DaemonError::storage("show layout", e))?;
  println!("{rendered}");
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
      .map_err(|_| DaemonError::ShutdownTimeout {
        subsystem: "docs-only server",
        secs: GRACEFUL_SHUTDOWN_TIMEOUT_SECS,
      })?
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
      .map_err(|_| DaemonError::ShutdownTimeout {
        subsystem: "docs-only server",
        secs: GRACEFUL_SHUTDOWN_TIMEOUT_SECS,
      })?
    }
  }
}

async fn run_async(
  cfg: config::Config,
  options: Options,
  source: Arc<dyn RestSource>,
  datadir: PathBuf,
) -> Result<(), DaemonError> {
  let mut sigterm = tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate())
    .map_err(|e| DaemonError::Io {
      context: "sigterm handler",
      source: e,
    })?;

  let mut runtime = tokio::select! {
    runtime = ServeRuntime::build(&cfg, &options, source, &datadir) => runtime?,
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
  // Subscribed *before* the heartbeat select below can itself request a
  // shutdown, so a subsystem that calls `lifecycle.interrupt_fatal()` (an
  // unrecoverable ingest failure, or a bounded park timing out) wakes this
  // loop up on its own. Without this, the only way out of this select was an
  // OS signal — a subsystem-initiated shutdown would quiesce every worker
  // and then leave this loop waiting for a SIGINT/SIGTERM that nobody sends,
  // which is a live, unresponsive process indistinguishable from a hang.
  let mut internal_rx = lifecycle.subscribe();

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
    changed = internal_rx.changed() => {
      if changed.is_ok() && *internal_rx.borrow_and_update() {
        info!("a subsystem requested shutdown on its own; shutting down");
      }
      false
    }
  };
  let graceful_deadline = graceful_shutdown_started
    .then(|| tokio::time::Instant::now() + Duration::from_secs(GRACEFUL_SHUTDOWN_TIMEOUT_SECS));

  runtime.stop_all(&mut sigterm, graceful_deadline).await?;

  // A subsystem-initiated shutdown (an unrecoverable ingest failure, or a
  // bounded park that timed out with no operator intervention) must end the
  // process with a non-zero exit code: that is the only liveness signal a
  // container supervisor without a healthcheck has, and returning `Ok(())`
  // here — as every operator-requested shutdown does — would report success
  // for exactly the condition this exists to surface.
  if lifecycle.is_fatal() {
    return Err(DaemonError::Supervisor {
      subsystem: "ingest",
      failure: crate::error::SupervisorFailure::Unavailable(
        "shut down on its own (unrecoverable failure, or a bounded park that timed out)",
      ),
    });
  }

  Ok(())
}

fn main() {
  if let Err(e) = run() {
    eprintln!("error: {e}");
    std::process::exit(1);
  }
}
