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

/// How a column's raw JSON value should be turned into display text.
///
/// Rendering is driven by what a column *means*, not by the JSON type of the
/// value underneath it -- a `size` in bytes and a `height` are both JSON
/// numbers, but they read very differently. `Plain` is the fallback and
/// mirrors the old type-driven behavior.
#[derive(Clone, Copy, PartialEq, Eq)]
enum ColFmt {
  /// Type-driven fallback: strings as-is, numbers thousands-separated,
  /// bools as yes/no, arrays joined.
  Plain,
  /// An integer count or height -- thousands-separated, no decoration.
  Count,
  /// A hex hash/txid -- truncated with a middle ellipsis so both the
  /// recognizable prefix and suffix survive.
  Hash,
  /// A point in time -- either an RFC3339 string or a unix-epoch number,
  /// rendered as `YYYY-MM-DD HH:MM:SS` UTC.
  Timestamp,
  /// A byte count, rendered as `418 kB` / `1.20 MB` / `2.60 GB`.
  Bytes,
  /// A duff (1e-8 DASH) amount, rendered as a DASH quantity with thousands
  /// separators on the integer part.
  Amount,
}

struct ColDef {
  header: &'static str,
  /// Lookup path into the row's JSON object. Dot-separated for nested
  /// fields (e.g. `services.coreP2p`).
  key: &'static str,
  width: u16,
  fmt: ColFmt,
  /// At most one column per table should set this. It gets `Constraint::Min`
  /// instead of `Constraint::Length` so it absorbs any slack left in the
  /// pane instead of the fixed-width columns collapsing to fit `Fill`
  /// weights.
  flex: bool,
}

const BLOCK_COLS: &[ColDef] = &[
  ColDef {
    header: "Height",
    key: "blockHeight",
    width: 11,
    fmt: ColFmt::Count,
    flex: false,
  },
  ColDef {
    header: "Hash",
    key: "blockHash",
    width: 20,
    fmt: ColFmt::Hash,
    flex: true,
  },
  ColDef {
    header: "Time",
    key: "blockTime",
    width: 19,
    fmt: ColFmt::Timestamp,
    flex: false,
  },
  ColDef {
    header: "Txs",
    key: "txCount",
    width: 6,
    fmt: ColFmt::Count,
    flex: false,
  },
  ColDef {
    header: "Size",
    key: "blockSize",
    width: 9,
    fmt: ColFmt::Bytes,
    flex: false,
  },
];

const TX_COLS: &[ColDef] = &[
  ColDef {
    header: "TxID",
    key: "txHash",
    width: 20,
    fmt: ColFmt::Hash,
    flex: true,
  },
  ColDef {
    header: "Block",
    key: "blockHeight",
    width: 10,
    fmt: ColFmt::Count,
    flex: false,
  },
  ColDef {
    header: "Type",
    key: "txType",
    // Longest current tx-type tag is `provider_update_registrar` (26 chars).
    width: 26,
    fmt: ColFmt::Plain,
    flex: false,
  },
  ColDef {
    header: "Size",
    key: "txSize",
    width: 9,
    fmt: ColFmt::Bytes,
    flex: false,
  },
  ColDef {
    header: "Value",
    key: "valueOut",
    width: 20,
    fmt: ColFmt::Amount,
    flex: false,
  },
  ColDef {
    header: "Ins",
    key: "inputCount",
    width: 5,
    fmt: ColFmt::Count,
    flex: false,
  },
  ColDef {
    header: "Outs",
    key: "outputCount",
    width: 5,
    fmt: ColFmt::Count,
    flex: false,
  },
];

const VALIDATOR_COLS: &[ColDef] = &[
  ColDef {
    header: "ProTxHash",
    key: "proTxHash",
    width: 20,
    fmt: ColFmt::Hash,
    flex: true,
  },
  ColDef {
    header: "Type",
    key: "type",
    width: 8,
    fmt: ColFmt::Plain,
    flex: false,
  },
  ColDef {
    header: "Status",
    key: "status",
    width: 12,
    fmt: ColFmt::Plain,
    flex: false,
  },
  ColDef {
    header: "Core P2P",
    key: "services.coreP2p",
    width: 22,
    fmt: ColFmt::Plain,
    flex: false,
  },
  ColDef {
    header: "Platform P2P",
    key: "services.platformP2p",
    width: 22,
    fmt: ColFmt::Plain,
    flex: false,
  },
  ColDef {
    header: "Platform HTTPS",
    key: "services.platformHttps",
    width: 22,
    fmt: ColFmt::Plain,
    flex: false,
  },
  ColDef {
    header: "Payout",
    key: "addresses.payout",
    width: 20,
    fmt: ColFmt::Plain,
    flex: false,
  },
  ColDef {
    header: "Registered",
    key: "registeredHeight",
    width: 10,
    fmt: ColFmt::Count,
    flex: false,
  },
  ColDef {
    header: "Updated",
    key: "lastUpdatedHeight",
    width: 10,
    fmt: ColFmt::Count,
    flex: false,
  },
];

