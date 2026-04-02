//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Latest block summary card.

use ratatui::{
  layout::Rect,
  text::{Line, Span},
  widgets::{Block, Borders, Paragraph},
  Frame,
};

use hyphend::wire::msg::{IngestionActivity, IngestionStateName};

use crate::model::AppState;
use crate::theme;

/// Render the latest-ingested block summary.
pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let block = Block::default()
    .title(Span::styled(" Latest Block ", theme::title()))
    .borders(Borders::ALL)
    .border_style(theme::muted())
    .style(theme::normal());

  let inner = block.inner(area);
  frame.render_widget(block, area);

  let lines = match state.snapshot.as_ref() {
    Some(snap) => {
      let hash = snap
        .ingestion_tip_hash
        .as_deref()
        .unwrap_or("--")
        .to_string();
      let height = snap
        .ingestion_tip
        .map(theme::fmt_num)
        .unwrap_or_else(|| "--".to_string());
      let session_total = theme::fmt_num(snap.blocks_ingested_total);
      let activity = match snap.ingestion_diag.activity {
        IngestionActivity::Active => "active",
        IngestionActivity::Idle => "idle",
        IngestionActivity::Sleeping => "sleeping",
      };
      let state_label = ingest_state_label(snap.ingestion_diag.state);
      let activity_line = if snap.ingestion_diag.stalled {
        " stalled".to_string()
      } else {
        format!("{activity} ({state_label})")
      };
      vec![
        Line::from(vec![
          Span::styled(" Hash    ", theme::muted()),
          Span::styled(hash, theme::normal()),
        ]),
        Line::from(vec![
          Span::styled(" Height  ", theme::muted()),
          Span::styled(height, theme::normal()),
        ]),
        Line::from(vec![
          Span::styled(" Session ", theme::muted()),
          Span::styled(format!("{session_total} blocks imported"), theme::normal()),
        ]),
        Line::from(vec![
          Span::styled(" State   ", theme::muted()),
          Span::styled(activity_line, theme::normal()),
        ]),
      ]
    }
    None => vec![Line::from(Span::styled(" --", theme::muted()))],
  };

  frame.render_widget(Paragraph::new(lines), inner);
}

fn ingest_state_label(state: IngestionStateName) -> &'static str {
  match state {
    IngestionStateName::Boot => "boot",
    IngestionStateName::Reconciling => "reconciling",
    IngestionStateName::PollingTip => "polling",
    IngestionStateName::Idle => "idle",
    IngestionStateName::Paused => "paused",
    IngestionStateName::Admitting => "admitting",
    IngestionStateName::Throttled => "throttled",
    IngestionStateName::Fetching => "fetching",
    IngestionStateName::Committing => "committing",
    IngestionStateName::Backoff => "backoff",
  }
}
