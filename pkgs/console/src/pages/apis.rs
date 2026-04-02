//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! APIs tab: REST server stats and endpoint inventory.

use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  text::{Line, Span},
  widgets::{Block, Borders, Cell, Paragraph, Row, Table},
  Frame,
};

use crate::model::{ApiPanel, AppState, Focus, Section};
use crate::theme;

/// One indexed capability required for an endpoint to be ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum IndexRequirement {
  Address,
  Metrics,
  Validator,
}

/// One live capability required for an endpoint to be ready.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LiveRequirement {
  Governance,
  Islock,
  Mempool,
  Network,
  Stream,
}

/// One endpoint row shown in the console API inventory.
///
/// `requires_index` and `requires_live` describe runtime readiness gates.
/// `dependency` is a human-facing summary of the indexed/live domain the
/// endpoint reads from.
struct Endpoint {
  method: &'static str,
  path: &'static str,
  requires_index: &'static [IndexRequirement],
  requires_live: &'static [LiveRequirement],
  dependency: &'static str,
}

const ENDPOINTS: &[Endpoint] = &[
  Endpoint {
    method: "GET",
    path: "/api/v1/status",
    requires_index: &[],
    requires_live: &[],
    dependency: "--",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/blocks",
    requires_index: &[],
    requires_live: &[],
    dependency: "block",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/block/{hashOrHeight}",
    requires_index: &[],
    requires_live: &[],
    dependency: "block",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/block/{hashOrHeight}/txs",
    requires_index: &[],
    requires_live: &[],
    dependency: "block|tx",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/rawblock/{hashOrHeight}",
    requires_index: &[],
    requires_live: &[],
    dependency: "block raw payload",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/tx/{txid}",
    requires_index: &[],
    requires_live: &[LiveRequirement::Mempool, LiveRequirement::Islock],
    dependency: "tx|live:mempool*|live:islock*",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/address/{addr}",
    requires_index: &[IndexRequirement::Address],
    requires_live: &[],
    dependency: "spent/address summary",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/address/{addr}/txs",
    requires_index: &[IndexRequirement::Address],
    requires_live: &[],
    dependency: "spent/address history",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/address/{addr}/utxos",
    requires_index: &[IndexRequirement::Address],
    requires_live: &[],
    dependency: "script|spent",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/balances",
    requires_index: &[IndexRequirement::Address],
    requires_live: &[],
    dependency: "spent/address summary",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/validators",
    requires_index: &[IndexRequirement::Validator],
    requires_live: &[],
    dependency: "validator",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/validator/{proTxHash}",
    requires_index: &[IndexRequirement::Validator],
    requires_live: &[],
    dependency: "validator",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/metrics/latest",
    requires_index: &[IndexRequirement::Metrics],
    requires_live: &[],
    dependency: "metrics",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/metrics/chart",
    requires_index: &[IndexRequirement::Metrics],
    requires_live: &[],
    dependency: "metrics",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/txs",
    requires_index: &[],
    requires_live: &[],
    dependency: "tx",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/rawtx/{txid}",
    requires_index: &[],
    requires_live: &[LiveRequirement::Mempool],
    dependency: "tx raw payload|live:mempool*",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/mempool",
    requires_index: &[],
    requires_live: &[LiveRequirement::Mempool],
    dependency: "live:mempool",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/mempool/txs",
    requires_index: &[],
    requires_live: &[LiveRequirement::Mempool, LiveRequirement::Islock],
    dependency: "live:mempool|live:islock*",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/network",
    requires_index: &[],
    requires_live: &[LiveRequirement::Network],
    dependency: "live:network",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/proposals",
    requires_index: &[],
    requires_live: &[LiveRequirement::Governance],
    dependency: "live:governance",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/proposal/{hash}",
    requires_index: &[],
    requires_live: &[LiveRequirement::Governance],
    dependency: "live:governance",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/proposal/{hash}/votes",
    requires_index: &[],
    requires_live: &[LiveRequirement::Governance],
    dependency: "live:governance",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/search?q=...",
    requires_index: &[],
    requires_live: &[
      LiveRequirement::Governance,
      LiveRequirement::Islock,
      LiveRequirement::Mempool,
    ],
    dependency: "block|tx|address|validator|proposal|live overlays*",
  },
  Endpoint {
    method: "GET",
    path: "/api/v1/stream",
    requires_index: &[],
    requires_live: &[LiveRequirement::Stream],
    dependency: "live:stream",
  },
];

