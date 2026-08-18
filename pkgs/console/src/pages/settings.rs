//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Settings page: display all daemon configuration sections.

use ratatui::{
  layout::Rect,
  text::{Line, Span},
  widgets::{Block, Borders, Paragraph, Wrap},
  Frame,
};

use crate::model::{AppState, Focus, Section};
use crate::theme;

/// Render the daemon configuration snapshot page.
pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let block = Block::default()
    .title(Span::styled(" Settings ", theme::title()))
    .borders(Borders::ALL)
    .border_style(main_section_border(state))
    .style(theme::normal());

  let inner = block.inner(area);
  frame.render_widget(block, area);

  let mut lines: Vec<Line> = Vec::new();

  match state.snapshot.as_ref() {
    Some(snap) => {
      let c = &snap.config;
      let pg = &snap.postgres_diag;

      section_header(&mut lines, "General");
      kv(&mut lines, "REST URL", &redact_url(&c.sources.rest_url));
      kv(
        &mut lines,
        "REST Concurrency",
        &opt_val(c.api.rest_concurrency),
      );
      kv(&mut lines, "API Enabled", &bool_str(c.api.enabled));
      kv(
        &mut lines,
        "API Bind",
        c.api.bind.as_deref().unwrap_or("--"),
      );
      lines.push(Line::from(""));

      section_header(&mut lines, "PostgreSQL");
      kv(&mut lines, "Version", pg.version.as_deref().unwrap_or("--"));
      kv(
        &mut lines,
        "Write Pool",
        &opt_val(c.postgres.write_pool_size),
      );
      kv(
        &mut lines,
        "Query Pool",
        &opt_val(c.postgres.query_pool_size),
      );
      kv(
        &mut lines,
        "Heavy Query Pool",
        &opt_val(c.postgres.heavy_query_pool_size),
      );
      kv(
        &mut lines,
        "Admin Pool",
        &opt_val(c.postgres.admin_pool_size),
      );
      kv(
        &mut lines,
        "Slow Query",
        &c.postgres
          .slow_query_ms
          .map(|ms| format!("{ms} ms"))
          .unwrap_or_else(|| "--".into()),
      );
      kv(&mut lines, "Max Connections", &opt_val(pg.max_connections));
      kv(
        &mut lines,
        "Shared Buffers",
        pg.shared_buffers.as_deref().unwrap_or("--"),
      );
      kv(
        &mut lines,
        "Work Mem",
        pg.work_mem.as_deref().unwrap_or("--"),
      );
      kv(
        &mut lines,
        "Eff. Cache Size",
        pg.effective_cache_size.as_deref().unwrap_or("--"),
      );
      if let Some(url) = c.postgres.url_redacted.as_deref() {
        kv(&mut lines, "Connection", url);
      }
      lines.push(Line::from(""));

      section_header(&mut lines, "JSON-RPC");
      kv(&mut lines, "Enabled", &bool_str(c.sources.rpc_enabled));
      kv(
        &mut lines,
        "URL",
        &c.sources
          .rpc_url
          .as_deref()
          .map(redact_url)
          .unwrap_or_else(|| "--".into()),
      );
      kv(
        &mut lines,
        "Auth",
        c.sources.rpc_auth_type.as_deref().unwrap_or("--"),
      );
      lines.push(Line::from(""));

      section_header(&mut lines, "ZMQ");
      kv(&mut lines, "Enabled", &bool_str(c.sources.zmq_enabled));
      kv(&mut lines, "Connect", &list_or_dash(&c.sources.zmq_connect));
      kv(
        &mut lines,
        "Subscribe",
        &list_or_dash(&c.sources.zmq_subscriptions),
      );
      lines.push(Line::from(""));

      section_header(&mut lines, "Live");
      kv(&mut lines, "Mempool", &bool_str(c.live.mempool));
      kv(&mut lines, "ISLock", &bool_str(c.live.islock));
      kv(&mut lines, "ChainLock", &bool_str(c.live.chainlock));
      kv(
        &mut lines,
        "ChainLock Start",
        &c.live
          .chainlock_start_block
          .map(|height| height.to_string())
          .unwrap_or_else(|| "--".into()),
      );
      kv(&mut lines, "Governance", &bool_str(c.live.governance));
      kv(&mut lines, "Network", &bool_str(c.live.network));
      kv(&mut lines, "Stream", &bool_str(c.live.stream));
      kv(
        &mut lines,
        "Recent Mempool",
        &opt_val(c.live.recent_mempool_txs),
      );
      kv(
        &mut lines,
        "Stream Buffer",
        &opt_val(c.live.stream_client_buffer),
      );
      kv(
        &mut lines,
        "Stream Heartbeat",
        &c.live
          .stream_heartbeat_secs
          .map(|secs| format!("{secs}s"))
          .unwrap_or_else(|| "--".into()),
      );
      lines.push(Line::from(""));

      section_header(&mut lines, "Overlays");
      kv(&mut lines, "ProTx", &bool_str(c.overlays.protx));
      kv(&mut lines, "ChainLock", &bool_str(c.overlays.chainlock));
      lines.push(Line::from(""));

      section_header(&mut lines, "Controls");
      kv(&mut lines, "Reindex Index", "Overview/Indices -> r");
      kv(&mut lines, "Disconnect Tip", "Overview -> d");
      kv(&mut lines, "Rewind Height", "Overview -> w");
      lines.push(Line::from(""));

      section_header(&mut lines, "Indices");
      lines.push(Line::from(vec![
        Span::styled("  address          ", theme::info()),
        bool_span(c.indices.address),
        Span::styled("    validator        ", theme::info()),
        bool_span(c.indices.validator),
      ]));
      lines.push(Line::from(vec![
        Span::styled("  metrics          ", theme::info()),
        bool_span(c.indices.metrics),
      ]));
    }
    None => {
      lines.push(Line::from(Span::styled(
        "  No snapshot data",
        theme::muted(),
      )));
    }
  }

  frame.render_widget(Paragraph::new(lines).wrap(Wrap { trim: false }), inner);
}

