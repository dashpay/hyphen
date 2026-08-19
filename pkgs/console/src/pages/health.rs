//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Health page: storage, source diagnostics, and batch tuner status.

use ratatui::{
  layout::{Constraint, Direction, Layout, Rect},
  text::{Line, Span},
  widgets::{Block, Borders, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, Wrap},
  Frame,
};

use hyphend::wire::msg::{ForwardLanePhase, IngestionActivity, IngestionStateName};

use crate::model::{AppState, Focus, HealthPanel, Section};
use crate::theme;

/// Render the health and diagnostics page.
pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let [storage, postgres, services, status] = health_panel_areas(area);

  draw_storage(frame, storage, state);
  draw_postgres(frame, postgres, state);
  draw_services(frame, services, state);
  draw_live_sources(frame, status, state);
}

/// Compute the current per-panel health scroll limits for the given render area.
pub fn scroll_limits(area: Rect, state: &AppState) -> [u16; 4] {
  let [storage, postgres, services, status] = health_panel_areas(area);
  [
    panel_scroll_limit(
      panel_inner(storage),
      build_storage_lines(panel_inner(storage), state),
      false,
    ),
    panel_scroll_limit(
      panel_inner(postgres),
      build_postgres_lines(panel_inner(postgres), state),
      false,
    ),
    panel_scroll_limit(
      panel_inner(services),
      build_services_lines(panel_inner(services), state),
      false,
    ),
    panel_scroll_limit(
      panel_inner(status),
      build_status_lines(panel_inner(status), state),
      false,
    ),
  ]
}

fn draw_storage(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let block = Block::default()
    .title(Span::styled(" Storage ", theme::title()))
    .borders(Borders::ALL)
    .border_style(panel_border_style(state, HealthPanel::Storage))
    .style(theme::normal());
  let inner = block.inner(area);
  frame.render_widget(block, area);
  let lines = build_storage_lines(inner, state);

  render_panel(frame, inner, lines, state, HealthPanel::Storage, false);
}

fn draw_postgres(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let block = Block::default()
    .title(Span::styled(" PostgreSQL ", theme::title()))
    .borders(Borders::ALL)
    .border_style(panel_border_style(state, HealthPanel::Postgres))
    .style(theme::normal());
  let inner = block.inner(area);
  frame.render_widget(block, area);
  let lines = build_postgres_lines(inner, state);

  render_panel(frame, inner, lines, state, HealthPanel::Postgres, false);
}

fn draw_services(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let block = Block::default()
    .title(Span::styled(" Services ", theme::title()))
    .borders(Borders::ALL)
    .border_style(panel_border_style(state, HealthPanel::Services))
    .style(theme::normal());
  let inner = block.inner(area);
  frame.render_widget(block, area);
  let lines = build_services_lines(inner, state);

  render_panel(frame, inner, lines, state, HealthPanel::Services, false);
}

fn draw_live_sources(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let block = Block::default()
    .title(Span::styled(" Status ", theme::title()))
    .borders(Borders::ALL)
    .border_style(panel_border_style(state, HealthPanel::Status))
    .style(theme::normal());
  let inner = block.inner(area);
  frame.render_widget(block, area);
  let lines = build_status_lines(inner, state);

  render_panel(frame, inner, lines, state, HealthPanel::Status, false);
}

