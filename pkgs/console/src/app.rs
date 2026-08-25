//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Application setup, event loop, and key handling.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use crossterm::event::{Event, KeyCode, KeyModifiers};
use hyphend::wire::msg::{DaemonCommand, DaemonMessage};
use ratatui::backend::CrosstermBackend;
use ratatui::Terminal;

use crate::error::TuiError;
use crate::event::{AppEvent, BrowseIndex, RestRequest, RestResult};
use crate::model::{
  ApiPanel, AppState, ControlAction, Focus, HealthPanel, OverviewSubFocus, PromptState, Section,
};
use crate::pages::overview::{INDEX_TREE, STORE_TREE};
use crate::{appview, ipc, rest};

struct AppThreads {
  shutdown: Arc<AtomicBool>,
  ipc_reader: JoinHandle<()>,
  ipc_writer: JoinHandle<()>,
  rest_worker: JoinHandle<()>,
  terminal_worker: JoinHandle<()>,
  tick_worker: JoinHandle<()>,
}

impl AppThreads {
  fn start(
    socket_path: &Path,
    event_tx: mpsc::Sender<AppEvent>,
    rest_req_rx: mpsc::Receiver<RestRequest>,
    ipc_cmd_rx: mpsc::Receiver<DaemonCommand>,
  ) -> Self {
    let shutdown = Arc::new(AtomicBool::new(false));
    let ipc_reader = ipc::spawn_ipc_thread(socket_path, event_tx.clone(), Arc::clone(&shutdown));
    let ipc_writer = ipc::spawn_ipc_cmd_thread(socket_path, ipc_cmd_rx, event_tx.clone());
    let rest_worker = rest::spawn_rest_thread(event_tx.clone(), rest_req_rx);
    let terminal_worker = spawn_terminal_thread(event_tx.clone(), Arc::clone(&shutdown));
    let tick_worker = spawn_tick_thread(event_tx, Arc::clone(&shutdown));
    Self {
      shutdown,
      ipc_reader,
      ipc_writer,
      rest_worker,
      terminal_worker,
      tick_worker,
    }
  }

  fn shutdown(self) -> Result<(), TuiError> {
    self.shutdown.store(true, Ordering::Relaxed);
    join_thread("join IPC reader thread", self.ipc_reader)?;
    join_thread("join IPC command thread", self.ipc_writer)?;
    join_thread("join REST worker thread", self.rest_worker)?;
    join_thread("join terminal event thread", self.terminal_worker)?;
    join_thread("join tick thread", self.tick_worker)?;
    Ok(())
  }
}

struct TerminalSession {
  active: bool,
}

impl TerminalSession {
  fn enter() -> Result<Self, TuiError> {
    crossterm::terminal::enable_raw_mode().map_err(|e| TuiError::Io {
      context: "enable raw mode",
      source: e,
    })?;
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::EnterAlternateScreen).map_err(|e| {
      TuiError::Io {
        context: "enter alternate screen",
        source: e,
      }
    })?;
    Ok(Self { active: true })
  }

  fn restore(&mut self) -> Result<(), TuiError> {
    if !self.active {
      return Ok(());
    }
    let mut stdout = std::io::stdout();
    crossterm::execute!(stdout, crossterm::terminal::LeaveAlternateScreen).map_err(|e| {
      TuiError::Io {
        context: "leave alternate screen",
        source: e,
      }
    })?;
    crossterm::terminal::disable_raw_mode().map_err(|e| TuiError::Io {
      context: "disable raw mode",
      source: e,
    })?;
    self.active = false;
    Ok(())
  }
}

impl Drop for TerminalSession {
  fn drop(&mut self) {
    if !self.active {
      return;
    }
    let mut stdout = std::io::stdout();
    let _ = crossterm::execute!(stdout, crossterm::terminal::LeaveAlternateScreen);
    let _ = crossterm::terminal::disable_raw_mode();
  }
}

