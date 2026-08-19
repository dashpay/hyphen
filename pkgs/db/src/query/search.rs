//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The search domain: prefix search across blocks, txs, addresses, validators.
//!
//! THE SEARCH CONTRACT (single source of truth, mirrored in the schema): hash
//! identifiers are matched and returned in DISPLAY order — the byte-reverse
//! of the stored internal bytes, i.e. exactly the hex an explorer UI renders
//! and a user copies. Matching happens against the `display_hex()` SQL
//! function (backed by expression indexes), the prefix is lowercased and
//! `LIKE`-escaped here, and result `id`s are display-order hex too, so a
//! prefix copied from the UI always finds its row. Addresses are matched
//! case-sensitively (base58), escaped only.
//!
//! Probes are dispatched by query shape: a lookup only touches the pools that
//! could possibly match it. Classification yields a CANDIDATE SET, never a
//! precedence winner — hex and base58 overlap on `[1-9a-f]`, and mainnet p2sh
//! addresses start with `7` (testnet `8`/`9`), all hex digits, so a query can
//! be both a hash prefix and an address prefix and both arms must run.
//!
//! Each probed kind gets its own quota of the requested limit, divided by the
//! number of pools actually reached rather than by a fixed four, and the
//! probes run concurrently — a flood of block matches can never crowd out the
//! transaction the user actually searched for.
//!
//! # Relationship to the REST search endpoint (task C6)
//!
//! [`HyDbSearch::prefix`] still has NO production caller: `rest/search.rs`
//! drives the four per-kind `search_prefix` probes itself. The db-side half of
//! the consolidation is done here — `prefix` no longer holds its own copies of
//! those four queries, it calls the same domain methods REST calls, so the two
//! paths cannot drift again (they already had: see the validator ordering note
//! below).
//!
//! What is left is NOT a mechanical swap of one call for four, and the
//! difference is not a rounding error. Before REST can be pointed at this
//! function, it needs from it:
//!
//! - **Result payloads.** REST answers with per-hit detail this trait cannot
//!   express: validator `status`, block `canonical`/`orphan` labels, tx
//!   `mined`/`coinbase` tags. [`HyDbQrSearchHit`] carries `kind` + `id` only,
//!   which is exactly why the REST validator arm re-fetches each hit
//!   (`validators().get()`) after its prefix probe.
//! - **Arms with no Postgres pool behind them.** The numeric block-height
//!   lookup (a primary-key hit that short-circuits the fan-out), the exact
//!   64-hex both-endian probes, the mempool overlay for txs, the orphan-block
//!   fallback, and governance (served from in-memory live state, and matching
//!   free text rather than a prefix) all live above this seam.
//! - **A decision about trim direction, which is a real behavioral choice.**
//!   This function merges most-specific-first and sheds BLOCKS when the total
//!   overshoots; REST appends blocks FIRST and stops at ten, so it sheds
//!   transactions, addresses and governance instead. And REST asks every kind
//!   for the full ten rather than a quota, so a flooding prefix can take all
//!   ten slots. Consolidating changes which results a user sees; it is not a
//!   refactor, and it is not this crate's call to make silently.

use async_trait::async_trait;

use super::addrs::{HyDbAddrIdx as _, PgAddrIdx};
use super::blocks::{HyDbBlockIdx as _, PgBlockIdx};
use super::txs::{HyDbTxIdx as _, PgTxIdx};
use super::validators::{HyDbVlIdx as _, PgVlIdx};
use super::PgRead;
use crate::error::HyDbResult;
use crate::types::HyDbHash;

/// A single search result. `id` is the entity's natural identifier: a
/// DISPLAY-order hex hash for blocks/txs/validators, or the address string.
#[derive(Debug, Clone)]
pub struct HyDbQrSearchHit {
  pub kind: HyDbSearchKind,
  pub id: String,
}

/// What kind of entity a search hit refers to.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyDbSearchKind {
  Block,
  Tx,
  Address,
  Validator,
}