fn build_storage_lines(inner: Rect, state: &AppState) -> Vec<Line<'static>> {
  let content_width = panel_text_width(inner) as usize;
  let mut lines: Vec<Line> = Vec::new();
  if let Some(snap) = state.snapshot.as_ref() {
    let storage = &snap.storage_diag;
    kv(
      &mut lines,
      "Database",
      &storage
        .database_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "--".into()),
      content_width,
    );
    kv(
      &mut lines,
      "WAL Written",
      &storage
        .wal_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "--".into()),
      content_width,
    );
    let conn_str = match (storage.connections, storage.max_connections) {
      (Some(open), Some(max)) => format!("{open} / {max}"),
      (Some(open), None) => open.to_string(),
      _ => "--".into(),
    };
    kv(&mut lines, "Connections", &conn_str, content_width);
    let vacuum_str = match storage.autovacuum_running {
      Some(n) if n > 0 => format!("Running ({n})"),
      Some(_) => "Idle".to_string(),
      None => "--".to_string(),
    };
    kv(&mut lines, "Autovacuum", &vacuum_str, content_width);
    kv(
      &mut lines,
      "Undo Window",
      &storage
        .undo_span
        .map(|(lo, hi)| format!("{}..{}", theme::fmt_num(lo), theme::fmt_num(hi)))
        .unwrap_or_else(|| "--".into()),
      content_width,
    );
    let activity = match snap.ingestion_diag.activity {
      IngestionActivity::Active => "Active".to_string(),
      IngestionActivity::Idle => "Idle".to_string(),
      IngestionActivity::Sleeping => "Sleeping".to_string(),
    };
    kv(
      &mut lines,
      "Ingestion",
      &format!(
        "{activity} ({})",
        ingest_state_label(snap.ingestion_diag.state)
      ),
      content_width,
    );
    // Stepped sync defers the open partition's query-only indexes, so an
    // operator seeing a fast sync and a slow explorer query has somewhere to
    // look. Blank on an older daemon that does not report it.
    if !snap.ingestion_diag.sync_mode.is_empty() {
      kv(
        &mut lines,
        "Sync Mode",
        &snap.ingestion_diag.sync_mode,
        content_width,
      );
    }
    if snap.ingestion_diag.stalled {
      kv(&mut lines, "Watchdog", "Stalled", content_width);
    }
    if !storage.pool_stats.is_empty() {
      lines.push(Line::from(Span::styled("  Pools:", theme::info())));
      for (name, size, idle) in &storage.pool_stats {
        lines.push(storage_line(
          content_width,
          4,
          name,
          &format!("{} open / {idle} idle", size),
        ));
      }
    }
  } else {
    lines.push(Line::from(Span::styled("  No data", theme::muted())));
  }
  lines
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

fn build_postgres_lines(inner: Rect, state: &AppState) -> Vec<Line<'static>> {
  let content_width = panel_text_width(inner) as usize;
  let mut lines: Vec<Line> = Vec::new();
  if let Some(snap) = state.snapshot.as_ref() {
    let pg = &snap.postgres_diag;
    kv(
      &mut lines,
      "Write Pool",
      &pool_stats(pg.write_pool_size, pg.write_pool_available),
      content_width,
    );
    kv(
      &mut lines,
      "Query Pool",
      &pool_stats(pg.query_pool_size, pg.query_pool_available),
      content_width,
    );
    kv(
      &mut lines,
      "Admin Pool",
      &pool_stats(pg.admin_pool_size, pg.admin_pool_available),
      content_width,
    );
    kv(
      &mut lines,
      "Max Connections",
      &pg
        .max_connections
        .map(|v| v.to_string())
        .unwrap_or_else(|| "--".into()),
      content_width,
    );
    let pg_total = match (snap.pg_db_bytes, snap.pg_wal_bytes) {
      (Some(db), Some(wal)) => Some(db + wal),
      (Some(db), None) => Some(db),
      _ => None,
    };
    kv(
      &mut lines,
      "Disk (total)",
      &pg_total.map(format_bytes).unwrap_or_else(|| "--".into()),
      content_width,
    );
    kv(
      &mut lines,
      "DB",
      &snap
        .pg_db_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "--".into()),
      content_width,
    );
    kv(
      &mut lines,
      "WAL",
      &snap
        .pg_wal_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "--".into()),
      content_width,
    );
    if !snap.pg_table_sizes.is_empty() {
      lines.push(Line::from(Span::styled("  Tables:", theme::info())));
      for (tbl, bytes) in snap.pg_table_sizes.iter().take(8) {
        match split_pg_partition_name(tbl) {
          Some((base, part)) => {
            lines.push(Line::from(vec![Span::styled(
              truncate_with_indent(base, content_width, 4),
              theme::muted(),
            )]));
            lines.push(storage_line(content_width, 6, &part, &format_bytes(*bytes)));
          }
          None => lines.push(storage_line(content_width, 4, tbl, &format_bytes(*bytes))),
        }
      }
    }
  } else {
    lines.push(Line::from(Span::styled("  No data", theme::muted())));
  }
  lines
}