/// Run the TUI until the user exits or the event streams terminate.
///
/// # Errors
///
/// Returns `TuiError::Io` when terminal setup, drawing, or teardown fails.
pub fn run(datadir: &Path, api_url: Option<&str>) -> Result<(), TuiError> {
  let socket_path = datadir.join("hyphen.sock");

  let (event_tx, event_rx) = mpsc::channel::<AppEvent>();
  let (rest_req_tx, rest_req_rx) = mpsc::channel::<RestRequest>();
  let (ipc_cmd_tx, ipc_cmd_rx) = mpsc::channel::<DaemonCommand>();
  let threads = AppThreads::start(&socket_path, event_tx, rest_req_rx, ipc_cmd_rx);
  let mut session = TerminalSession::enter()?;

  let backend = CrosstermBackend::new(std::io::stdout());
  let mut terminal = Terminal::new(backend).map_err(|e| TuiError::Io {
    context: "create terminal",
    source: e,
  })?;

  let mut state = AppState::new(rest_req_tx, ipc_cmd_tx);
  state.api_url_override = api_url.map(str::to_owned);
  let result = event_loop(&mut terminal, &mut state, event_rx);
  drop(state);
  drop(terminal);
  let thread_result = threads.shutdown();
  let restore_result = session.restore();
  result?;
  thread_result?;
  restore_result
}

/// Drive the terminal redraw loop from the merged application event stream.
///
/// # Errors
///
/// Returns `TuiError::Io` when terminal drawing fails.
fn event_loop(
  terminal: &mut Terminal<CrosstermBackend<std::io::Stdout>>,
  state: &mut AppState,
  rx: mpsc::Receiver<AppEvent>,
) -> Result<(), TuiError> {
  refresh_layout_state(terminal, state)?;
  terminal
    .draw(|f| appview::draw(f, state))
    .map_err(|e| TuiError::Io {
      context: "terminal draw",
      source: e,
    })?;

  for event in rx {
    match event {
      AppEvent::Ipc(msg) => handle_ipc(state, msg),
      AppEvent::IpcDisconnected => {
        clear_ipc_state(state);
      }
      AppEvent::IpcCommandFailed { id, message } => {
        state.command_status = Some(crate::model::CommandStatus {
          id,
          message: message.clone(),
          started_at: state
            .command_status
            .as_ref()
            .filter(|status| status.id == id)
            .map_or_else(std::time::Instant::now, |status| status.started_at),
          finished_at: Some(std::time::Instant::now()),
          is_error: true,
        });
        state.flash_message = Some((message, std::time::Instant::now(), true));
      }
      AppEvent::Terminal(Event::Key(key)) => {
        handle_key(state, key.code, key.modifiers);
      }
      AppEvent::Terminal(_) => {}
      AppEvent::RestResponse(result) => handle_rest(state, result),
      AppEvent::Tick => handle_tick(state),
    }

    if !state.running {
      return Ok(());
    }

    refresh_layout_state(terminal, state)?;
    terminal
      .draw(|f| appview::draw(f, state))
      .map_err(|e| TuiError::Io {
        context: "terminal draw",
        source: e,
      })?;
  }

  Ok(())
}

fn refresh_layout_state(
  terminal: &Terminal<CrosstermBackend<std::io::Stdout>>,
  state: &mut AppState,
) -> Result<(), TuiError> {
  if state.section == Section::Health {
    let area = terminal.size().map_err(|e| TuiError::Io {
      context: "read terminal size",
      source: e,
    })?;
    let main_area = appview::main_area(area.into());
    state.health_scroll_limits = crate::pages::health::scroll_limits(main_area, state);
    for idx in 0..state.health_scrolls.len() {
      state.health_scrolls[idx] = state.health_scrolls[idx].min(state.health_scroll_limits[idx]);
    }
  }
  Ok(())
}

/// Drop all daemon-derived UI state after the IPC connection breaks.
fn clear_ipc_state(state: &mut AppState) {
  state.connected = false;
  state.snapshot = None;
  state.last_snapshot_time = None;
  state.ingestion_rate = 0;
  state.prev_blocks_total = 0;
  state.prev_snapshot_instant = None;
  state.ingestion_sparkline = crate::model::TimeSeries::new(60);
  state.index_sparklines.clear();
  state.prev_index_heights.clear();
}

