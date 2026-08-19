//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Store progress table.

use ratatui::{
  layout::{Constraint, Rect},
  text::Span,
  widgets::{Block, Borders, Cell, Row, Table},
  Frame,
};

use hyphend::wire::msg::{IngestionActivity, IngestionStateName};

use crate::model::{AppState, Focus, OverviewSubFocus};
use crate::theme;
use crate::widgets::{calc_eta, calc_eta_by_transactions};

/// Render the storage backends table.
pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let stores_focused = state.focus == Focus::Main && state.overview_sub == OverviewSubFocus::Stores;
  let block = Block::default()
    .title(Span::styled(" Stores ", theme::title()))
    .borders(Borders::ALL)
    .border_style(if stores_focused {
      theme::title()
    } else {
      theme::muted()
    })
    .style(theme::normal());

  let header = Row::new(vec![
    Cell::from(Span::styled(" Name", theme::table_header())),
    Cell::from(Span::styled("Backend", theme::table_header())),
    Cell::from(Span::styled("Height", theme::table_header())),
    Cell::from(Span::styled("Tip", theme::table_header())),
    Cell::from(Span::styled("blk/s", theme::table_header())),
    Cell::from(Span::styled("ETA", theme::table_header())),
    Cell::from(Span::styled("Status", theme::table_header())),
  ]);
  let table_rows: Vec<Row> = match state.snapshot.as_ref() {
    Some(snap) => {
      let abs_tip = snap.network_tip;
      let ingestion_height = snap.ingestion_tip;
      let (height, tip, mut status, mut style) = match (ingestion_height, abs_tip) {
        (Some(height), Some(tip)) if tip > 0 => {
          let ratio = height as f64 / tip as f64;
          if ratio >= 1.0 {
            (
              theme::fmt_num(height),
              theme::fmt_num(tip),
              "Synced".to_string(),
              theme::connected(),
            )
          } else {
            // Both figures, always. A block percentage on its own read "36%"
            // on a run that was ~6% done by transaction.
            let progress = match snap.chain_progress_diag.tx_progress_pct {
              Some(tx_pct) => format!("{:.0}% blk / {tx_pct:.0}% tx", ratio * 100.0),
              None => format!("{:.0}% blk", ratio * 100.0),
            };
            (
              theme::fmt_num(height),
              theme::fmt_num(tip),
              format!("Importing ({progress})"),
              theme::syncing(),
            )
          }
        }
        (Some(height), None) => (
          theme::fmt_num(height),
          "--".to_string(),
          "Importing".to_string(),
          theme::syncing(),
        ),
        _ => (
          "--".to_string(),
          "--".to_string(),
          "Waiting".to_string(),
          theme::muted(),
        ),
      };
      status = match snap.ingestion_diag.activity {
        IngestionActivity::Active => format!("{status} [active]"),
        IngestionActivity::Idle => format!("{status} [idle]"),
        IngestionActivity::Sleeping => format!("{status} [sleeping]"),
      };
      status = format!(
        "{status} / {}",
        ingest_state_label(snap.ingestion_diag.state)
      );
      if snap.ingestion_diag.stalled {
        status = "Stalled".to_string();
        style = theme::disconnected();
      }
      let bps = state.ingestion_sparkline.last().unwrap_or(0);
      let bps_str = if bps > 0 {
        bps.to_string()
      } else {
        "-".to_string()
      };
      // Prefer the transaction-weighted ETA; the block-rate one is optimistic
      // by roughly the density climb still ahead of it.
      let eta = calc_eta_by_transactions(
        snap.chain_progress_diag.transactions_indexed,
        snap.chain_progress_diag.transactions_estimated_total,
        snap.chain_progress_diag.txs_per_block_recent,
        bps,
      )
      .unwrap_or_else(|| {
        calc_eta(
          ingestion_height.map(|height| height as i32),
          abs_tip.map(|tip| tip as i32),
          bps,
        )
      });
      let name_style = if stores_focused && state.store_cursor == 0 {
        theme::highlight()
      } else {
        theme::normal()
      };
      vec![Row::new(vec![
        Cell::from(Span::styled(" Chain Data", name_style)),
        Cell::from(Span::styled("PostgreSQL", theme::muted())),
        Cell::from(Span::styled(height, theme::normal())),
        Cell::from(Span::styled(tip, theme::muted())),
        Cell::from(Span::styled(bps_str, theme::info())),
        Cell::from(Span::styled(eta, theme::muted())),
        Cell::from(Span::styled(status, style)),
      ])]
    }
    None => vec![Row::new(vec![Cell::from(Span::styled(
      " No data",
      theme::muted(),
    ))])],
  };

  let table = Table::new(
    table_rows,
    [
      Constraint::Min(12),
      Constraint::Length(12),
      Constraint::Length(10),
      Constraint::Length(10),
      Constraint::Length(7),
      Constraint::Length(10),
      Constraint::Length(16),
    ],
  )
  .header(header)
  .block(block);

  frame.render_widget(table, area);
}

fn ingest_state_label(state: IngestionStateName) -> &'static str {
  match state {
    IngestionStateName::Boot => "boot",
    IngestionStateName::PollingTip => "polling",
    IngestionStateName::Idle => "idle",
    IngestionStateName::Paused => "paused",
    IngestionStateName::Fetching => "fetching",
    IngestionStateName::Committing => "committing",
    IngestionStateName::Backoff => "backoff",
  }
}
