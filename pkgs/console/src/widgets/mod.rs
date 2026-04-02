//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Reusable console view components and shared rendering helpers.

pub mod filter_bar;
pub mod index_tree;
pub mod latest_block;
pub mod store_tree;
pub mod table_browser;

/// Format an ETA from indexed height, absolute tip, and observed blocks per
/// second.
pub(crate) fn calc_eta(indexed_height: Option<i32>, abs_tip: Option<i32>, bps: u64) -> String {
  match (indexed_height, abs_tip) {
    (Some(h), Some(tip)) if tip > h && bps > 0 => {
      let remaining = (tip - h) as u64;
      let secs = remaining / bps;
      if secs >= 86400 {
        let d = secs / 86400;
        let h = (secs % 86400) / 3600;
        format!("{d}d {h}h")
      } else if secs >= 3600 {
        let h = secs / 3600;
        let m = (secs % 3600) / 60;
        format!("{h}h {m}m")
      } else if secs >= 60 {
        let m = secs / 60;
        let s = secs % 60;
        format!("{m}m {s}s")
      } else {
        format!("{secs}s")
      }
    }
    (Some(h), Some(tip)) if tip <= h => "synced".to_string(),
    _ => "--".to_string(),
  }
}