fn build_services_lines(inner: Rect, state: &AppState) -> Vec<Line<'static>> {
  let content_width = panel_text_width(inner) as usize;
  let mut lines: Vec<Line> = Vec::new();
  service_line(
    &mut lines,
    "REST",
    true,
    Some(snap_value(state, |snap| {
      snap.config.sources.rest_url.clone()
    })),
    content_width,
  );
  if let Some(snap) = state.snapshot.as_ref() {
    service_line(
      &mut lines,
      "JSON-RPC",
      snap.config.sources.rpc_enabled,
      snap.config.sources.rpc_url.as_deref().map(redact_url),
      content_width,
    );
    service_line(
      &mut lines,
      "ZMQ",
      snap.config.sources.zmq_enabled,
      (!snap.config.sources.zmq_connect.is_empty())
        .then(|| snap.config.sources.zmq_connect.join(", ")),
      content_width,
    );
    service_line(
      &mut lines,
      "PostgreSQL",
      snap.config.sources.postgres_enabled,
      None,
      content_width,
    );
    service_line(
      &mut lines,
      "API",
      snap.config.api.enabled,
      snap.config.api.bind.clone(),
      content_width,
    );
    service_line(&mut lines, "IPC", state.connected, None, content_width);
    kv(
      &mut lines,
      "Daemon Uptime",
      &format_duration(snap.uptime_secs),
      content_width,
    );
    kv(
      &mut lines,
      "Session Imports",
      &theme::fmt_num(snap.blocks_ingested_total),
      content_width,
    );
    kv(
      &mut lines,
      "Database Size",
      &snap
        .storage_diag
        .database_bytes
        .map(format_bytes)
        .unwrap_or_else(|| "--".into()),
      content_width,
    );
  }
  lines
}

fn build_status_lines(inner: Rect, state: &AppState) -> Vec<Line<'static>> {
  let content_width = panel_text_width(inner) as usize;
  let mut lines: Vec<Line> = Vec::new();
  if let Some(snap) = state.snapshot.as_ref() {
    let scheduler = &snap.index_scheduler_diag;
    let rest = &snap.rest_stats;
    let rpc = &snap.rpc_diag;
    let zmq = &snap.zmq_diag;
    source_section(
      &mut lines,
      "Core Pipeline",
      &[
        (
          "Health",
          format!("{:?}", snap.core_pipeline_diag.health).to_lowercase(),
        ),
        (
          "Forward Lane",
          scheduler
            .active_lane
            .map(lane_label)
            .unwrap_or("idle")
            .to_owned(),
        ),
        (
          "Forward Phase",
          scheduler
            .forward
            .phase
            .map(forward_phase_label)
            .unwrap_or("--")
            .to_owned(),
        ),
        (
          "Forward Height",
          scheduler
            .forward
            .height
            .map(theme::fmt_num)
            .unwrap_or_else(|| "--".into()),
        ),
        (
          "Last Commit",
          scheduler
            .forward
            .last_success_height
            .map(theme::fmt_num)
            .unwrap_or_else(|| "--".into()),
        ),
        (
          "Sync by Block",
          snap
            .chain_progress_diag
            .block_progress_pct
            .map(|pct| format!("{pct:.1}%"))
            .unwrap_or_else(|| "--".into()),
        ),
        (
          // The figure that tracks remaining work: block percentage overstates
          // progress because chain density rises toward the tip.
          "Sync by Transaction",
          match (
            snap.chain_progress_diag.tx_progress_pct,
            snap.chain_progress_diag.transactions_indexed,
            snap.chain_progress_diag.transactions_estimated_total,
          ) {
            (Some(pct), Some(indexed), Some(total)) => format!(
              "{pct:.1}% ({} of ~{})",
              theme::fmt_num(indexed),
              theme::fmt_num(total)
            ),
            (_, Some(indexed), _) => format!("{} indexed", theme::fmt_num(indexed)),
            _ => "--".into(),
          },
        ),
        (
          "Tx per Block (recent)",
          snap
            .chain_progress_diag
            .txs_per_block_recent
            .map(|density| format!("{density:.1}"))
            .unwrap_or_else(|| "--".into()),
        ),
        (
          "Forward Freshness",
          snap
            .core_pipeline_diag
            .last_forward_success_epoch
            .map(|epoch| format!("{}s ago", now_epoch().saturating_sub(epoch)))
            .unwrap_or_else(|| "--".into()),
        ),
        (
          "Recovery",
          if scheduler.forward.stalled {
            format!("stalled (attempt {})", scheduler.forward.recovery_attempts)
          } else {
            scheduler.forward.recovery_attempts.to_string()
          },
        ),
      ],
      content_width,
    );
    lines.push(Line::from(""));
    source_section(
      &mut lines,
      "Live Sources: REST",
      &[
        ("Requests", theme::fmt_num(rest.total_requests)),
        (
          "Latency",
          rest
            .avg_latency_ms
            .map(|ms| format!("{ms:.0}ms"))
            .unwrap_or_else(|| "--".into()),
        ),
        (
          "Error %",
          error_rate(rest.total_errors, rest.total_requests),
        ),
      ],
      content_width,
    );
    lines.push(Line::from(""));
    source_section(
      &mut lines,
      "Live Sources: JSON-RPC",
      &[
        (
          "State",
          source_status(rpc.enabled, rpc.connected).to_owned(),
        ),
        ("Requests", theme::fmt_num(rpc.total_calls)),
        ("Latency", "--".into()),
        ("Error %", error_rate(rpc.total_errors, rpc.total_calls)),
        ("Auth", rpc.auth_type.as_deref().unwrap_or("--").to_owned()),
        ("Methods", top_counts(&rpc.method_counts, 3)),
      ],
      content_width,
    );
    lines.push(Line::from(""));
    source_section(
      &mut lines,
      "Live Sources: ZMQ",
      &[
        (
          "State",
          source_status(zmq.enabled, zmq.connected).to_owned(),
        ),
        ("Requests", theme::fmt_num(zmq.total_events)),
        ("Latency", "--".into()),
        ("Error %", error_rate(zmq.total_errors, zmq.total_events)),
        ("Seq Gaps", theme::fmt_num(zmq.sequence_gaps)),
        ("Topics", list_or_dash(&zmq.subscriptions)),
      ],
      content_width,
    );
  } else {
    lines.push(Line::from(Span::styled("  No data", theme::muted())));
  }
  lines
}

