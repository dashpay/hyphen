//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Index progress tree table.

use ratatui::{
  layout::{Constraint, Rect},
  text::{Line, Span},
  widgets::{Block, Borders, Cell, Row, Table},
  Frame,
};

use hyphend::wire::msg::{CorePipelineHealth, ForwardLanePhase, IndexSchedulerLane};

use crate::model::{AppState, Focus, OverviewSubFocus};
use crate::theme;
use crate::widgets::calc_eta;

/// Render the index progress tree.
pub fn draw(
  frame: &mut Frame<'_>,
  area: Rect,
  state: &AppState,
  index_tree: &[(&str, &str, usize, bool)],
) {
  let indices_focused =
    state.focus == Focus::Main && state.overview_sub == OverviewSubFocus::Indices;
  let title = index_title(state);
  let block = Block::default()
    .title(Span::styled(title, theme::title()))
    .borders(Borders::ALL)
    .border_style(if indices_focused {
      theme::title()
    } else {
      theme::muted()
    })
    .style(theme::normal());

  let header = Row::new(vec![
    Cell::from(Span::styled(" Name", theme::table_header())),
    Cell::from(Span::styled("Backend", theme::table_header())),
    Cell::from(Span::styled("Height", theme::table_header())),
    Cell::from(Span::styled("Status", theme::table_header())),
    Cell::from(Span::styled("ETA", theme::table_header())),
    Cell::from(Span::styled("blk/s", theme::table_header())),
  ]);

  let is_focused = state.focus == Focus::Main;
  let abs_tip = state
    .snapshot
    .as_ref()
    .and_then(|s| s.network_tip)
    .map(|tip| tip as i32);
  let table_rows: Vec<Row> = index_tree
    .iter()
    .enumerate()
    .map(|(i, &(key, name, _depth, _has_children))| {
      let stage = state
        .snapshot
        .as_ref()
        .and_then(|snap| snap.stages.iter().find(|s| s.stage == key));

      let prefix = tree_prefix(key);
      let label = format!(" {prefix}{name}");
      let enabled = stage.map(|s| s.enabled).unwrap_or(false);

      let snap = state.snapshot.as_ref();
      let is_suspended = snap.is_some_and(|s| s.suspended_stages.contains(&key.to_string()));
      let is_pausing = snap.is_some_and(|s| s.pausing_stages.contains(&key.to_string()));

      let height_at = |sp: Option<&hyphend::wire::msg::StageProgress>| {
        sp.filter(|s| s.indexed_height >= 0)
          .map(|s| theme::fmt_num(s.indexed_height as u32))
          .unwrap_or_else(|| "--".to_string())
      };

      let (height_str, status, status_style) = if is_suspended {
        (
          height_at(stage),
          "Rebuilding".to_string(),
          theme::disconnected(),
        )
      } else if is_pausing {
        (height_at(stage), "Pausing".to_string(), theme::syncing())
      } else if !enabled {
        ("--".to_string(), "Disabled".to_string(), theme::muted())
      } else if let Some(err) = stage.and_then(|s| s.error.as_ref()) {
        let display = if err.len() > 16 {
          format!("{}...", &err[..15])
        } else {
          err.clone()
        };
        ("--".to_string(), display, theme::disconnected())
      } else {
        match stage {
          Some(sp) if sp.tip_height > 0 && sp.indexed_height >= 0 => {
            let ratio = sp.indexed_height as f64 / sp.tip_height as f64;
            let gated = sp
              .gate
              .as_ref()
              .zip(abs_tip)
              .is_some_and(|(_, network_tip)| sp.tip_height < network_tip);
            let core_health = snap.map(|snapshot| snapshot.core_pipeline_diag.health);
            let stalled = key == "block" && core_health == Some(CorePipelineHealth::Stalled);
            let degraded = key == "block" && core_health == Some(CorePipelineHealth::Degraded);
            if ratio >= 1.0 {
              if gated {
                (
                  theme::fmt_num(sp.indexed_height as u32),
                  format!("Gated by {}", gate_label(sp.gate.as_deref())),
                  theme::syncing(),
                )
              } else if stalled {
                (
                  theme::fmt_num(sp.indexed_height as u32),
                  "Stalled".to_string(),
                  theme::disconnected(),
                )
              } else if degraded {
                (
                  theme::fmt_num(sp.indexed_height as u32),
                  "Degraded".to_string(),
                  theme::syncing(),
                )
              } else {
                (
                  theme::fmt_num(sp.indexed_height as u32),
                  "Done".to_string(),
                  theme::connected(),
                )
              }
            } else if stalled {
              (
                theme::fmt_num(sp.indexed_height as u32),
                format!("Stalled ({:.0}%)", ratio * 100.0),
                theme::disconnected(),
              )
            } else if degraded {
              (
                theme::fmt_num(sp.indexed_height as u32),
                format!("Degraded ({:.0}%)", ratio * 100.0),
                theme::syncing(),
              )
            } else {
              (
                theme::fmt_num(sp.indexed_height as u32),
                format!("Indexing ({:.0}%)", ratio * 100.0),
                theme::syncing(),
              )
            }
          }
          Some(sp) if sp.tip_height > 0 => {
            ("--".to_string(), "Reset".to_string(), theme::syncing())
          }
          _ => ("--".to_string(), "Waiting".to_string(), theme::muted()),
        }
      };

      let name_style = if is_focused
        && state.overview_sub == OverviewSubFocus::Indices
        && i == state.index_tree_cursor
      {
        theme::highlight()
      } else if !enabled {
        theme::muted()
      } else {
        theme::normal()
      };

      let ts = state.index_sparklines.get(key);
      let sparkline_str = ts.map(|t| t.sparkline(8)).unwrap_or_default();
      let bps = ts.and_then(|t| t.last_nonzero()).unwrap_or(0);
      let bps_str = if enabled && bps > 0 {
        format!("{bps:>5} ")
      } else {
        "    - ".to_string()
      };

      let eta = if enabled {
        let tip_for_eta = stage.and_then(|sp| {
          if sp.gate.is_some() {
            abs_tip.or(Some(sp.tip_height))
          } else {
            Some(sp.tip_height)
          }
        });
        calc_eta(stage.map(|s| s.indexed_height), tip_for_eta, bps)
      } else {
        "--".to_string()
      };

      Row::new(vec![
        Cell::from(Span::styled(label, name_style)),
        Cell::from(Span::styled("PostgreSQL", theme::muted())),
        Cell::from(Span::styled(height_str, status_style)),
        Cell::from(Span::styled(status, status_style)),
        Cell::from(Span::styled(eta, theme::muted())),
        Cell::from(Line::from(vec![
          Span::styled(bps_str, theme::normal()),
          Span::styled(sparkline_str, theme::highlight()),
        ])),
      ])
    })
    .collect();

  let table = Table::new(
    table_rows,
    [
      Constraint::Min(24),
      Constraint::Length(12),
      Constraint::Length(10),
      Constraint::Length(16),
      Constraint::Length(10),
      Constraint::Length(16),
    ],
  )
  .header(header)
  .block(block);

  frame.render_widget(table, area);
}