const METRICS_COLS: &[ColDef] = &[
  ColDef {
    header: "Height",
    key: "height",
    width: 11,
    fmt: ColFmt::Count,
    flex: false,
  },
  ColDef {
    header: "Txs",
    key: "txCount",
    width: 12,
    fmt: ColFmt::Count,
    flex: false,
  },
  ColDef {
    header: "UTXOs",
    key: "utxosSpendable",
    width: 12,
    fmt: ColFmt::Count,
    flex: false,
  },
  ColDef {
    header: "Unspendable",
    key: "utxosUnspendable",
    width: 12,
    fmt: ColFmt::Count,
    flex: false,
  },
  ColDef {
    header: "Chain Size",
    key: "blockchainSize",
    width: 10,
    fmt: ColFmt::Bytes,
    flex: false,
  },
  ColDef {
    header: "Supply",
    key: "circulatingSupply",
    width: 20,
    fmt: ColFmt::Amount,
    flex: true,
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
      cols
        .iter()
        .any(|c: &ColDef| format_value(row, c).to_lowercase().contains(&filter_lower))
    })
    .take(inner.height.saturating_sub(1) as usize)
    .map(|row: &serde_json::Value| {
      Row::new(
        cols
          .iter()
          .map(|c: &ColDef| Cell::from(Span::styled(display_value(row, c), theme::normal())))
          .collect::<Vec<Cell>>(),
      )
    })
    .collect();

  let widths: Vec<Constraint> = cols
    .iter()
    .map(|c| {
      if c.flex {
        Constraint::Min(c.width)
      } else {
        Constraint::Length(c.width)
      }
    })
    .collect();
  let table = Table::new(data_rows, widths).header(header);
  frame.render_widget(table, inner);
}

/// Look up a (possibly dot-separated, nested) field path in a JSON object.
fn json_get_path<'a>(row: &'a serde_json::Value, path: &str) -> Option<&'a serde_json::Value> {
  let mut cur = row;
  for part in path.split('.') {
    cur = cur.get(part)?;
  }
  Some(cur)
}

/// Full (untruncated) formatted text for a cell, per its column's format.
/// Used both for display (subject to further truncation) and for filter
/// matching, where the untruncated text is what should be searched.
fn format_value(row: &serde_json::Value, col: &ColDef) -> String {
  match col.fmt {
    ColFmt::Plain => json_cell_str(row, col.key),
    ColFmt::Count => count_cell(row, col.key),
    ColFmt::Hash => match json_get_path(row, col.key).and_then(serde_json::Value::as_str) {
      Some(s) => s.to_owned(),
      None => "-".to_owned(),
    },
    ColFmt::Timestamp => timestamp_cell(row, col.key),
    ColFmt::Bytes => bytes_cell(row, col.key),
    ColFmt::Amount => amount_cell(row, col.key),
  }
}

/// Display text for a cell: the full formatted value, truncated to fit the
/// column so no cell is ever wider than its column.
fn display_value(row: &serde_json::Value, col: &ColDef) -> String {
  let full = format_value(row, col);
  let width = col.width as usize;
  if col.fmt == ColFmt::Hash {
    truncate_middle(&full, width)
  } else {
    truncate_tail(&full, width)
  }
}

fn count_cell(row: &serde_json::Value, key: &str) -> String {
  match json_get_path(row, key).and_then(json_as_i64) {
    Some(n) => theme::fmt_num(n),
    None => "-".to_owned(),
  }
}

fn timestamp_cell(row: &serde_json::Value, key: &str) -> String {
  match json_get_path(row, key) {
    Some(serde_json::Value::String(s)) => fmt_iso_timestamp(s),
    Some(serde_json::Value::Number(n)) => number_as_i64(n)
      .map(fmt_epoch_utc)
      .unwrap_or_else(|| "-".to_owned()),
    _ => "-".to_owned(),
  }
}

fn number_as_i64(n: &serde_json::Number) -> Option<i64> {
  n.as_i64()
    .or_else(|| n.as_u64().and_then(|u| i64::try_from(u).ok()))
    .or_else(|| n.as_f64().map(|f| f as i64))
}

fn bytes_cell(row: &serde_json::Value, key: &str) -> String {
  match json_get_path(row, key).and_then(serde_json::Value::as_f64) {
    Some(n) => fmt_bytes(n),
    None => "-".to_owned(),
  }
}