/// Query handle for prefix search.
#[async_trait]
pub trait HyDbSearch: Send + Sync {
  /// Resolve a user-entered prefix to matching entities across all domains,
  /// capped at `limit` hits total (each kind receives its own quota).
  ///
  /// Length policy is the caller's: this returns an empty vector for a query
  /// too short to be selective, rather than rejecting it. REST's four-character
  /// minimum (and its 400) is a transport decision made above this seam.
  ///
  /// # Errors
  ///
  /// Propagates query failure from any probed domain.
  async fn prefix(&self, query: String, limit: u32) -> HyDbResult<Vec<HyDbQrSearchHit>>;
}

#[derive(Debug)]
pub(crate) struct PgSearch {
  pub(crate) pg: PgRead,
}

/// A hash in DISPLAY order — the byte-reverse of the stored bytes, which is
/// what a UI renders and a user copies (see the search contract above). The
/// per-kind probes match on the SQL `display_hex()` of the same value, so this
/// is the Rust side of one convention, not a second one.
fn display_hex(hash: &HyDbHash) -> String {
  hash
    .as_bytes()
    .iter()
    .rev()
    .fold(String::with_capacity(64), |mut out, b| {
      use core::fmt::Write as _;
      let _ = write!(out, "{b:02x}");
      out
    })
}

#[async_trait]
impl HyDbSearch for PgSearch {
  async fn prefix(&self, query: String, limit: u32) -> HyDbResult<Vec<HyDbQrSearchHit>> {
    let limit = limit.clamp(1, 50);
    // Dispatch by shape. The two charsets overlap, so this is a union: a
    // `7`-prefixed p2sh address is all hex digits and must still reach the
    // address pool.
    let want_hex = is_hex_charset(&query);
    let want_addr = is_base58_charset(&query);
    // Three hex-backed kinds plus addresses. A query in neither charset (a
    // stray LIKE metacharacter, punctuation, non-ASCII) can match nothing here
    // and is answered without touching the database at all.
    let pools = u32::from(want_hex) * 3 + u32::from(want_addr);
    if pools == 0 {
      return Ok(Vec::new());
    }
    // Per-kind quota: no kind can starve another. Divided by the pools
    // actually probed rather than a fixed four — once dispatch narrows a query
    // to one kind, that kind should get the whole limit. The remainder (if the
    // quotas overshoot) is trimmed at the end, blocks last.
    let each = limit.div_ceil(pools).max(1);

    // Each arm is the domain's own `search_prefix`, not a second copy of its
    // query. The copies had already drifted: this function used to order
    // validators by `(last_updated_height DESC, protx_hash ASC)` while
    // `validators::search_prefix` tie-breaks on `registered_height DESC`
    // between them, so the two paths returned different validators for the
    // same prefix. Sharing the probe is what makes "both paths order
    // identically" a property of the code rather than a claim about it —
    // including the pieces of reasoning that live in those methods (why blocks
    // are ordered by recency and txs and addresses deliberately are not).
    let blocks_q = async {
      if !want_hex {
        return Ok(Vec::new());
      }
      PgBlockIdx {
        pg: self.pg.clone(),
      }
      .search_prefix(query.clone(), each)
      .await
    };
    let txs_q = async {
      if !want_hex {
        return Ok(Vec::new());
      }
      PgTxIdx {
        pg: self.pg.clone(),
      }
      .search_prefix(query.clone(), each)
      .await
    };
    let addrs_q = async {
      if !want_addr {
        return Ok(Vec::new());
      }
      PgAddrIdx {
        pg: self.pg.clone(),
      }
      .search_prefix(query.clone(), each)
      .await
    };
    let validators_q = async {
      if !want_hex {
        return Ok(Vec::new());
      }
      PgVlIdx {
        pg: self.pg.clone(),
      }
      .search_prefix(query.clone(), each)
      .await
    };

    // Still concurrent: four pooled round trips on the latency-sensitive pool.
    let (blocks, txs, addrs, validators) =
      tokio::try_join!(blocks_q, txs_q, addrs_q, validators_q)?;

    // Merge with the most specific kinds first, so trimming to the total
    // limit sheds block hits (the broadest kind) before anything else.
    let mut hits: Vec<HyDbQrSearchHit> = Vec::new();
    hits.extend(txs.iter().map(|txid| HyDbQrSearchHit {
      kind: HyDbSearchKind::Tx,
      id: display_hex(txid),
    }));
    hits.extend(addrs.into_iter().map(|addr| HyDbQrSearchHit {
      kind: HyDbSearchKind::Address,
      id: addr.0,
    }));
    hits.extend(validators.iter().map(|protx| HyDbQrSearchHit {
      kind: HyDbSearchKind::Validator,
      id: display_hex(protx),
    }));
    hits.extend(blocks.iter().map(|block| HyDbQrSearchHit {
      kind: HyDbSearchKind::Block,
      id: display_hex(&block.hash),
    }));
    hits.truncate(limit as usize);
    Ok(hits)
  }
}