/// Merge one inbound daemon message into the UI state model.
fn handle_ipc(state: &mut AppState, msg: DaemonMessage) {
  match msg {
    DaemonMessage::Snapshot(boxed) => {
      let snap = *boxed;
      state.connected = true;
      // Compute ingestion rate from blocks_ingested_total delta
      if let Some(prev_time) = state.prev_snapshot_instant {
        let elapsed = prev_time.elapsed().as_secs_f64();
        if elapsed > 0.5 {
          let delta = snap
            .blocks_ingested_total
            .saturating_sub(state.prev_blocks_total);
          state.ingestion_rate = (delta as f64 / elapsed).round() as u64;
        }
      }
      // Compute per-index height deltas for sparklines
      if let Some(prev_time) = state.prev_snapshot_instant {
        let elapsed = prev_time.elapsed().as_secs_f64();
        if elapsed > 0.5 {
          for sp in &snap.stages {
            let prev_h = state
              .prev_index_heights
              .get(&sp.stage)
              .copied()
              .unwrap_or(sp.indexed_height);
            let delta = (sp.indexed_height - prev_h).max(0) as u64;
            let rate = (delta as f64 / elapsed).round() as u64;
            let ts = state
              .index_sparklines
              .entry(sp.stage.clone())
              .or_insert_with(|| crate::model::TimeSeries::new(16));
            ts.push(rate);
            state
              .prev_index_heights
              .insert(sp.stage.clone(), sp.indexed_height);
          }
        }
      } else {
        for sp in &snap.stages {
          state
            .prev_index_heights
            .insert(sp.stage.clone(), sp.indexed_height);
        }
      }
      state.prev_blocks_total = snap.blocks_ingested_total;
      state.prev_snapshot_instant = Some(std::time::Instant::now());
      state.last_snapshot_time = Some(std::time::Instant::now());
      state.snapshot = Some(snap);
    }
    DaemonMessage::BlockIngested { .. }
    | DaemonMessage::IndexProgress { .. }
    | DaemonMessage::TxReceived { .. } => {
      state.connected = true;
    }
    DaemonMessage::CommandProgress { id, message } => {
      state.connected = true;
      state.command_status = Some(crate::model::CommandStatus {
        id,
        message: message.clone(),
        started_at: state
          .command_status
          .as_ref()
          .filter(|status| status.id == id)
          .map_or_else(std::time::Instant::now, |status| status.started_at),
        finished_at: None,
        is_error: false,
      });
      state.flash_message = Some((message, std::time::Instant::now(), false));
    }
    DaemonMessage::CommandResult {
      id,
      success,
      message,
      ..
    } => {
      state.command_status = Some(crate::model::CommandStatus {
        id,
        message: message.clone(),
        started_at: state
          .command_status
          .as_ref()
          .filter(|status| status.id == id)
          .map_or_else(std::time::Instant::now, |status| status.started_at),
        finished_at: Some(std::time::Instant::now()),
        is_error: !success,
      });
      state.flash_message = Some((message, std::time::Instant::now(), !success));
    }
  }
}

/// Merge one REST browse result into the UI state model.
fn handle_rest(state: &mut AppState, result: RestResult) {
  state.rest_pending = false;
  match result {
    RestResult::Browse {
      index,
      rows,
      next_cursor,
      as_of_height,
    } => {
      state.rest_error = None;
      state.browse_index = Some(index);
      state.browse_rows = rows;
      state.browse_next_cursor = next_cursor;
      state.browse_as_of_height = as_of_height;
    }
    RestResult::Error(e) => {
      state.rest_error = Some(e.clone());
      state.flash_message = Some((
        format!("REST fetch failed: {e}"),
        std::time::Instant::now(),
        true,
      ));
    }
  }
}

fn spawn_terminal_thread(tx: mpsc::Sender<AppEvent>, shutdown: Arc<AtomicBool>) -> JoinHandle<()> {
  return std::thread::spawn(move || loop {
    if shutdown.load(Ordering::Relaxed) {
      return;
    }
    match crossterm::event::poll(Duration::from_millis(250)) {
      Ok(true) => match crossterm::event::read() {
        Ok(event) => {
          if tx.send(AppEvent::Terminal(event)).is_err() {
            return;
          }
        }
        Err(error) => {
          tracing::error!("terminal event read error: {error}");
          return;
        }
      },
      Ok(false) => {}
      Err(error) => {
        tracing::error!("terminal event poll error: {error}");
        return;
      }
    }
  });
}

fn spawn_tick_thread(tx: mpsc::Sender<AppEvent>, shutdown: Arc<AtomicBool>) -> JoinHandle<()> {
  return std::thread::spawn(move || loop {
    std::thread::sleep(Duration::from_secs(1));
    if shutdown.load(Ordering::Relaxed) {
      return;
    }
    if tx.send(AppEvent::Tick).is_err() {
      return;
    }
  });
}

fn join_thread(context: &'static str, handle: JoinHandle<()>) -> Result<(), TuiError> {
  handle.join().map_err(|_| TuiError::ThreadPanic { context })
}