fn amount_cell(row: &serde_json::Value, key: &str) -> String {
  match json_get_path(row, key).and_then(json_as_i64) {
    Some(n) => fmt_amount(n),
    None => "-".to_owned(),
  }
}

fn json_as_i64(v: &serde_json::Value) -> Option<i64> {
  v.as_i64()
    .or_else(|| v.as_u64().and_then(|u| i64::try_from(u).ok()))
    .or_else(|| v.as_f64().map(|f| f as i64))
}

/// Reformat an RFC3339 timestamp (`2019-02-19T01:08:46Z`, optionally with
/// fractional seconds or a numeric offset) as `YYYY-MM-DD HH:MM:SS`.
fn fmt_iso_timestamp(s: &str) -> String {
  let Some((date, rest)) = s.split_once('T') else {
    return s.to_owned();
  };
  let time = rest.trim_end_matches('Z');
  let time = time.split('.').next().unwrap_or(time);
  let time = time.split('+').next().unwrap_or(time);
  format!("{date} {time}")
}

/// Convert unix-epoch seconds (UTC) to `YYYY-MM-DD HH:MM:SS`.
fn fmt_epoch_utc(secs: i64) -> String {
  let days = secs.div_euclid(86400);
  let rem = secs.rem_euclid(86400);
  let (y, m, d) = civil_from_days(days);
  let hh = rem / 3600;
  let mm = (rem % 3600) / 60;
  let ss = rem % 60;
  format!("{y:04}-{m:02}-{d:02} {hh:02}:{mm:02}:{ss:02}")
}

/// Howard Hinnant's `civil_from_days`: days since the unix epoch to a
/// proleptic-Gregorian (year, month, day), valid for any `i64` day count.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
  let z = z + 719_468;
  let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
  let doe = (z - era * 146_097) as u64;
  let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
  let y = yoe as i64 + era * 400;
  let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
  let mp = (5 * doy + 2) / 153;
  let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
  let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
  let y = if m <= 2 { y + 1 } else { y };
  (y, m, d)
}

/// Format a byte count using decimal (1000-based) units: `418 B`,
/// `1.20 kB`, `2.60 MB`. Bumps to the next unit rather than ever showing a
/// value that would round to `1000.xx` at the current one.
fn fmt_bytes(bytes: f64) -> String {
  const UNITS: [&str; 5] = ["B", "kB", "MB", "GB", "TB"];
  let sign = if bytes < 0.0 { "-" } else { "" };
  let mut value = bytes.abs();
  let mut idx = 0usize;
  while value >= 999.995 && idx < UNITS.len() - 1 {
    value /= 1000.0;
    idx += 1;
  }
  if idx == 0 {
    format!("{sign}{value:.0} {}", UNITS[idx])
  } else {
    format!("{sign}{value:.2} {}", UNITS[idx])
  }
}

/// Format a duff amount (1e-8 DASH) as a DASH quantity, thousands-separated
/// on the integer part, e.g. `160001600` duffs -> `1.60001600`.
fn fmt_amount(duffs: i64) -> String {
  let sign = if duffs < 0 { "-" } else { "" };
  let abs = duffs.unsigned_abs();
  let int_part = abs / 100_000_000;
  let frac_part = abs % 100_000_000;
  format!("{sign}{}.{:08}", theme::fmt_num(int_part), frac_part)
}

/// Truncate to `width` characters, keeping the head and appending a
/// trailing ellipsis when the text does not fit.
fn truncate_tail(s: &str, width: usize) -> String {
  if s.chars().count() <= width {
    return s.to_owned();
  }
  if width == 0 {
    return String::new();
  }
  if width == 1 {
    return "…".to_owned();
  }
  let head: String = s.chars().take(width - 1).collect();
  format!("{head}…")
}

/// Truncate to `width` characters with a middle ellipsis, keeping both the
/// head and tail -- for hashes, where a human matches on both ends.
fn truncate_middle(s: &str, width: usize) -> String {
  let chars: Vec<char> = s.chars().collect();
  if chars.len() <= width {
    return s.to_owned();
  }
  if width == 0 {
    return String::new();
  }
  if width == 1 {
    return "…".to_owned();
  }
  // Bias toward the head: a hash's leading digits carry more of what a
  // human recognizes it by than the last few, so give the tail roughly a
  // third of the budget and the head the rest (e.g. `0000abcd…7f3e`).
  let budget = width - 1;
  let tail = (budget / 3).max(1).min(budget);
  let head = budget - tail;
  let head_str: String = chars[..head].iter().collect();
  let tail_str: String = chars[chars.len() - tail..].iter().collect();
  format!("{head_str}…{tail_str}")
}

