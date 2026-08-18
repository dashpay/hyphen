//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Status bar with connection indicator and context hints.

use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  text::{Line, Span},
  widgets::Paragraph,
  Frame,
};

use crate::model::{AppState, Focus, OverviewSubFocus, Section};
use crate::theme;

fn recovery_status(state: &AppState) -> Option<String> {
  let snap = state.snapshot.as_ref()?;
  let progress = &snap.chain_progress_diag;
  if progress.recovery_state != "clean" {
    return Some(match progress.recovery_height {
      Some(height) => format!("recovery {} @ {}", progress.recovery_state, height),
      None => format!("recovery {}", progress.recovery_state),
    });
  }
  None
}

/// Render the footer status bar and context-sensitive key hints.
pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let cols = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Min(0), Constraint::Length(18)])
    .split(area);

  draw_left(frame, cols[0], state);

  let version = Paragraph::new(Line::from(Span::styled("Hyphen TUI v0.1 ", theme::muted())))
    .alignment(ratatui::layout::Alignment::Right)
    .style(theme::normal());
  frame.render_widget(version, cols[1]);
}

fn draw_left(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let conn_dot = if state.connected {
    Span::styled("● ", theme::connected())
  } else {
    Span::styled("● ", theme::disconnected())
  };

  let conn_text = if state.connected {
    Span::styled("Connected", theme::connected())
  } else {
    Span::styled("Disconnected", theme::disconnected())
  };

  let age = match state.last_snapshot_time {
    Some(t) => format!("  {}s ago", t.elapsed().as_secs()),
    None => String::new(),
  };

  let sep = Span::styled(format!("  {}  ", theme::DOT), theme::dither_style());

  let right_side: Vec<Span<'_>> = if let Some(status) = &state.command_status {
    let style = if status.is_error {
      theme::disconnected()
    } else if status.finished_at.is_some() {
      theme::connected()
    } else {
      theme::info()
    };
    let label = if status.finished_at.is_some() {
      "cmd"
    } else {
      "cmd active"
    };
    let elapsed_secs = status.started_at.elapsed().as_secs();
    vec![
      Span::styled(format!("{label} #{} ", status.id), theme::highlight()),
      Span::styled(status.message.as_str(), style),
      Span::styled(format!("  {}s", elapsed_secs), theme::muted()),
    ]
  } else if let Some((msg, _, is_err)) = &state.flash_message {
    let style = if *is_err {
      theme::disconnected()
    } else {
      theme::connected()
    };
    vec![Span::styled(msg.as_str(), style)]
  } else if let Some(message) = recovery_status(state) {
    vec![Span::styled(message, theme::info())]
  } else if let Some(prompt) = state.prompt.as_ref() {
    let hint = match prompt {
      crate::model::PromptState::Confirm(_) => "y:confirm  n/Esc:cancel",
      crate::model::PromptState::RewindInput { .. } => {
        "digits:type  Enter:send  Backspace:erase  Esc:cancel"
      }
    };
    vec![Span::styled(hint, theme::muted())]
  } else if state.browse_filter_active {
    vec![Span::styled(
      "type to filter  Esc/Enter:done",
      theme::muted(),
    )]
  } else {
    let hints = if state.focus == Focus::Main && state.section == Section::Overview {
      match state.overview_sub {
        OverviewSubFocus::Stores => "j/k:nav  d:disconnect  w:rewind  Tab:focus  q:quit",
        OverviewSubFocus::Indices => {
          "j/k:nav  Enter:browse  r:reindex  d:disconnect  w:rewind  /:filter  n/p:page  Tab:focus  q:quit"
        }
      }
    } else {
      "j/k:nav  Enter:select  Tab:focus  q:quit"
    };
    vec![Span::styled(hints, theme::muted())]
  };

  let mut spans = vec![
    Span::styled(" ", theme::normal()),
    conn_dot,
    conn_text,
    Span::styled(age, theme::muted()),
    sep,
  ];
  spans.extend(right_side);

  let line = Line::from(spans);
  let bar = Paragraph::new(line).style(theme::normal());
  frame.render_widget(bar, area);
}