#[cfg(test)]
mod tests {
  use super::{handle_rest, issue_daemon_command};
  use crate::event::{BrowseIndex, RestResult};
  use crate::model::AppState;
  use hyphend::wire::msg::DaemonCommand;

  #[test]
  fn rest_errors_preserve_existing_rows_and_flash_error() {
    let (rest_tx, _) = std::sync::mpsc::channel();
    let (ipc_tx, _) = std::sync::mpsc::channel();
    let mut state = AppState::new(rest_tx, ipc_tx);
    state.browse_index = Some(BrowseIndex::Blocks);
    state.browse_rows = vec![serde_json::json!({"height": 7})];

    handle_rest(&mut state, RestResult::Error("shape mismatch".to_owned()));

    assert_eq!(state.browse_rows.len(), 1);
    assert_eq!(state.rest_error.as_deref(), Some("shape mismatch"));
    assert!(state.flash_message.is_some());
  }

  #[test]
  fn issue_daemon_command_reports_local_queue_failure() {
    let (rest_tx, _) = std::sync::mpsc::channel();
    let (ipc_tx, ipc_rx) = std::sync::mpsc::channel();
    drop(ipc_rx);
    let mut state = AppState::new(rest_tx, ipc_tx);

    let queued = issue_daemon_command(
      &mut state,
      7,
      DaemonCommand::DisconnectTip { id: 7 },
      "disconnect_tip waiting_for_idle".to_owned(),
      "Disconnecting current tip...".to_owned(),
    );

    assert!(!queued);
    let status = state.command_status.expect("status should exist");
    assert_eq!(status.id, 7);
    assert!(status.is_error);
    assert!(status.finished_at.is_some());
    assert!(status.message.contains("daemon command queue failed"));
  }
}

/// Advance time-based UI state once per tick.
fn handle_tick(state: &mut AppState) {
  state.ingestion_sparkline.push(state.ingestion_rate);
  if let Some(status) = &state.command_status {
    if status
      .finished_at
      .is_some_and(|finished| finished.elapsed().as_secs() >= 10)
    {
      state.command_status = None;
    }
  }
  if let Some((_, t, _)) = &state.flash_message {
    if t.elapsed().as_secs() >= 5 {
      state.flash_message = None;
    }
  }
}