/// Render the API inventory and live browse panel.
pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let rows = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Length(10), Constraint::Min(10)])
    .split(area);

  draw_rest_server(frame, rows[0], state);
  draw_endpoint_table(frame, rows[1], state);
}

fn draw_rest_server(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let block = Block::default()
    .title(Span::styled(" REST Server ", theme::title()))
    .borders(Borders::ALL)
    .border_style(api_panel_border(state, ApiPanel::RestServer))
    .style(theme::normal());

  let inner = block.inner(area);
  let content_width = inner.width.saturating_sub(1) as usize;
  frame.render_widget(block, area);

  let snap = state.snapshot.as_ref();

  let api_bind = snap
    .and_then(|s| s.config.api.bind.as_deref())
    .unwrap_or("--");
  let api_enabled = snap.map(|s| s.config.api.enabled).unwrap_or(false);
  let concurrency = snap
    .and_then(|s| s.config.api.rest_concurrency)
    .map(|c| c.to_string())
    .unwrap_or_else(|| "--".into());

  let enabled_style = if api_enabled {
    theme::connected()
  } else {
    theme::muted()
  };

  let server = snap.map(|s| &s.server_stats);
  let avg_latency = server
    .and_then(|s| s.avg_latency_ms)
    .map(|ms| format!("{ms:.1}ms"))
    .unwrap_or_else(|| "--".into());
  let requests = server
    .map(|s| theme::fmt_num(s.total_requests))
    .unwrap_or_else(|| "--".into());
  let errors = server
    .map(|s| {
      if s.total_requests > 0 {
        format!(
          "{} ({:.2}%)",
          theme::fmt_num(s.total_errors),
          s.total_errors as f64 / s.total_requests as f64 * 100.0
        )
      } else {
        theme::fmt_num(s.total_errors)
      }
    })
    .unwrap_or_else(|| "--".into());

  let mut lines = vec![
    Line::from(vec![
      Span::styled("  Bind             ", theme::info()),
      Span::styled(
        truncate_value(api_bind, content_width.saturating_sub(17)),
        theme::normal(),
      ),
    ]),
    Line::from(vec![
      Span::styled("  Enabled          ", theme::info()),
      Span::styled(if api_enabled { "Yes" } else { "No" }, enabled_style),
    ]),
    Line::from(vec![
      Span::styled("  Concurrency      ", theme::info()),
      Span::styled(
        truncate_value(&concurrency, content_width.saturating_sub(17)),
        theme::normal(),
      ),
    ]),
    Line::from(vec![
      Span::styled("  Avg Latency      ", theme::info()),
      Span::styled(
        truncate_value(&avg_latency, content_width.saturating_sub(17)),
        theme::normal(),
      ),
    ]),
    Line::from(vec![
      Span::styled("  Requests         ", theme::info()),
      Span::styled(
        truncate_value(&requests, content_width.saturating_sub(17)),
        theme::normal(),
      ),
    ]),
    Line::from(vec![
      Span::styled("  Errors           ", theme::info()),
      Span::styled(
        truncate_value(&errors, content_width.saturating_sub(17)),
        theme::normal(),
      ),
    ]),
  ];

  // Swagger UI URL
  if api_enabled {
    let swagger_url = format!(
      "http://{}/api/docs",
      snap
        .and_then(|s| s.config.api.bind.as_deref())
        .unwrap_or("127.0.0.1:3080")
    );
    lines.push(Line::from(vec![
      Span::styled("  Swagger UI       ", theme::info()),
      Span::styled(
        truncate_value(&swagger_url, content_width.saturating_sub(17)),
        theme::highlight(),
      ),
    ]));
  }

  if let Some(err) = &state.rest_error {
    lines.push(Line::from(vec![
      Span::styled("  Last Error       ", theme::info()),
      Span::styled(
        truncate_value(err, content_width.saturating_sub(17)),
        theme::disconnected(),
      ),
    ]));
  }

  frame.render_widget(Paragraph::new(lines), inner);
}