/// Whether every character is a hex digit — the blocks/transactions/validators
/// arm. Overlaps `is_base58_charset` on `[1-9a-f]`.
fn is_hex_charset(query: &str) -> bool {
  !query.is_empty() && query.bytes().all(|b| b.is_ascii_hexdigit())
}

/// Whether every character is in the base58 alphabet — the address arm.
/// Deliberately not "base58 and not hex": mainnet p2sh addresses start with
/// `7` and testnet with `8`/`9`, which are hex digits, and dropping them from
/// the address arm would make p2sh addresses unsearchable.
fn is_base58_charset(query: &str) -> bool {
  !query.is_empty()
    && query
      .chars()
      .all(|ch| matches!(ch, '1'..='9' | 'A'..='H' | 'J'..='N' | 'P'..='Z' | 'a'..='k' | 'm'..='z'))
}

#[cfg(test)]
mod tests {
  use super::{is_base58_charset, is_hex_charset};

  /// The number of pools `prefix` would probe for a query, mirroring the
  /// dispatch arithmetic: three hex-backed kinds plus addresses.
  fn pools(query: &str) -> u32 {
    u32::from(is_hex_charset(query)) * 3 + u32::from(is_base58_charset(query))
  }

  #[test]
  fn hex_and_base58_overlap_reaches_both_arms() {
    // Mainnet p2sh starts with `7`, testnet with `8`/`9` — all hex digits.
    // Precedence would drop these from the address pool entirely.
    for prefix in ["7abc", "8abc", "9abc"] {
      assert!(is_base58_charset(prefix), "{prefix} must reach addresses");
      assert!(is_hex_charset(prefix), "{prefix} is also a hash prefix");
      assert_eq!(pools(prefix), 4, "{prefix} is ambiguous, so nothing is cut");
    }
  }

  #[test]
  fn unambiguous_shapes_narrow_the_fan_out() {
    // `X` (mainnet p2pkh) and `y` (testnet) are outside the hex charset.
    assert_eq!(pools("Xabc"), 1);
    assert_eq!(pools("yabc"), 1);
    // `0` is hex but not base58: hash pools only.
    assert_eq!(pools("dead0"), 3);
    // `l`, `I`, `O` and `0` are excluded from base58; `g`..`z` from hex.
    assert!(!is_base58_charset("l33t"));
    assert!(!is_hex_charset("zzzz"));
  }

  #[test]
  fn queries_in_neither_charset_touch_no_pool() {
    // A bare LIKE metacharacter cannot appear in hex or base58, so dispatch
    // answers it without a query. The escaping that also defeats it is
    // covered by `util::escapes_like_metacharacters`.
    for query in ["%", "_", "", "hello world", "café"] {
      assert_eq!(pools(query), 0, "{query:?} can match nothing");
    }
  }
}