fn now_epoch() -> u64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs()
}

fn kv<'a>(lines: &mut Vec<Line<'a>>, key: &'static str, value: &str, width: usize) {
  let padded = format!("  {key:18}");
  let value = truncate_value(value, width.saturating_sub(padded.len()));
  lines.push(Line::from(vec![
    Span::styled(padded, theme::info()),
    Span::styled(value, theme::normal()),
  ]));
}

fn format_bytes(b: u64) -> String {
  const TB: u64 = 1024 * 1024 * 1024 * 1024;
  const GB: u64 = 1024 * 1024 * 1024;
  const MB: u64 = 1024 * 1024;
  const KB: u64 = 1024;
  if b >= TB {
    format!("{:.2} TB", b as f64 / TB as f64)
  } else if b >= GB {
    format!("{:.2} GB", b as f64 / GB as f64)
  } else if b >= MB {
    format!("{:.1} MB", b as f64 / MB as f64)
  } else if b >= KB {
    format!("{:.1} KB", b as f64 / KB as f64)
  } else {
    format!("{b} B")
  }
}

fn format_duration(secs: u64) -> String {
  let days = secs / 86400;
  let hours = (secs % 86400) / 3600;
  let minutes = (secs % 3600) / 60;
  let s = secs % 60;
  if days > 0 {
    format!("{days}d {hours}h {minutes}m")
  } else if hours > 0 {
    format!("{hours}h {minutes}m {s}s")
  } else if minutes > 0 {
    format!("{minutes}m {s}s")
  } else {
    format!("{s}s")
  }
}

fn source_status(enabled: bool, connected: bool) -> &'static str {
  if !enabled {
    "Disabled"
  } else if connected {
    "Connected"
  } else {
    "Down"
  }
}

fn service_line<'a>(
  lines: &mut Vec<Line<'a>>,
  name: &'static str,
  enabled: bool,
  detail: Option<String>,
  width: usize,
) {
  let state = if enabled { "ON" } else { "OFF" };
  let label = format!("  {name:<12}");
  let state_style = if enabled {
    theme::connected()
  } else {
    theme::muted()
  };
  let detail = detail.filter(|value| !value.is_empty()).unwrap_or_default();
  let suffix = if detail.is_empty() {
    String::new()
  } else {
    format!(" ({detail})")
  };
  let available = width.saturating_sub(label.len() + state.len() + 1);
  let suffix = truncate_value(&suffix, available);
  lines.push(Line::from(vec![
    Span::styled(label, theme::info()),
    Span::styled(format!("{state:<3}"), state_style),
    Span::styled(suffix, theme::normal()),
  ]));
}