fn draw_endpoint_table(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let block = Block::default()
    .title(Span::styled(" API Endpoints ", theme::title()))
    .borders(Borders::ALL)
    .border_style(api_panel_border(state, ApiPanel::Endpoints))
    .style(theme::normal());

  let api_enabled = state
    .snapshot
    .as_ref()
    .map(|s| s.config.api.enabled)
    .unwrap_or(false);

  let header = Row::new(vec![
    Cell::from(Span::styled(" Method", theme::table_header())),
    Cell::from(Span::styled("Path", theme::table_header())),
    Cell::from(Span::styled("Status", theme::table_header())),
    Cell::from(Span::styled("Depends On", theme::table_header())),
  ]);

  let method_width = (ENDPOINTS
    .iter()
    .map(|ep| ep.method.len())
    .max()
    .unwrap_or(6)
    + 2) as u16;
  let status_width = [
    "API off".len(),
    "Index off".len(),
    "Live off".len(),
    "Ready".len(),
    "Status".len(),
  ]
  .into_iter()
  .max()
  .unwrap_or(10) as u16
    + 2;
  let depends_width = (ENDPOINTS
    .iter()
    .map(|ep| ep.dependency.len())
    .max()
    .unwrap_or("Depends On".len())
    .max("Depends On".len())
    + 2) as u16;

  let table_rows: Vec<Row> = ENDPOINTS
    .iter()
    .map(|ep| {
      let (status_label, status_style) = if !api_enabled {
        ("API off", theme::muted())
      } else if !index_enabled(state, ep.requires_index) {
        ("Index off", theme::syncing())
      } else if !live_enabled(state, ep.requires_live) {
        ("Live off", theme::syncing())
      } else {
        ("Ready", theme::connected())
      };

      Row::new(vec![
        Cell::from(Span::styled(format!(" {}", ep.method), theme::info())),
        Cell::from(Span::styled(ep.path, theme::normal())),
        Cell::from(Span::styled(status_label, status_style)),
        Cell::from(Span::styled(ep.dependency, theme::muted())),
      ])
    })
    .collect();

  let table = Table::new(
    table_rows,
    [
      Constraint::Length(method_width),
      Constraint::Fill(1),
      Constraint::Length(status_width),
      Constraint::Length(depends_width),
    ],
  )
  .header(header)
  .block(block);

  frame.render_widget(table, area);
}

/// Return whether all indexed capabilities required by an endpoint are enabled.
fn index_enabled(state: &AppState, requirements: &[IndexRequirement]) -> bool {
  requirements.iter().all(|requirement| match requirement {
    IndexRequirement::Address => state
      .snapshot
      .as_ref()
      .is_some_and(|snap| snap.config.indices.address),
    IndexRequirement::Metrics => state
      .snapshot
      .as_ref()
      .is_some_and(|snap| snap.config.indices.metrics),
    IndexRequirement::Validator => state
      .snapshot
      .as_ref()
      .is_some_and(|snap| snap.config.indices.validator),
  })
}

/// Return whether all live capabilities required by an endpoint are enabled.
fn live_enabled(state: &AppState, requirements: &[LiveRequirement]) -> bool {
  requirements.iter().all(|requirement| match requirement {
    LiveRequirement::Governance => state
      .snapshot
      .as_ref()
      .is_some_and(|snap| snap.config.live.governance),
    LiveRequirement::Islock => state
      .snapshot
      .as_ref()
      .is_some_and(|snap| snap.config.live.islock),
    LiveRequirement::Mempool => state
      .snapshot
      .as_ref()
      .is_some_and(|snap| snap.config.live.mempool),
    LiveRequirement::Network => state
      .snapshot
      .as_ref()
      .is_some_and(|snap| snap.config.live.network),
    LiveRequirement::Stream => state
      .snapshot
      .as_ref()
      .is_some_and(|snap| snap.config.live.stream),
  })
}

fn api_panel_border(state: &AppState, panel: ApiPanel) -> ratatui::style::Style {
  if state.focus == Focus::Main && state.section == Section::Apis && state.api_panel == panel {
    theme::title()
  } else {
    theme::muted()
  }
}

fn truncate_value(value: &str, max_chars: usize) -> String {
  let len = value.chars().count();
  if len <= max_chars {
    return value.to_owned();
  }
  if max_chars <= 3 {
    return ".".repeat(max_chars);
  }
  let mut truncated = value.chars().take(max_chars - 3).collect::<String>();
  truncated.push_str("...");
  truncated
}
