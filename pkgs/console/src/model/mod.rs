//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Console model state shared across pages and widgets.

use std::collections::{HashMap, VecDeque};
use std::sync::mpsc;
use std::time::Instant;

use hyphend::wire::msg::{DaemonCommand, DaemonSnapshot};

use crate::event::{BrowseIndex, RestRequest};

const SECTION_COUNT: usize = 4;

/// Top-level sidebar sections rendered by the console.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Section {
  Overview,
  Apis,
  Health,
  Settings,
}

impl Section {
  pub const ALL: [Section; SECTION_COUNT] = [
    Section::Overview,
    Section::Apis,
    Section::Health,
    Section::Settings,
  ];

  /// Return the sidebar label shown for this section.
  pub fn label(self) -> &'static str {
    match self {
      Self::Overview => "Overview",
      Self::Apis => "APIs",
      Self::Health => "Health",
      Self::Settings => "Settings",
    }
  }
}

/// High-level focus target for keyboard navigation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Focus {
  Sidebar,
  Main,
}

/// Which sub-panel has cursor focus within the Overview main area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OverviewSubFocus {
  Stores,
  Indices,
}

/// Which panel has focus inside the Health page grid.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HealthPanel {
  Storage,
  Postgres,
  Services,
  Status,
}

/// Which panel has focus inside the APIs page.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ApiPanel {
  RestServer,
  Endpoints,
}

impl ApiPanel {
  /// Return the next panel when moving up/down inside the APIs page.
  pub fn offset(self, dy: i8) -> Self {
    match (self, dy.signum()) {
      (Self::RestServer, 1) => Self::Endpoints,
      (Self::Endpoints, -1) => Self::RestServer,
      _ => self,
    }
  }
}

impl HealthPanel {
  /// Return the next panel when moving left/right/up/down inside the 2x2 grid.
  pub fn offset(self, dx: i8, dy: i8) -> Self {
    let (mut col, mut row) = match self {
      Self::Storage => (0_i8, 0_i8),
      Self::Postgres => (1, 0),
      Self::Services => (0, 1),
      Self::Status => (1, 1),
    };
    col = (col + dx).clamp(0, 1);
    row = (row + dy).clamp(0, 1);
    match (col, row) {
      (0, 0) => Self::Storage,
      (1, 0) => Self::Postgres,
      (0, 1) => Self::Services,
      _ => Self::Status,
    }
  }
}

/// One daemon control the user can trigger from the console.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ControlAction {
  Reindex { stage: String, name: String },
  DisconnectTip,
}

/// One active modal prompt shown above the main console view.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PromptState {
  Confirm(ControlAction),
  RewindInput { value: String },
}

/// Fixed-capacity recent-sample buffer used for sparklines and rates.
#[derive(Debug)]
pub struct TimeSeries {
  buf: VecDeque<u64>,
  cap: usize,
}

impl TimeSeries {
  /// Allocate a fixed-capacity ring buffer for recent sample values.
  pub fn new(cap: usize) -> Self {
    Self {
      buf: VecDeque::with_capacity(cap),
      cap,
    }
  }

  /// Append one sample, evicting the oldest when the buffer is full.
  pub fn push(&mut self, value: u64) {
    if self.buf.len() >= self.cap {
      self.buf.pop_front();
    }
    self.buf.push_back(value);
  }

  /// Return the most recent sample, if any.
  pub fn last(&self) -> Option<u64> {
    self.buf.back().copied()
  }

  /// Return the most recent non-zero sample, if any.
  pub fn last_nonzero(&self) -> Option<u64> {
    self.buf.iter().rev().copied().find(|value| *value > 0)
  }