/// Apply one keyboard input to the current UI state.
fn handle_key(state: &mut AppState, code: KeyCode, modifiers: KeyModifiers) {
  // Global quit
  match code {
    KeyCode::Char('q') if !state.browse_filter_active => {
      state.running = false;
      return;
    }
    KeyCode::Char('c') if modifiers.contains(KeyModifiers::CONTROL) => {
      state.running = false;
      return;
    }
    _ => {}
  }

  // Confirmation dialog input
  if state.prompt.is_some() {
    handle_prompt_key(state, code);
    return;
  }

  // Filter mode input
  if state.browse_filter_active {
    match code {
      KeyCode::Esc | KeyCode::Enter => {
        state.browse_filter_active = false;
      }
      KeyCode::Backspace => {
        state.browse_filter.pop();
      }
      KeyCode::Char(c) => {
        state.browse_filter.push(c);
      }
      _ => {}
    }
    return;
  }

  match code {
    KeyCode::Tab => {
      state.focus = match state.focus {
        Focus::Sidebar => Focus::Main,
        Focus::Main => Focus::Sidebar,
      };
    }
    KeyCode::Char('j') => {
      if state.focus == Focus::Main && state.section == Section::Health {
        scroll_health_panel(state, 1);
      } else if state.focus == Focus::Sidebar {
        state.sidebar_index = (state.sidebar_index + 1).min(Section::ALL.len() - 1);
      } else if state.focus == Focus::Main && state.section == Section::Overview {
        match state.overview_sub {
          OverviewSubFocus::Stores => {
            if state.store_cursor + 1 < STORE_TREE.len() {
              state.store_cursor += 1;
            } else {
              state.overview_sub = OverviewSubFocus::Indices;
              state.index_tree_cursor = 0;
            }
          }
          OverviewSubFocus::Indices => {
            state.index_tree_cursor = (state.index_tree_cursor + 1).min(INDEX_TREE.len() - 1);
          }
        }
      } else if state.focus == Focus::Main {
        state.page_scroll = state.page_scroll.saturating_add(1);
      }
    }
    KeyCode::Down => {
      if state.focus == Focus::Sidebar {
        state.sidebar_index = (state.sidebar_index + 1).min(Section::ALL.len() - 1);
      } else if state.focus == Focus::Main && state.section == Section::Health {
        state.health_panel = state.health_panel.offset(0, 1);
      } else if state.focus == Focus::Main && state.section == Section::Apis {
        state.api_panel = state.api_panel.offset(1);
      } else if state.focus == Focus::Main && state.section == Section::Overview {
        match state.overview_sub {
          OverviewSubFocus::Stores => {
            if state.store_cursor + 1 < STORE_TREE.len() {
              state.store_cursor += 1;
            } else {
              state.overview_sub = OverviewSubFocus::Indices;
              state.index_tree_cursor = 0;
            }
          }
          OverviewSubFocus::Indices => {
            state.index_tree_cursor = (state.index_tree_cursor + 1).min(INDEX_TREE.len() - 1);
          }
        }
      } else if state.focus == Focus::Main {
        state.page_scroll = state.page_scroll.saturating_add(1);
      }
    }
    KeyCode::Char('k') => {
      if state.focus == Focus::Main && state.section == Section::Health {
        scroll_health_panel(state, -1);
      } else if state.focus == Focus::Sidebar {
        state.sidebar_index = state.sidebar_index.saturating_sub(1);
      } else if state.focus == Focus::Main && state.section == Section::Overview {
        match state.overview_sub {
          OverviewSubFocus::Stores => {
            state.store_cursor = state.store_cursor.saturating_sub(1);
          }
          OverviewSubFocus::Indices => {
            if state.index_tree_cursor == 0 {
              state.overview_sub = OverviewSubFocus::Stores;
              state.store_cursor = STORE_TREE.len().saturating_sub(1);
            } else {
              state.index_tree_cursor -= 1;
            }
          }
        }
      } else if state.focus == Focus::Main {
        state.page_scroll = state.page_scroll.saturating_sub(1);
      }
    }
    KeyCode::Up => {
      if state.focus == Focus::Sidebar {
        state.sidebar_index = state.sidebar_index.saturating_sub(1);
      } else if state.focus == Focus::Main && state.section == Section::Health {
        state.health_panel = state.health_panel.offset(0, -1);
      } else if state.focus == Focus::Main && state.section == Section::Apis {
        state.api_panel = state.api_panel.offset(-1);
      } else if state.focus == Focus::Main && state.section == Section::Overview {
        match state.overview_sub {
          OverviewSubFocus::Stores => {
            state.store_cursor = state.store_cursor.saturating_sub(1);
          }
          OverviewSubFocus::Indices => {
            if state.index_tree_cursor == 0 {
              state.overview_sub = OverviewSubFocus::Stores;
              state.store_cursor = STORE_TREE.len().saturating_sub(1);
            } else {
              state.index_tree_cursor -= 1;
            }
          }
        }
      } else if state.focus == Focus::Main {
        state.page_scroll = state.page_scroll.saturating_sub(1);
      }
    }
    KeyCode::Char('h') => {
      if state.focus == Focus::Main && state.section == Section::Health {
        state.health_panel = state.health_panel.offset(-1, 0);
      }
    }
    KeyCode::Left => {
      if state.focus == Focus::Main && state.section == Section::Health {
        state.health_panel = state.health_panel.offset(-1, 0);
      }
    }
    KeyCode::Char('l') => {
      if state.focus == Focus::Main && state.section == Section::Health {
        state.health_panel = state.health_panel.offset(1, 0);
      }
    }
    KeyCode::Right => {
      if state.focus == Focus::Main && state.section == Section::Health {
        state.health_panel = state.health_panel.offset(1, 0);
      }
    }
    KeyCode::Enter => {
      if state.focus == Focus::Sidebar {
        state.section = Section::ALL[state.sidebar_index];
        state.focus = Focus::Main;
        on_section_enter(state);
      } else if state.focus == Focus::Main && state.section == Section::Overview {
        if let Some(idx) = cursor_to_browse_index(state.index_tree_cursor) {
          state.browse_page = 0;
          state.browse_cursor_history = vec![None];
          state.browse_next_cursor = None;
          state.browse_as_of_height = None;
          state.browse_filter.clear();
          fire_browse(state, idx);
        } else {
          state.browse_index = None;
          state.browse_rows.clear();
          state.browse_next_cursor = None;
          state.browse_cursor_history = vec![None];
          state.browse_as_of_height = None;
          state.browse_page = 0;
        }
      }
    }
    KeyCode::Char('/') => {
      if state.section == Section::Overview {
        state.browse_filter_active = true;
        state.browse_filter.clear();
      }
    }
    KeyCode::Char('n') => {
      if state.focus == Focus::Main
        && state.section == Section::Overview
        && state.browse_index.is_some()
        && state.browse_next_cursor.is_some()
      {
        let Some(next_cursor) = state.browse_next_cursor.clone() else {
          return;
        };
        state.browse_cursor_history.push(Some(next_cursor));
        state.browse_page += 1;
        if let Some(idx) = state.browse_index {
          fire_browse(state, idx);
        }
      }
    }
    KeyCode::Char('p') => {
      if state.focus == Focus::Main && state.section == Section::Overview && state.browse_page > 0 {
        state.browse_cursor_history.pop();
        state.browse_page -= 1;
        if let Some(idx) = state.browse_index {
          fire_browse(state, idx);
        }
      }
    }
    KeyCode::Char('r') => {
      if state.focus == Focus::Main
        && state.section == Section::Overview
        && state.overview_sub == OverviewSubFocus::Indices
        && state.connected
        && state.index_tree_cursor < INDEX_TREE.len()
      {
        let (stage, name, _, _) = INDEX_TREE[state.index_tree_cursor];
        state.prompt = Some(PromptState::Confirm(ControlAction::Reindex {
          stage: stage.to_owned(),
          name: name.to_owned(),
        }));
      }
    }
    KeyCode::Char('d') => {
      if state.focus == Focus::Main && state.section == Section::Overview && state.connected {
        state.prompt = Some(PromptState::Confirm(ControlAction::DisconnectTip));
      }
    }
    KeyCode::Char('w') => {
      if state.focus == Focus::Main && state.section == Section::Overview && state.connected {
        state.prompt = Some(PromptState::RewindInput {
          value: String::new(),
        });
      }
    }
    KeyCode::Esc => {
      state.focus = Focus::Sidebar;
    }
    KeyCode::Char(c @ '1'..='4') => {
      let idx = (c as usize) - ('1' as usize);
      if idx < Section::ALL.len() {
        state.sidebar_index = idx;
        state.section = Section::ALL[idx];
      }
    }
    _ => {}
  }
}

