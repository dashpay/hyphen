//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Console color palette and composite styles.

use ratatui::style::{Color, Modifier, Style};

// ── Palette ─────────────────────────────────────────────
/// Background color used for the main canvas.
pub const BG: Color = Color::Rgb(22, 22, 30);
/// Default foreground color for readable body text.
pub const FG: Color = Color::Rgb(210, 210, 220);
/// Accent color used for primary highlights.
pub const PASTEL_PINK: Color = Color::Rgb(255, 182, 193);
/// Success accent color.
pub const PASTEL_GREEN: Color = Color::Rgb(152, 251, 152);
/// In-progress accent color.
pub const PASTEL_YELLOW: Color = Color::Rgb(255, 255, 186);
/// Informational accent color.
pub const PASTEL_BLUE: Color = Color::Rgb(173, 216, 230);
/// Secondary highlight accent color.
pub const PASTEL_LAVENDER: Color = Color::Rgb(200, 180, 255);
/// Error accent color.
pub const ERROR: Color = Color::Rgb(255, 130, 130);
/// Muted text color for secondary details.
pub const MUTED: Color = Color::Rgb(90, 90, 110);
/// Background color used for the sidebar.
pub const SIDEBAR_BG: Color = Color::Rgb(28, 28, 38);
/// Foreground color used for the selected sidebar row.
pub const SIDEBAR_SEL: Color = PASTEL_PINK;
/// Decorative dither accent color.
pub const DITHER: Color = Color::Rgb(50, 50, 65);

// ── Dither characters ───────────────────────────────────
/// Light dither glyph.
pub const DITHER_LIGHT: &str = "░";
/// Medium dither glyph.
pub const DITHER_MED: &str = "▒";
/// Heavy dither glyph.
pub const DITHER_HEAVY: &str = "▓";
/// Dot separator glyph.
pub const DOT: &str = "·";

// ── Composite styles ────────────────────────────────────
/// Title style used for primary page headings.
pub fn title() -> Style {
  Style::default()
    .fg(PASTEL_PINK)
    .add_modifier(Modifier::BOLD)
}

/// Default text style for normal foreground content.
pub fn normal() -> Style {
  Style::default().fg(FG).bg(BG)
}

/// Muted text style for secondary information.
pub fn muted() -> Style {
  Style::default().fg(MUTED).bg(BG)
}

/// Style for connected or healthy status text.
pub fn connected() -> Style {
  Style::default().fg(PASTEL_GREEN).bg(BG)
}

/// Style for disconnected or error status text.
pub fn disconnected() -> Style {
  Style::default().fg(ERROR).bg(BG)
}

/// Style for syncing or in-progress status text.
pub fn syncing() -> Style {
  Style::default().fg(PASTEL_YELLOW).bg(BG)
}

/// Base style for sidebar entries.
pub fn sidebar_item() -> Style {
  Style::default().fg(FG).bg(SIDEBAR_BG)
}

/// Highlighted style for the selected sidebar entry.
pub fn sidebar_selected() -> Style {
  Style::default()
    .fg(SIDEBAR_SEL)
    .bg(SIDEBAR_BG)
    .add_modifier(Modifier::BOLD)
}

/// Informational accent style.
pub fn info() -> Style {
  Style::default().fg(PASTEL_BLUE).bg(BG)
}

/// Highlight accent style distinct from status colors.
pub fn highlight() -> Style {
  Style::default().fg(PASTEL_LAVENDER).bg(BG)
}

/// Filled portion style for gauges.
pub fn gauge_filled() -> Style {
  Style::default().fg(BG).bg(PASTEL_GREEN)
}

/// Unfilled portion style for gauges.
pub fn gauge_unfilled() -> Style {
  Style::default().fg(MUTED).bg(BG)
}

/// Dithered background accent style.
pub fn dither_style() -> Style {
  Style::default().fg(DITHER).bg(BG)
}

/// Table header style for tabular sections.
pub fn table_header() -> Style {
  Style::default()
    .fg(PASTEL_PINK)
    .add_modifier(Modifier::BOLD)
}

/// Format a duration in seconds compactly, for a status cell that has to stay
/// narrow while a long-running task keeps counting (`9s`, `4m21s`, `1h04m`).
pub fn fmt_dur_secs(secs: u64) -> String {
  match secs {
    s if s < 60 => format!("{s}s"),
    s if s < 3600 => format!("{}m{:02}s", s / 60, s % 60),
    s => format!("{}h{:02}m", s / 3600, (s % 3600) / 60),
  }
}

/// Format an integer with thousands separators (e.g. 1,234,567).
pub fn fmt_num(n: impl std::fmt::Display) -> String {
  let s = n.to_string();
  let mut result = String::new();
  for (i, c) in s.chars().rev().enumerate() {
    if i > 0 && i % 3 == 0 {
      result.push(',');
    }
    result.push(c);
  }
  result.chars().rev().collect()
}

#[cfg(test)]
mod tests {
  use super::fmt_dur_secs;

  /// The boundaries are the whole point: the seconds and minutes fields stay
  /// zero-padded so the status column does not shift width while an operator
  /// watches a long build tick, and an hour-plus build stops showing seconds
  /// rather than growing the cell.
  #[test]
  fn durations_stay_narrow_across_the_boundaries() {
    assert_eq!(fmt_dur_secs(0), "0s");
    assert_eq!(fmt_dur_secs(9), "9s");
    assert_eq!(fmt_dur_secs(59), "59s");
    assert_eq!(fmt_dur_secs(60), "1m00s");
    assert_eq!(fmt_dur_secs(261), "4m21s");
    assert_eq!(fmt_dur_secs(3599), "59m59s");
    assert_eq!(fmt_dur_secs(3600), "1h00m");
    assert_eq!(fmt_dur_secs(3840), "1h04m");
  }
}