fn json_cell_str(row: &serde_json::Value, key: &str) -> String {
  match json_get_path(row, key) {
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

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::{
    display_value, fmt_amount, fmt_bytes, fmt_epoch_utc, fmt_iso_timestamp, truncate_middle,
    truncate_tail, ColDef, ColFmt,
  };

  #[test]
  fn truncate_middle_leaves_short_hashes_untouched() {
    assert_eq!(truncate_middle("abcd", 20), "abcd");
  }

  #[test]
  fn truncate_middle_leaves_exact_fit_untouched() {
    let s = "0123456789012345678a"; // exactly 20 chars
    assert_eq!(s.chars().count(), 20);
    assert_eq!(truncate_middle(s, 20), s);
  }

  #[test]
  fn truncate_middle_shortens_long_hashes_keeping_both_ends() {
    let hash = "0000abcd00000000000000000000000000000000000000000000000000007f3e";
    let out = truncate_middle(hash, 13);
    assert_eq!(out.chars().count(), 13);
    assert!(out.starts_with("0000abcd"));
    assert!(out.ends_with("7f3e"));
    assert!(out.contains('…'));
  }

  #[test]
  fn truncate_tail_shortens_long_plain_text() {
    let out = truncate_tail("provider_update_registrar_extra", 10);
    assert_eq!(out.chars().count(), 10);
    assert!(out.ends_with('…'));
  }

  #[test]
  fn no_formatted_cell_exceeds_its_column_width() {
    let row = json!({
      "blockHeight": 1_024_538,
      "blockHash": "00000000000000312ea805b96cb3528e1cc0f6c42dc3ce3de660774fea9dc515",
      "blockTime": "2019-02-19T01:08:46Z",
      "txCount": 6,
      "blockSize": 8_303_593_613i64,
    });
    let cols: &[ColDef] = &[
      ColDef {
        header: "Height",
        key: "blockHeight",
        width: 11,
        fmt: ColFmt::Count,
        flex: false,
      },
      ColDef {
        header: "Hash",
        key: "blockHash",
        width: 20,
        fmt: ColFmt::Hash,
        flex: true,
      },
      ColDef {
        header: "Time",
        key: "blockTime",
        width: 19,
        fmt: ColFmt::Timestamp,
        flex: false,
      },
      ColDef {
        header: "Size",
        key: "blockSize",
        width: 9,
        fmt: ColFmt::Bytes,
        flex: false,
      },
    ];
    for col in cols {
      let out = display_value(&row, col);
      assert!(
        out.chars().count() <= col.width as usize,
        "column {} rendered {out:?} ({} chars) wider than width {}",
        col.header,
        out.chars().count(),
        col.width
      );
    }
  }

  #[test]
  fn bytes_format_below_one_kb() {
    assert_eq!(fmt_bytes(0.0), "0 B");
    assert_eq!(fmt_bytes(999.0), "999 B");
  }

  #[test]
  fn bytes_format_crosses_kb_boundary() {
    assert_eq!(fmt_bytes(1_000.0), "1.00 kB");
    // Would round to "1000.00 kB" at 2 decimals -- must bump to MB instead.
    assert_eq!(fmt_bytes(999_999.0), "1.00 MB");
  }

  #[test]
  fn bytes_format_crosses_mb_and_gb_boundary() {
    assert_eq!(fmt_bytes(1_000_000.0), "1.00 MB");
    assert_eq!(fmt_bytes(1_000_000_000.0), "1.00 GB");
    assert_eq!(fmt_bytes(8_303_593_613.0), "8.30 GB");
  }

  #[test]
  fn epoch_zero_is_the_unix_epoch() {
    assert_eq!(fmt_epoch_utc(0), "1970-01-01 00:00:00");
  }

  #[test]
  fn epoch_matches_known_block_time() {
    // 2019-02-19T01:08:46Z, cross-checked against the real REST payload.
    assert_eq!(fmt_epoch_utc(1_550_538_526), "2019-02-19 01:08:46");
  }

  #[test]
  fn iso_timestamp_is_reformatted_for_readability() {
    assert_eq!(
      fmt_iso_timestamp("2019-02-19T01:08:46Z"),
      "2019-02-19 01:08:46"
    );
  }

  #[test]
  fn amount_formats_duffs_as_dash_with_separators() {
    assert_eq!(fmt_amount(160_001_600), "1.60001600");
    assert_eq!(fmt_amount(864_574_503_066_067), "8,645,745.03066067");
    assert_eq!(fmt_amount(0), "0.00000000");
  }

  #[test]
  fn height_numbers_keep_thousands_separators() {
    let row = json!({ "height": 1_024_538 });
    let col = ColDef {
      header: "Height",
      key: "height",
      width: 11,
      fmt: ColFmt::Count,
      flex: false,
    };
    assert_eq!(display_value(&row, &col), "1,024,538");
  }
}