fn handle_prompt_key(state: &mut AppState, code: KeyCode) {
  let Some(prompt) = state.prompt.clone() else {
    return;
  };

  match prompt {
    PromptState::Confirm(action) => match code {
      KeyCode::Char('y') => {
        state.prompt = None;
        issue_control_action(state, action);
      }
      KeyCode::Char('n') | KeyCode::Esc => {
        state.prompt = None;
      }
      _ => {}
    },
    PromptState::RewindInput { mut value } => match code {
      KeyCode::Esc => {
        state.prompt = None;
      }
      KeyCode::Backspace => {
        value.pop();
        state.prompt = Some(PromptState::RewindInput { value });
      }
      KeyCode::Char(c) if c.is_ascii_digit() => {
        value.push(c);
        state.prompt = Some(PromptState::RewindInput { value });
      }
      KeyCode::Enter => {
        if value.is_empty() {
          state.flash_message = Some((
            "Rewind height cannot be empty".to_owned(),
            std::time::Instant::now(),
            true,
          ));
          state.prompt = Some(PromptState::RewindInput { value });
          return;
        }
        match value.parse::<u32>() {
          Ok(target_height) => {
            state.prompt = None;
            state.next_cmd_id += 1;
            let command_id = state.next_cmd_id;
            let _ = issue_daemon_command(
              state,
              command_id,
              DaemonCommand::Rewind {
                id: command_id,
                target_height,
              },
              format!("rewind waiting_for_idle target_height={target_height}"),
              format!("Rewinding to height {target_height}..."),
            );
          }
          Err(_) => {
            state.flash_message = Some((
              "Rewind height is invalid".to_owned(),
              std::time::Instant::now(),
              true,
            ));
            state.prompt = Some(PromptState::RewindInput { value });
          }
        }
      }
      _ => {
        state.prompt = Some(PromptState::RewindInput { value });
      }
    },
  }
}

