//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Application event types.

use crossterm::event::Event as TermEvent;
use hyphend::wire::msg::DaemonMessage;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BrowseIndex {
  Blocks,
  Transactions,
  Validators,
  Metrics,
}

impl BrowseIndex {
  /// REST API path segment for each browseable index.
  pub fn endpoint(self) -> &'static str {
    match self {
      Self::Blocks => "blocks",
      Self::Transactions => "txs",
      Self::Validators => "validators",
      Self::Metrics => "metrics/chart",
    }
  }

  /// Human-readable label shown in the browse UI.
  pub fn label(self) -> &'static str {
    match self {
      Self::Blocks => "Blocks",
      Self::Transactions => "Transactions",
      Self::Validators => "Validators",
      Self::Metrics => "Metrics",
    }
  }
}

/// One request sent from the UI thread to the REST worker.
#[derive(Debug)]
pub enum RestRequest {
  Browse {
    index: BrowseIndex,
    limit: u32,
    cursor: Option<String>,
  },
}

/// One response sent back from the REST worker to the UI thread.
#[derive(Debug)]
pub enum RestResult {
  Browse {
    index: BrowseIndex,
    rows: Vec<serde_json::Value>,
    next_cursor: Option<String>,
    as_of_height: Option<i32>,
  },
  Error(String),
}

/// Top-level event type consumed by the application event loop.
#[derive(Debug)]
pub enum AppEvent {
  Ipc(DaemonMessage),
  IpcDisconnected,
  IpcCommandFailed { id: u64, message: String },
  Terminal(TermEvent),
  RestResponse(RestResult),
  Tick,
}
