//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Shared API server metrics.

use std::sync::atomic::{AtomicU64, Ordering};

/// Shared counters for REST server request metrics.
#[derive(Debug, Default)]
pub struct ApiServerMetrics {
  pub total_requests: AtomicU64,
  pub total_errors: AtomicU64,
  /// EWMA latency in microseconds (fixed-point, alpha ~ 1/20).
  pub ewma_latency_us: AtomicU64,
}

impl ApiServerMetrics {
  /// Update the EWMA with a new observation.
  pub fn record_latency(&self, elapsed_us: u64) {
    let _ = self
      .ewma_latency_us
      .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
        if old == 0 {
          Some(elapsed_us)
        } else {
          Some(old + elapsed_us / 20 - old / 20)
        }
      });
  }

  /// Produce a point-in-time snapshot of the server metrics.
  pub fn snapshot(&self) -> hyphend::wire::msg::ApiServerStats {
    let reqs = self.total_requests.load(Ordering::Relaxed);
    let errs = self.total_errors.load(Ordering::Relaxed);
    let ewma = self.ewma_latency_us.load(Ordering::Relaxed);
    let avg = if reqs > 0 {
      Some(ewma as f64 / 1000.0)
    } else {
      None
    };
    hyphend::wire::msg::ApiServerStats {
      total_requests: reqs,
      total_errors: errs,
      avg_latency_ms: avg,
    }
  }
}
