//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Sidebar with logo and section navigation.

use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  style::Modifier,
  text::{Line, Span},
  widgets::{Block, Borders, List, ListItem, ListState, Paragraph},
  Frame,
};

use crate::model::{AppState, Focus, Section};
use crate::theme;

/// Render the sidebar logo and section menu.
pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let rows = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Length(4), Constraint::Min(0)])
    .split(area);

  draw_logo(frame, rows[0]);
  draw_menu(frame, rows[1], state);
}

fn draw_logo(frame: &mut Frame<'_>, area: Rect) {
  let w = area.width.saturating_sub(1) as usize;
  let dither_line = theme::DITHER_LIGHT.repeat(w);
  let label = " hyphen ";
  let right_pad = w.saturating_sub(2 + label.len());
  let pink_dither = ratatui::style::Style::default()
    .fg(theme::PASTEL_PINK)
    .bg(theme::SIDEBAR_BG);
  let lines = vec![
    Line::from(Span::styled(&dither_line, pink_dither)),
    Line::from(vec![
      Span::styled(theme::DITHER_LIGHT.repeat(2), pink_dither),
      Span::styled(label, theme::title()),
      Span::styled(theme::DITHER_LIGHT.repeat(right_pad), pink_dither),
    ]),
    Line::from(Span::styled(&dither_line, pink_dither)),
    Line::from(""),
  ];

  let block = Block::default()
    .borders(Borders::RIGHT)
    .border_style(theme::muted())
    .style(ratatui::style::Style::default().bg(theme::SIDEBAR_BG));

  let inner = block.inner(area);
  frame.render_widget(block, area);
  frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_menu(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let items: Vec<ListItem> = Section::ALL
    .iter()
    .enumerate()
    .map(|(i, sec)| {
      let style = if i == state.sidebar_index {
        theme::sidebar_selected()
      } else {
        theme::sidebar_item()
      };
      let marker = if i == state.sidebar_index {
        "▸ "
      } else {
        "  "
      };
      ListItem::new(Line::from(Span::styled(
        format!("{marker}{}", sec.label()),
        style,
      )))
    })
    .collect();

  let border_style = if state.focus == Focus::Sidebar {
    theme::title()
  } else {
    theme::muted()
  };

  let block = Block::default()
    .borders(Borders::RIGHT)
    .border_style(border_style)
    .style(ratatui::style::Style::default().bg(theme::SIDEBAR_BG));

  let list = List::new(items)
    .block(block)
    .highlight_style(theme::sidebar_selected().add_modifier(Modifier::BOLD));

  let mut list_state = ListState::default().with_selected(Some(state.sidebar_index));
  frame.render_stateful_widget(list, area, &mut list_state);
}
