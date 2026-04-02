//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Browser filter widget.

use ratatui::{
  layout::Rect,
  text::{Line, Span},
  widgets::{Block, Borders, Paragraph},
  Frame,
};

use crate::model::AppState;
use crate::theme;

/// Render the browser filter control.
pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let block = Block::default()
    .title(Span::styled(" Filter ", theme::title()))
    .borders(Borders::ALL)
    .border_style(if state.browse_filter_active {
      theme::highlight()
    } else {
      theme::muted()
    })
    .style(theme::normal());

  let inner = block.inner(area);
  frame.render_widget(block, area);

  let content = if state.browse_filter_active {
    Line::from(vec![
      Span::styled(" / ", theme::highlight()),
      Span::styled(&state.browse_filter, theme::normal()),
      Span::styled("_", theme::highlight()),
    ])
  } else if state.browse_filter.is_empty() {
    Line::from(Span::styled(" Press / to filter columns", theme::muted()))
  } else {
    Line::from(vec![
      Span::styled(" / ", theme::muted()),
      Span::styled(&state.browse_filter, theme::normal()),
    ])
  };

  frame.render_widget(Paragraph::new(content), inner);
}