fn source_section<'a>(
  lines: &mut Vec<Line<'a>>,
  title: &'static str,
  rows: &[(&str, String)],
  width: usize,
) {
  lines.push(Line::from(Span::styled(
    format!("  {title}"),
    theme::highlight(),
  )));
  for (key, value) in rows {
    if matches!(*key, "Methods" | "Topics") && value != "--" {
      lines.push(Line::from(vec![Span::styled(
        format!("    {key}"),
        theme::info(),
      )]));
      for item in value.split(", ") {
        lines.push(Line::from(vec![Span::styled(
          truncate_with_indent(item, width, 6),
          theme::normal(),
        )]));
      }
    } else {
      let label = format!("    {key:<10}");
      let value = truncate_value(value, width.saturating_sub(label.len()));
      lines.push(Line::from(vec![
        Span::styled(label, theme::info()),
        Span::styled(value, theme::normal()),
      ]));
    }
  }
}

fn error_rate(errors: u64, total: u64) -> String {
  if total == 0 {
    "--".into()
  } else {
    format!("{:.2}%", errors as f64 / total as f64 * 100.0)
  }
}

fn snap_value(
  state: &AppState,
  f: impl FnOnce(&hyphend::wire::msg::DaemonSnapshot) -> String,
) -> String {
  state
    .snapshot
    .as_ref()
    .map(f)
    .unwrap_or_else(|| "--".into())
}

fn pool_stats(total: Option<usize>, idle: Option<usize>) -> String {
  match (total, idle) {
    (Some(total), Some(idle)) => format!(
      "{} active / {} idle / {} max",
      total.saturating_sub(idle),
      idle,
      total
    ),
    (Some(total), None) => format!("{total} (pool size)"),
    _ => "--".into(),
  }
}

fn top_counts(counts: &[(String, u64)], limit: usize) -> String {
  if counts.is_empty() {
    "--".into()
  } else {
    counts
      .iter()
      .take(limit)
      .map(|(name, count)| format!("{name}:{}", theme::fmt_num(*count)))
      .collect::<Vec<_>>()
      .join(", ")
  }
}

fn list_or_dash(values: &[String]) -> String {
  if values.is_empty() {
    "--".into()
  } else {
    values.join(", ")
  }
}

fn redact_url(url: &str) -> String {
  if let Some((scheme, rest)) = url.split_once("://") {
    format!("{scheme}://{rest}")
  } else {
    url.to_owned()
  }
}

fn lane_label(lane: hyphend::wire::msg::IndexSchedulerLane) -> &'static str {
  match lane {
    hyphend::wire::msg::IndexSchedulerLane::Idle => "idle",
    hyphend::wire::msg::IndexSchedulerLane::Forward => "forward",
    hyphend::wire::msg::IndexSchedulerLane::TxTreeCatchUp => "tx catch-up",
    hyphend::wire::msg::IndexSchedulerLane::MetricsCatchUp => "metrics catch-up",
    hyphend::wire::msg::IndexSchedulerLane::ValidatorCatchUp => "validator catch-up",
  }
}

fn forward_phase_label(phase: ForwardLanePhase) -> &'static str {
  match phase {
    ForwardLanePhase::Idle => "idle",
    ForwardLanePhase::AcquireChainState => "chain",
    ForwardLanePhase::BeginTx => "begin",
    ForwardLanePhase::FetchBlock => "fetch",
    ForwardLanePhase::PlanConnect => "plan",
    ForwardLanePhase::ApplyStagePlan => "apply",
    ForwardLanePhase::CommitTx => "commit",
  }
}

fn split_pg_partition_name(name: &str) -> Option<(&str, String)> {
  match name.rsplit_once("_p") {
    Some((base, suffix)) if !suffix.is_empty() && suffix.chars().all(|ch| ch.is_ascii_digit()) => {
      Some((base, format!("p{suffix}")))
    }
    _ => None,
  }
}

