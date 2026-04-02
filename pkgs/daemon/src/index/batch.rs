//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Elastic batch-size controller for index stages.

use std::collections::{HashMap, VecDeque};
use std::sync::{Arc, Mutex};

pub const DEFAULT_INGESTION_BATCH_SIZE: u32 = 100;
pub const DEFAULT_PREFETCH_BLOCKS: u32 = 64;
pub const DEFAULT_COMMIT_BATCH_BLOCKS: u32 = 2000;
pub const DEFAULT_MIN_BATCH_SIZE: u32 = 100;
pub const DEFAULT_MAX_BATCH_SIZE: u32 = 50_000;
pub const DEFAULT_TARGET_BATCH_MS: u64 = 1_000;

/// Adjusts the batch size after each completed batch to keep
/// processing time near `target_ms`.
#[derive(Debug)]
pub struct BatchSizer {
  pub current: u32,
  min: u32,
  max: u32,
  target_ms: u64,
  /// Duration of the last committed batch (milliseconds).
  pub last_batch_ms: Option<u64>,
  /// Last 3 size adjustments: (elapsed_ms, prev_size, new_size).
  pub history: VecDeque<(u64, u32, u32)>,
}

impl BatchSizer {
  pub fn new(initial: u32, min: u32, max: u32, target_ms: u64) -> Self {
    Self {
      current: initial.clamp(min, max),
      min,
      max,
      target_ms,
      last_batch_ms: None,
      history: VecDeque::new(),
    }
  }

  /// Record elapsed time for the last batch and adjust size.
  pub fn adjust(&mut self, elapsed_ms: u64, name: &str) {
    let prev = self.current;
    self.current = if elapsed_ms == 0 || elapsed_ms < self.target_ms / 2 {
      // Well under target: double.
      self.current.saturating_mul(2).min(self.max)
    } else if elapsed_ms > self.target_ms * 2 {
      // Well over target: halve.
      (self.current / 2).max(self.min)
    } else {
      // Within ±2× window: nudge proportionally.
      let ratio = self.target_ms as f64 / elapsed_ms.max(1) as f64;
      ((self.current as f64 * ratio) as u32).clamp(self.min, self.max)
    };

    self.last_batch_ms = Some(elapsed_ms);

    if self.current != prev {
      if self.history.len() >= 3 {
        self.history.pop_front();
      }
      self.history.push_back((elapsed_ms, prev, self.current));
      tracing::debug!(
        name,
        batch_size = self.current,
        prev_batch_size = prev,
        elapsed_ms,
        target_ms = self.target_ms,
        "batch sizer adjusted"
      );
    } else {
      tracing::debug!(
        name,
        batch_size = self.current,
        elapsed_ms,
        target_ms = self.target_ms,
        "batch sizer stable"
      );
    }
  }
}

/// Point-in-time snapshot of a batch sizer for IPC broadcast.
#[derive(Debug, Clone)]
pub struct BatchSizerSnapshot {
  pub stage: String,
  pub current: u32,
  pub last_batch_ms: Option<u64>,
  /// Last 3 size adjustments: (elapsed_ms, prev_size, new_size).
  pub history: Vec<(u64, u32, u32)>,
}

/// Shared registry of batch sizers keyed by stage name.
#[derive(Debug, Clone, Default)]
pub struct BatchSizerRegistry {
  sizers: Arc<Mutex<HashMap<String, Arc<Mutex<BatchSizer>>>>>,
}

impl BatchSizerRegistry {
  pub fn new() -> Self {
    Self::default()
  }

  /// Register a sizer for a stage.
  pub fn register(&self, stage: &str, sizer: Arc<Mutex<BatchSizer>>) {
    if let Ok(mut map) = self.sizers.lock() {
      map.insert(stage.to_string(), sizer);
    }
  }

  /// Snapshot all sizers for IPC broadcast.
  pub fn snapshot(&self) -> Vec<BatchSizerSnapshot> {
    let map = match self.sizers.lock() {
      Ok(m) => m,
      Err(_) => return Vec::new(),
    };
    map
      .iter()
      .filter_map(|(stage, sizer)| {
        sizer.lock().ok().map(|s| BatchSizerSnapshot {
          stage: stage.clone(),
          current: s.current,
          last_batch_ms: s.last_batch_ms,
          history: s.history.iter().copied().collect(),
        })
      })
      .collect()
  }
}