fn issue_control_action(state: &mut AppState, action: ControlAction) {
  state.next_cmd_id += 1;
  let command_id = state.next_cmd_id;
  match action {
    ControlAction::Reindex { stage, name } => {
      if !issue_daemon_command(
        state,
        command_id,
        DaemonCommand::ForceReindex {
          id: command_id,
          stage,
        },
        format!("reindexing {name}"),
        format!("Reindexing {name}..."),
      ) {
        return;
      }
    }
    ControlAction::DisconnectTip => {
      let _ = issue_daemon_command(
        state,
        command_id,
        DaemonCommand::DisconnectTip { id: command_id },
        "disconnect_tip waiting_for_idle".to_owned(),
        "Disconnecting current tip...".to_owned(),
      );
    }
  }
}

fn issue_daemon_command(
  state: &mut AppState,
  command_id: u64,
  command: DaemonCommand,
  status_message: String,
  flash_message: String,
) -> bool {
  if let Err(error) = state.ipc_cmd_tx.send(command) {
    let message = format!("daemon command queue failed: {error}");
    state.command_status = Some(crate::model::CommandStatus {
      id: command_id,
      message: message.clone(),
      started_at: std::time::Instant::now(),
      finished_at: Some(std::time::Instant::now()),
      is_error: true,
    });
    state.flash_message = Some((message, std::time::Instant::now(), true));
    return false;
  }
  state.command_status = Some(crate::model::CommandStatus {
    id: command_id,
    message: status_message,
    started_at: std::time::Instant::now(),
    finished_at: None,
    is_error: false,
  });
  state.flash_message = Some((flash_message, std::time::Instant::now(), false));
  true
}

fn cursor_to_browse_index(cursor: usize) -> Option<BrowseIndex> {
  match cursor {
    0 => Some(BrowseIndex::Blocks),
    1 => Some(BrowseIndex::Metrics),
    2 => Some(BrowseIndex::Validators),
    3 => Some(BrowseIndex::Transactions),
    _ => None,
  }
}

fn on_section_enter(state: &mut AppState) {
  state.page_scroll = 0;
  if state.section == Section::Health {
    state.health_panel = HealthPanel::Storage;
    state.health_scrolls = [0; 4];
    state.health_scroll_limits = [0; 4];
  }
  if state.section == Section::Apis {
    state.api_panel = ApiPanel::RestServer;
  }
  if state.section == Section::Overview {
    state.overview_sub = OverviewSubFocus::Indices;
    state.store_cursor = 0;
    state.index_tree_cursor = 0;
    state.browse_page = 0;
    state.browse_cursor_history = vec![None];
    state.browse_next_cursor = None;
    state.browse_as_of_height = None;
    fire_browse(state, BrowseIndex::Blocks);
  }
}

fn scroll_health_panel(state: &mut AppState, delta: i16) {
  let idx = match state.health_panel {
    HealthPanel::Storage => 0,
    HealthPanel::Postgres => 1,
    HealthPanel::Services => 2,
    HealthPanel::Status => 3,
  };
  let current = state.health_scrolls[idx] as i16;
  let max = state.health_scroll_limits[idx] as i16;
  state.health_scrolls[idx] = (current + delta).clamp(0, max) as u16;
}

fn fire_browse(state: &mut AppState, index: BrowseIndex) {
  // Resolved per request from the latest frame, so moving the bind or turning
  // the API off is reflected without restarting the console.
  let base_url = match browse_endpoint(state) {
    crate::apibase::ApiBase::Url(url) => url,
    crate::apibase::ApiBase::Disabled => {
      state.rest_pending = false;
      state.rest_error =
        Some("REST API is disabled in hyphen.conf ([api] absent); nothing to browse".to_owned());
      return;
    }
    crate::apibase::ApiBase::Unknown => {
      state.rest_pending = false;
      state.rest_error = Some("waiting for the daemon to report its API address".to_owned());
      return;
    }
  };
  state.rest_pending = true;
  state.rest_error = None;
  state.browse_next_cursor = None;
  let _ = state.rest_tx.send(RestRequest::Browse {
    base_url,
    index,
    limit: state.browse_limit,
    cursor: state.browse_cursor_history.last().cloned().unwrap_or(None),
  });
}

/// Where browse requests should go right now, from the flag and the most
/// recent heartbeat.
fn browse_endpoint(state: &AppState) -> crate::apibase::ApiBase {
  crate::apibase::resolve(
    state.api_url_override.as_deref(),
    state
      .snapshot
      .as_ref()
      .map(|snap| (snap.config.api.enabled, snap.config.api.bind.as_deref())),
  )
}