fn storage_line<'a>(width: usize, indent: usize, name: &str, value: &str) -> Line<'a> {
  let value_width = value.chars().count();
  let min_gap = 2;
  let right_padding = 1;
  let reserve = indent + value_width + min_gap + right_padding;
  let name = truncate_value(name, width.saturating_sub(reserve));
  let gap = width
    .saturating_sub(indent + name.chars().count() + value_width + right_padding)
    .max(min_gap);
  Line::from(vec![
    Span::styled(format!("{}{}", " ".repeat(indent), name), theme::muted()),
    Span::styled(" ".repeat(gap), theme::normal()),
    Span::styled(value.to_owned(), theme::normal()),
  ])
}

fn truncate_with_indent(value: &str, width: usize, indent: usize) -> String {
  format!(
    "{}{}",
    " ".repeat(indent),
    truncate_value(value, width.saturating_sub(indent))
  )
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

fn panel_border_style(state: &AppState, panel: HealthPanel) -> ratatui::style::Style {
  if state.focus == Focus::Main && state.section == Section::Health && state.health_panel == panel {
    theme::title()
  } else {
    theme::muted()
  }
}

fn health_scroll(state: &AppState, panel: HealthPanel) -> u16 {
  match panel {
    HealthPanel::Storage => state.health_scrolls[0],
    HealthPanel::Postgres => state.health_scrolls[1],
    HealthPanel::Services => state.health_scrolls[2],
    HealthPanel::Status => state.health_scrolls[3],
  }
}

fn render_panel(
  frame: &mut Frame<'_>,
  inner: Rect,
  lines: Vec<Line<'_>>,
  state: &AppState,
  panel: HealthPanel,
  wrap: bool,
) {
  let scroll = health_scroll(state, panel) as usize;
  let text_area = panel_text_area(inner);
  let viewport = text_area.height as usize;
  let content_height = visual_line_count(&lines, text_area.width as usize, wrap);
  let max_scroll = content_height.saturating_sub(viewport);
  let scroll = scroll.min(max_scroll);

  let mut paragraph = Paragraph::new(lines).scroll((scroll as u16, 0));
  if wrap {
    paragraph = paragraph.wrap(Wrap { trim: false });
  }
  frame.render_widget(paragraph, text_area);

  if max_scroll > 0 {
    let mut scrollbar_state = ScrollbarState::new(content_height)
      .viewport_content_length(viewport)
      .position(scroll);
    let scrollbar_area = Rect {
      x: inner.x + inner.width.saturating_sub(1),
      y: inner.y,
      width: 1,
      height: inner.height,
    };
    frame.render_stateful_widget(
      Scrollbar::default()
        .orientation(ScrollbarOrientation::VerticalRight)
        .begin_symbol(None)
        .end_symbol(None),
      scrollbar_area,
      &mut scrollbar_state,
    );
  }
}

fn panel_scroll_limit(inner: Rect, lines: Vec<Line<'_>>, wrap: bool) -> u16 {
  let text_area = panel_text_area(inner);
  let viewport = text_area.height as usize;
  let content_height = visual_line_count(&lines, text_area.width as usize, wrap);
  content_height.saturating_sub(viewport) as u16
}

fn health_panel_areas(area: Rect) -> [Rect; 4] {
  let outer = Layout::default()
    .direction(Direction::Vertical)
    .constraints([Constraint::Percentage(48), Constraint::Percentage(52)])
    .split(area);
  let top = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Fill(1), Constraint::Fill(1)])
    .split(outer[0]);
  let bot = Layout::default()
    .direction(Direction::Horizontal)
    .constraints([Constraint::Fill(1), Constraint::Fill(1)])
    .split(outer[1]);
  [top[0], top[1], bot[0], bot[1]]
}

fn panel_inner(area: Rect) -> Rect {
  Block::default().borders(Borders::ALL).inner(area)
}

fn panel_text_area(inner: Rect) -> Rect {
  Rect {
    x: inner.x,
    y: inner.y,
    width: inner.width.saturating_sub(2),
    height: inner.height,
  }
}

fn panel_text_width(inner: Rect) -> u16 {
  panel_text_area(inner).width
}

fn visual_line_count(lines: &[Line<'_>], width: usize, wrap: bool) -> usize {
  if width == 0 {
    return lines.len();
  }
  lines
    .iter()
    .map(|line| {
      let len = line.width();
      if wrap {
        len.max(1).div_ceil(width)
      } else {
        1
      }
    })
    .sum()
}
