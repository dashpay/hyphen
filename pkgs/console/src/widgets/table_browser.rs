//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Tabular REST browse widget.

use ratatui::{
  layout::{Constraint, Rect},
  text::{Line, Span},
  widgets::{Block, Borders, Cell, Paragraph, Row, Table},
  Frame,
};

use crate::event::BrowseIndex;
use crate::model::{AppState, Focus};
use crate::theme;

struct ColDef {
  header: &'static str,
  key: &'static str,
  width: u16,
}

const BLOCK_COLS: &[ColDef] = &[
  ColDef {
    header: "Height",
    key: "height",
    width: 10,
  },
  ColDef {
    header: "Hash",
    key: "hash",
    width: 20,
  },
  ColDef {
    header: "Time",
    key: "time",
    width: 12,
  },
  ColDef {
    header: "Txs",
    key: "txCount",
    width: 6,
  },
  ColDef {
    header: "Size",
    key: "size",
    width: 8,
  },
];

const TX_COLS: &[ColDef] = &[
  ColDef {
    header: "TxID",
    key: "txid",
    width: 20,
  },
  ColDef {
    header: "Block",
    key: "blockHeight",
    width: 10,
  },
  ColDef {
    header: "Type",
    key: "txType",
    width: 10,
  },
  ColDef {
    header: "Size",
    key: "size",
    width: 8,
  },
  ColDef {
    header: "Value",
    key: "valueOut",
    width: 14,
  },
  ColDef {
    header: "Ins",
    key: "inputCount",
    width: 5,
  },
  ColDef {
    header: "Outs",
    key: "outputCount",
    width: 5,
  },
];

const VALIDATOR_COLS: &[ColDef] = &[
  ColDef {
    header: "ProTxHash",
    key: "proTxHash",
    width: 20,
  },
  ColDef {
    header: "Type",
    key: "type",
    width: 8,
  },
  ColDef {
    header: "Status",
    key: "status",
    width: 10,
  },
  ColDef {
    header: "Core P2P",
    key: "service",
    width: 22,
  },
  ColDef {
    header: "Platform P2P",
    key: "platformP2P",
    width: 22,
  },
  ColDef {
    header: "Platform HTTPS",
    key: "platformHttps",
    width: 22,
  },
  ColDef {
    header: "Payout",
    key: "payoutAddress",
    width: 18,
  },
  ColDef {
    header: "Registered",
    key: "registeredHeight",
    width: 10,
  },
  ColDef {
    header: "Updated",
    key: "lastUpdatedHeight",
    width: 10,
  },
];

const METRICS_COLS: &[ColDef] = &[
  ColDef {
    header: "Height",
    key: "height",
    width: 10,
  },
  ColDef {
    header: "Txs",
    key: "txCount",
    width: 10,
  },
  ColDef {
    header: "UTXOs",
    key: "utxosSpendable",
    width: 12,
  },
  ColDef {
    header: "Unspendable",
    key: "utxosUnspendable",
    width: 12,
  },
  ColDef {
    header: "Chain Size",
    key: "blockchainSize",
    width: 14,
  },
  ColDef {
    header: "Supply",
    key: "circulatingSupply",
    width: 16,
  },
];

fn cols_for(index: BrowseIndex) -> &'static [ColDef] {
  match index {
    BrowseIndex::Blocks => BLOCK_COLS,
    BrowseIndex::Transactions => TX_COLS,
    BrowseIndex::Validators => VALIDATOR_COLS,
    BrowseIndex::Metrics => METRICS_COLS,
  }
}