  /// Render the buffered values as a sparkline string.
  pub fn sparkline(&self, width: usize) -> String {
    let bars = ['▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    if self.buf.is_empty() {
      return " ".repeat(width);
    }
    let max = self.buf.iter().copied().max().unwrap_or(1).max(1);
    let start = self.buf.len().saturating_sub(width);
    self
      .buf
      .iter()
      .skip(start)
      .map(|&v| {
        let idx = ((v as f64 / max as f64) * 7.0).round() as usize;
        bars[idx.min(7)]
      })
      .collect()
  }
}

/// Mutable UI model shared by all console views.
#[derive(Debug)]
pub struct AppState {
  pub running: bool,
  pub focus: Focus,
  pub section: Section,
  pub sidebar_index: usize,
  pub snapshot: Option<DaemonSnapshot>,
  pub last_snapshot_time: Option<Instant>,
  pub connected: bool,
  pub ingestion_sparkline: TimeSeries,
  pub ingestion_rate: u64,
  pub prev_blocks_total: u64,
  pub prev_snapshot_instant: Option<Instant>,

  pub browse_index: Option<BrowseIndex>,
  pub browse_rows: Vec<serde_json::Value>,
  pub browse_next_cursor: Option<String>,
  pub browse_cursor_history: Vec<Option<String>>,
  pub browse_as_of_height: Option<i32>,
  pub browse_page: u32,
  pub browse_limit: u32,
  pub browse_filter: String,
  pub browse_filter_active: bool,
  pub page_scroll: u16,
  pub api_panel: ApiPanel,
  pub health_panel: HealthPanel,
  pub health_scrolls: [u16; 4],
  pub health_scroll_limits: [u16; 4],

  pub index_tree_cursor: usize,
  pub overview_sub: OverviewSubFocus,
  pub store_cursor: usize,

  pub rest_pending: bool,
  pub rest_error: Option<String>,

  pub index_sparklines: HashMap<String, TimeSeries>,
  pub prev_index_heights: HashMap<String, i32>,

  pub rest_tx: mpsc::Sender<RestRequest>,
  pub ipc_cmd_tx: mpsc::Sender<DaemonCommand>,
  pub next_cmd_id: u64,
  pub prompt: Option<PromptState>,
  pub command_status: Option<CommandStatus>,
  pub flash_message: Option<(String, Instant, bool)>,
}

/// Persistent status for the currently running or most recently finished command.
#[derive(Debug, Clone)]
pub struct CommandStatus {
  pub id: u64,
  pub message: String,
  pub started_at: Instant,
  pub finished_at: Option<Instant>,
  pub is_error: bool,
}

impl AppState {
  /// Build the initial UI state with connected worker channels.
  pub fn new(rest_tx: mpsc::Sender<RestRequest>, ipc_cmd_tx: mpsc::Sender<DaemonCommand>) -> Self {
    Self {
      running: true,
      focus: Focus::Sidebar,
      section: Section::Overview,
      sidebar_index: 0,
      snapshot: None,
      last_snapshot_time: None,
      connected: false,
      ingestion_sparkline: TimeSeries::new(60),
      ingestion_rate: 0,
      prev_blocks_total: 0,
      prev_snapshot_instant: None,
      browse_index: None,
      browse_rows: Vec::new(),
      browse_next_cursor: None,
      browse_cursor_history: vec![None],
      browse_as_of_height: None,
      browse_page: 0,
      browse_limit: 25,
      browse_filter: String::new(),
      browse_filter_active: false,
      page_scroll: 0,
      api_panel: ApiPanel::RestServer,
      health_panel: HealthPanel::Storage,
      health_scrolls: [0; 4],
      health_scroll_limits: [0; 4],
      index_tree_cursor: 0,
      overview_sub: OverviewSubFocus::Indices,
      store_cursor: 0,
      rest_pending: false,
      rest_error: None,
      index_sparklines: HashMap::new(),
      prev_index_heights: HashMap::new(),
      rest_tx,
      ipc_cmd_tx,
      next_cmd_id: 0,
      prompt: None,
      command_status: None,
      flash_message: None,
    }
  }
}
