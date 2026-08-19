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
    (Some(h), Some(tip)) if tip > h && bps > 0 => fmt_eta_secs((tip - h) as u64 / bps),
    (Some(h), Some(tip)) if tip <= h => "synced".to_string(),
    _ => "--".to_string(),
  }
}

/// ETA from remaining *transactions* rather than remaining blocks.
///
/// Blocks are not units of equal work: mainnet density rises roughly sixfold
/// toward the tip, so a block-rate ETA is optimistic by about that factor
/// early in a sync. The transaction rate is the block rate times the recently
/// observed transactions-per-block, both of which the daemon reports.
///
/// `None` when the daemon has not reported a transaction estimate, so the
/// caller can fall back to the block figure rather than show nothing.
pub(crate) fn calc_eta_by_transactions(
  indexed: Option<u64>,
  estimated_total: Option<u64>,
  txs_per_block: Option<f64>,
  bps: u64,
) -> Option<String> {
  let (indexed, total, density) = (indexed?, estimated_total?, txs_per_block?);
  if total <= indexed {
    return Some("synced".to_string());
  }
  let txs_per_sec = bps as f64 * density;
  // `is_finite` first, so a NaN density is rejected rather than compared.
  if !txs_per_sec.is_finite() || txs_per_sec <= 0.0 {
    return None;
  }
  Some(fmt_eta_secs(
    ((total - indexed) as f64 / txs_per_sec) as u64,
  ))
}

fn fmt_eta_secs(secs: u64) -> String {
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

#[cfg(test)]
mod tests {
  use super::{calc_eta, calc_eta_by_transactions};

  /// F2: the transaction-weighted ETA must be the pessimistic (correct) one
  /// while the dense part of the chain is still ahead of the indexer.
  #[test]
  fn transaction_eta_outweighs_the_block_eta() {
    // 907k of 2.52M blocks done, 4.4M of 70.5M transactions, 40 blk/s at the
    // ~3 tx/block density of the region being indexed.
    let block_eta = calc_eta(Some(907_000), Some(2_520_000), 40);
    let tx_eta = calc_eta_by_transactions(Some(4_400_000), Some(70_556_609), Some(3.0), 40);
    assert_eq!(block_eta, "11h 12m");
    assert_eq!(tx_eta.as_deref(), Some("6d 9h"));

    // Caught up.
    assert_eq!(
      calc_eta_by_transactions(Some(70_556_609), Some(70_556_609), Some(3.0), 40).as_deref(),
      Some("synced")
    );
    // No daemon-side estimate, or standing still: fall back to the block ETA.
    assert_eq!(calc_eta_by_transactions(None, Some(1), Some(3.0), 40), None);
    assert_eq!(
      calc_eta_by_transactions(Some(1), Some(2), Some(3.0), 0),
      None
    );
  }
}