fn section_header(lines: &mut Vec<Line<'_>>, title: &'static str) {
  let pad = 50usize.saturating_sub(title.len() + 4);
  let header = format!("  {title} {}", "─".repeat(pad));
  lines.push(Line::from(Span::styled(header, theme::highlight())));
}

fn kv<'a>(lines: &mut Vec<Line<'a>>, key: &'static str, value: &str) {
  let padded = format!("  {key:17}");
  lines.push(Line::from(vec![
    Span::styled(padded, theme::info()),
    Span::styled(value.to_string(), theme::normal()),
  ]));
}

fn bool_str(v: bool) -> String {
  if v {
    "true".to_string()
  } else {
    "false".to_string()
  }
}

fn bool_span(v: bool) -> Span<'static> {
  if v {
    Span::styled("true", theme::connected())
  } else {
    Span::styled("false", theme::muted())
  }
}

fn opt_val<T: std::fmt::Display>(v: Option<T>) -> String {
  v.map(|x| x.to_string()).unwrap_or_else(|| "--".into())
}

fn list_or_dash(values: &[String]) -> String {
  if values.is_empty() {
    "--".into()
  } else {
    values.join(", ")
  }
}

fn redact_url(url: &str) -> String {
  if let Some(at_pos) = url.find('@') {
    if let Some(proto_end) = url.find("://") {
      let proto = &url[..proto_end + 3];
      let rest = &url[at_pos..];
      return format!("{proto}***:***{rest}");
    }
  }
  url.to_string()
}

fn main_section_border(state: &AppState) -> ratatui::style::Style {
  if state.focus == Focus::Main && state.section == Section::Settings {
    theme::title()
  } else {
    theme::muted()
  }
}