/// Render the active browse table.
pub fn draw(frame: &mut Frame<'_>, area: Rect, state: &AppState) {
  let browse_idx = match state.browse_index {
    Some(idx) => idx,
    None => {
      let is_sub_index = state.focus == Focus::Main && state.index_tree_cursor >= 4;
      let msg = if is_sub_index {
        " No REST endpoint for this sub-index"
      } else {
        " Select an index and press Enter"
      };
      let block = Block::default()
        .title(Span::styled(" Browser ", theme::title()))
        .borders(Borders::ALL)
        .border_style(theme::muted())
        .style(theme::normal());
      let inner = block.inner(area);
      frame.render_widget(block, area);
      frame.render_widget(
        Paragraph::new(Line::from(Span::styled(msg, theme::muted()))),
        inner,
      );
      return;
    }
  };

  let suffix = if state.browse_next_cursor.is_some() {
    "more available"
  } else {
    "last page"
  };
  let title = match state.browse_as_of_height {
    Some(as_of_height) => format!(
      " {} -- Page {} ({suffix}, asOf {}) ",
      browse_idx.label(),
      state.browse_page + 1,
      as_of_height,
    ),
    None => format!(
      " {} -- Page {} ({suffix}) ",
      browse_idx.label(),
      state.browse_page + 1,
    ),
  };

  let block = Block::default()
    .title(Span::styled(title, theme::title()))
    .borders(Borders::ALL)
    .border_style(
      if state.focus == Focus::Main && state.browse_index.is_some() {
        theme::title()
      } else {
        theme::muted()
      },
    )
    .style(theme::normal());

  let inner = block.inner(area);
  frame.render_widget(block, area);

  if state.rest_pending {
    frame.render_widget(
      Paragraph::new(Line::from(Span::styled(" Loading...", theme::syncing()))),
      inner,
    );
    return;
  }

  let cols = cols_for(browse_idx);
  let header = Row::new(
    cols
      .iter()
      .map(|c| Cell::from(Span::styled(c.header, theme::table_header())))
      .collect::<Vec<_>>(),
  );

  let filter_lower = state.browse_filter.to_lowercase();
  let data_rows: Vec<Row> = state
    .browse_rows
    .iter()
    .filter(|row: &&serde_json::Value| {
      if filter_lower.is_empty() {
        return true;
      }
      cols.iter().any(|c: &ColDef| {
        json_cell_str(row, c.key)
          .to_lowercase()
          .contains(&filter_lower)
      })
    })
    .take(inner.height.saturating_sub(1) as usize)
    .map(|row: &serde_json::Value| {
      Row::new(
        cols
          .iter()
          .map(|c: &ColDef| Cell::from(Span::styled(json_cell_str(row, c.key), theme::normal())))
          .collect::<Vec<Cell>>(),
      )
    })
    .collect();

  let widths: Vec<Constraint> = cols.iter().map(|c| Constraint::Fill(c.width)).collect();
  let table = Table::new(data_rows, widths).header(header);
  frame.render_widget(table, inner);
}

fn json_cell_str(row: &serde_json::Value, key: &str) -> String {
  match row.get(key) {
    Some(v) => match v {
      serde_json::Value::String(s) => s.clone(),
      serde_json::Value::Bool(b) => if *b { "yes" } else { "no" }.to_string(),
      serde_json::Value::Null => "-".to_string(),
      serde_json::Value::Number(n) => fmt_json_number(n),
      serde_json::Value::Array(arr) => arr
        .iter()
        .filter_map(|v| v.as_str())
        .collect::<Vec<_>>()
        .join(", "),
      other => other.to_string(),
    },
    None => "-".to_string(),
  }
}

fn fmt_json_number(n: &serde_json::Number) -> String {
  if let Some(i) = n.as_i64() {
    return theme::fmt_num(i);
  }
  if let Some(u) = n.as_u64() {
    return theme::fmt_num(u);
  }
  if let Some(f) = n.as_f64() {
    let int_part = f.trunc() as i64;
    let frac = f.fract();
    if frac == 0.0 {
      return theme::fmt_num(int_part);
    }
    let int_str = theme::fmt_num(int_part);
    let decimals = ((frac.abs() * 100.0).round() as u64) % 100;
    return format!("{int_str}.{decimals:02}");
  }
  n.to_string()
}