fn index_title(state: &AppState) -> String {
  let Some(snap) = state.snapshot.as_ref() else {
    return " Indices ".to_owned();
  };
  let diag = &snap.index_scheduler_diag;
  let Some(active_lane) = diag.active_lane else {
    return " Indices ".to_owned();
  };
  let lane = lane_label(active_lane);
  let forward_suffix = if active_lane == IndexSchedulerLane::Forward {
    forward_title_suffix(&diag.forward)
  } else {
    String::new()
  };
  if diag.contention {
    format!(" Indices [{lane}{forward_suffix} | shared] ")
  } else {
    format!(" Indices [{lane}{forward_suffix}] ")
  }
}

fn forward_title_suffix(diag: &hyphend::wire::msg::ForwardLaneDiag) -> String {
  let phase = diag
    .phase
    .and_then(|phase| (phase != ForwardLanePhase::Idle).then_some(forward_phase_label(phase)));
  match (diag.stalled, phase) {
    (true, Some(phase)) => format!(":{phase}:stalled"),
    (true, None) => ":stalled".to_owned(),
    (false, Some(phase)) => format!(":{phase}"),
    (false, None) => String::new(),
  }
}

fn tree_prefix(key: &str) -> &'static str {
  match key {
    "block" => "",
    "metrics" => "├─ ",
    "validator" => "├─ ",
    "transaction" => "└─ ",
    "script" => "   └─ ",
    "spent" => "      └─ ",
    _ => "",
  }
}

fn gate_label(gate: Option<&str>) -> &'static str {
  match gate {
    Some("block") => "Block",
    Some("transaction") => "Tx",
    Some("script") => "Script",
    Some("spent") => "Spent",
    Some("metrics") => "Metrics",
    Some("validator") => "Validators",
    _ => "Upstream",
  }
}

fn lane_label(lane: IndexSchedulerLane) -> &'static str {
  match lane {
    IndexSchedulerLane::Idle => "idle",
    IndexSchedulerLane::Forward => "forward",
    IndexSchedulerLane::TxTreeCatchUp => "tx catch-up",
    IndexSchedulerLane::MetricsCatchUp => "metrics catch-up",
    IndexSchedulerLane::ValidatorCatchUp => "validator catch-up",
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
