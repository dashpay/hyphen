//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Logging initialization.

use std::path::Path;
use std::sync::OnceLock;

use crate::error::DaemonError;

static FILE_LOG_GUARD: OnceLock<tracing_appender::non_blocking::WorkerGuard> = OnceLock::new();

/// Initialize file and optional console logging.
///
/// # Errors
///
/// Returns `DaemonError::Io` when the log directory or log file cannot be
/// created.
pub fn init_logs(datadir: &Path, loglevel: u8, logconsole: bool) -> Result<(), DaemonError> {
  use tracing::level_filters::LevelFilter;
  use tracing_subscriber::{fmt, layer::SubscriberExt, util::SubscriberInitExt, Layer};

  let ts = std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .map(|d| d.as_secs())
    .unwrap_or(0);
  let logs_dir = datadir.join("logs");
  std::fs::create_dir_all(&logs_dir).map_err(|e| DaemonError::Io {
    context: "create logs directory",
    source: e,
  })?;
  let log_path = logs_dir.join(format!("{ts}.log"));
  let log_file = std::fs::File::create(&log_path).map_err(|e| DaemonError::Io {
    context: "create log file",
    source: e,
  })?;
  let filter = match loglevel {
    0 => LevelFilter::ERROR,
    1 => LevelFilter::WARN,
    2 => LevelFilter::INFO,
    _ => LevelFilter::DEBUG,
  };
  let (file_writer, file_guard) = tracing_appender::non_blocking::NonBlockingBuilder::default()
    .lossy(false)
    .buffered_lines_limit(16_384)
    .finish(log_file);
  let _ = FILE_LOG_GUARD.set(file_guard);

  let file_layer = fmt::layer()
    .with_ansi(false)
    .with_writer(file_writer)
    .with_filter(filter);

  let console_layer = logconsole.then(|| fmt::layer().with_filter(filter));

  tracing_subscriber::registry()
    .with(file_layer)
    .with(console_layer)
    .init();

  eprintln!("log file: {}", log_path.display());

  const MAX_LOG_AGE: std::time::Duration = std::time::Duration::from_secs(7 * 24 * 3600);
  if let Ok(entries) = std::fs::read_dir(&logs_dir) {
    for entry in entries.flatten() {
      let path = entry.path();
      if path.extension().is_some_and(|e| e == "log") && path != log_path {
        if let Ok(meta) = path.metadata() {
          if let Ok(modified) = meta.modified() {
            if modified.elapsed().is_ok_and(|age| age > MAX_LOG_AGE) {
              let _ = std::fs::remove_file(&path);
            }
          }
        }
      }
    }
  }

  Ok(())
}
