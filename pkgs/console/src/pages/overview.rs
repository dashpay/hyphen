//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Overview page: Latest Block, Stores table, Indices tree, Browser.

use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  Frame,
};

use crate::model::AppState;
use crate::widgets;

/// Static store list: (store_key, display_name, backend_name)
pub const STORE_TREE: &[(&str, &str, &str)] = &[("chain", "Chain Data", "PostgreSQL")];

/// Static tree definition: (stage_key, display_name, depth,
/// _has_children_below)
pub const INDEX_TREE: &[(&str, &str, usize, bool)] = &[
  ("block", "Blocks", 0, true),
  ("metrics", "Metrics", 1, false),
  ("validator", "Validators", 1, false),
  ("transaction", "Transactions", 1, true),
  ("script", "Script", 2, true),
  ("spent", "Spent / Address", 3, true),
];

const LATEST_H: u16 = 4;
const STORES_H: u16 = 5;
const TREE_H: u16 = 9;
const FILTER_H: u16 = 3;
const MIN_BROWSER_H: u16 = 5;

/// Render the overview dashboard with stores, stages, and browser panes.
pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let base = LATEST_H + STORES_H + TREE_H;
  let remaining = area.height.saturating_sub(base);

  let (show_filter, browser_h) = if remaining >= FILTER_H + MIN_BROWSER_H {
    (true, remaining - FILTER_H)
  } else if remaining >= FILTER_H {
    (true, 0)
  } else {
    (false, 0)
  };

  let mut constraints = vec![
    Constraint::Length(LATEST_H),
    Constraint::Length(STORES_H),
    Constraint::Length(TREE_H),
  ];
  if show_filter {
    constraints.push(Constraint::Length(FILTER_H));
  }
  if browser_h > 0 {
    constraints.push(Constraint::Length(browser_h));
  }
  constraints.push(Constraint::Min(0));

  let rows = Layout::default()
    .direction(Direction::Vertical)
    .constraints(constraints)
    .split(area);

  let mut slot = 0usize;
  widgets::latest_block::draw(frame, rows[slot], state);
  slot += 1;
  widgets::store_tree::draw(frame, rows[slot], state);
  slot += 1;
  widgets::index_tree::draw(frame, rows[slot], state, INDEX_TREE);
  slot += 1;
  if show_filter {
    widgets::filter_bar::draw(frame, rows[slot], state);
    slot += 1;
  }
  if browser_h > 0 {
    widgets::table_browser::draw(frame, rows[slot], state);
  }
}
