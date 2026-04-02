//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Main view rendering: layout, section dispatch, confirmation popup.

use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  text::{Line, Span},
  widgets::{Block, Borders, Clear, Paragraph},
  Frame,
};

use crate::model::{AppState, ControlAction, PromptState, Section};
use crate::{pages, sidebar, status_bar, theme};

/// Render the full application frame for the current state.
pub fn draw(frame: &mut Frame<'_>, state: &AppState) {
  let outer = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Min(3), Constraint::Length(1)])
    .split(frame.area());

  let body = split_body(outer[0]);

  frame.render_widget(Paragraph::new("").style(theme::normal()), frame.area());

  sidebar::draw(frame, body[0], state);
  draw_main(frame, body[1], state);
  status_bar::draw(frame, outer[1], state);

  if let Some(prompt) = state.prompt.as_ref() {
    draw_prompt_popup(frame, prompt);
  }
}

/// Return the main content area to the right of the sidebar.
pub fn main_area(area: Rect) -> Rect {
  split_body(split_outer(area)[0])[1]
}

fn split_outer(area: Rect) -> std::rc::Rc<[Rect]> {
  Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Min(3), Constraint::Length(1)])
    .split(area)
}

fn split_body(area: Rect) -> std::rc::Rc<[Rect]> {
  Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Length(22), Constraint::Min(20)])
    .split(area)
}

fn draw_main(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  match state.section {
    Section::Overview => pages::overview::draw(frame, area, state),
    Section::Apis => pages::apis::draw(frame, area, state),
    Section::Health => pages::health::draw(frame, area, state),
    Section::Settings => pages::settings::draw(frame, area, state),
  }
}

fn draw_prompt_popup(frame: &mut Frame<'_>, prompt: &PromptState) {
  let popup_w = 52_u16;
  let popup_h = 9_u16;
  let area = frame.area();
  let x = area.x + area.width.saturating_sub(popup_w) / 2;
  let y = area.y + area.height.saturating_sub(popup_h) / 2;
  let popup = Rect::new(x, y, popup_w.min(area.width), popup_h.min(area.height));

  frame.render_widget(Clear, popup);

  let (title, lines) = match prompt {
    PromptState::Confirm(action) => confirm_prompt(action),
    PromptState::RewindInput { value } => rewind_prompt(value),
  };

  let block = Block::default()
    .title(Span::styled(format!(" {title} "), theme::title()))
    .borders(Borders::ALL)
    .border_style(theme::highlight())
    .style(theme::normal());
  let inner = block.inner(popup);
  frame.render_widget(block, popup);

  frame.render_widget(Paragraph::new(lines), inner);
}

fn confirm_prompt(action: &ControlAction) -> (&'static str, Vec<Line<'static>>) {
  let message = match action {
    ControlAction::Reindex { name, .. } => format!("  Reindex \"{name}\" from scratch?"),
    ControlAction::Compact { name, .. } => format!("  Compact RocksDB store \"{name}\"?"),
    ControlAction::DisconnectTip => "  Disconnect the current indexed tip?".to_owned(),
  };
  let detail = match action {
    ControlAction::Reindex { .. } => "  All data for this index will be re-processed.",
    ControlAction::Compact { .. } => "  Compaction can increase IO while it runs.",
    ControlAction::DisconnectTip => "  The daemon will rewind exactly one block.",
  };

  (
    "Confirm Action",
    vec![
      Line::from(""),
      Line::from(Span::styled(message, theme::normal())),
      Line::from(Span::styled(detail, theme::muted())),
      Line::from(""),
      Line::from(vec![
        Span::styled("    [y]", theme::highlight()),
        Span::styled(" Confirm  ", theme::normal()),
        Span::styled("[n]", theme::highlight()),
        Span::styled(" Cancel", theme::normal()),
      ]),
    ],
  )
}

fn rewind_prompt(value: &str) -> (&'static str, Vec<Line<'static>>) {
  (
    "Rewind Height",
    vec![
      Line::from(""),
      Line::from(Span::styled(
        "  Enter the target height to rewind all indices to.",
        theme::normal(),
      )),
      Line::from(Span::styled(
        "  This uses the daemon Rewind command.",
        theme::muted(),
      )),
      Line::from(""),
      Line::from(vec![
        Span::styled("    Height: ", theme::info()),
        Span::styled(value.to_owned(), theme::highlight()),
      ]),
      Line::from(""),
      Line::from(vec![
        Span::styled("    [Enter]", theme::highlight()),
        Span::styled(" Send  ", theme::normal()),
        Span::styled("[Esc]", theme::highlight()),
        Span::styled(" Cancel", theme::normal()),
      ]),
    ],
  )
}
