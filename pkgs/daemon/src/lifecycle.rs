//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Graceful shutdown coordination via atomic flags and a watch channel.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Shared lifecycle handle for coordinated shutdown.
///
/// Loop pattern:
/// ```ignore
/// while lifecycle.running() {
/// // work
/// lifecycle.sleep(Duration::from_secs(5)).await;
/// }
/// ```
pub struct Lifecycle {
  interrupted: AtomicBool,
  terminated: AtomicBool,
  tx: tokio::sync::watch::Sender<bool>,
}

impl std::fmt::Debug for Lifecycle {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("Lifecycle")
      .field("interrupted", &self.interrupted.load(Ordering::Relaxed))
      .field("terminated", &self.terminated.load(Ordering::Relaxed))
      .finish()
  }
}

impl Lifecycle {
  /// Create a new lifecycle handle.
  pub fn start() -> Arc<Self> {
    let (tx, _) = tokio::sync::watch::channel(false);
    Arc::new(Self {
      interrupted: AtomicBool::new(false),
      terminated: AtomicBool::new(false),
      tx,
    })
  }

  /// Signal loops to finish current work then exit (SIGINT).
  pub fn interrupt(&self) {
    self.interrupted.store(true, Ordering::SeqCst);
    let _ = self.tx.send(true);
  }

  /// Signal loops to abandon work immediately (SIGTERM).
  pub fn stop(&self) {
    self.terminated.store(true, Ordering::SeqCst);
    self.interrupt();
  }

  /// `true` after a graceful interrupt has been requested.
  #[inline]
  pub fn interrupted(&self) -> bool {
    self.interrupted.load(Ordering::Relaxed)
  }

  /// `true` after an immediate stop has been requested.
  #[inline]
  pub fn terminated(&self) -> bool {
    self.terminated.load(Ordering::Relaxed)
  }

  /// `true` when callers may start new free-running work.
  #[inline]
  pub fn interruption_point(&self) -> bool {
    !self.interrupted()
  }

  /// `true` while the daemon should keep running.
  #[inline]
  pub fn running(&self) -> bool {
    self.interruption_point()
  }

  /// Sleep for `duration`, returning early on shutdown signal.
  pub async fn sleep(&self, duration: Duration) {
    if !self.running() {
      return;
    }
    let mut rx = self.tx.subscribe();
    if *rx.borrow_and_update() {
      return;
    }
    tokio::select! {
      _ = tokio::time::sleep(duration) => {}
      _ = rx.changed() => {}
    }
  }

  /// Subscribe to the shutdown channel for `select!` usage.
  pub fn subscribe(&self) -> tokio::sync::watch::Receiver<bool> {
    self.tx.subscribe()
  }
}
