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

use async_trait::async_trait;

use super::PgRead;
use crate::error::{HyDbError, HyDbResult};
use crate::util::{escape_like, hex_prefix_pattern};

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
  async fn prefix(&self, query: String, limit: u32) -> HyDbResult<Vec<HyDbQrSearchHit>>;
}

#[derive(Debug)]
pub(crate) struct PgSearch {
  pub(crate) pg: PgRead,
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
    let each = i64::from(limit.div_ceil(pools).max(1));
    let hex_pattern = hex_prefix_pattern(&query);
    let addr_pattern = format!("{}%", escape_like(&query));

    // Recency ordering is kept here: block hashes share a long run of leading
    // zeros, so a short prefix matches most of the table and the planner walks
    // `blocks_pkey` backwards and stops at the limit (~0.4 ms on mainnet).
    let blocks_q = async {
      if !want_hex {
        return Ok::<Vec<String>, sqlx::Error>(Vec::new());
      }
      sqlx::query_scalar!(
        r#"SELECT display_hex(hash) AS "hex!" FROM blocks
            WHERE display_hex(hash) LIKE $1 ESCAPE '\' ORDER BY height DESC LIMIT $2"#,
        hex_pattern,
        each
      )
      .fetch_all(&self.pg.query)
      .await
    };

    // No ORDER BY, deliberately. `LIMIT n` over the display_hex prefix index
    // early-terminates after n matches; sorting by recency first forces every
    // match to be enumerated and heap-fetched. Measured on the retained
    // mainnet database (8.8M txs), a 4-character prefix costs 39-48 ms ordered
    // against 0.4 ms unordered, and since txids are random, "most recent among
    // prefix matches" is arbitrary UX anyway.
    let txs_q = async {
      if !want_hex {
        return Ok::<Vec<String>, sqlx::Error>(Vec::new());
      }
      sqlx::query_scalar!(
        r#"SELECT display_hex(txid) AS "hex!" FROM transactions
            WHERE display_hex(txid) LIKE $1 ESCAPE '\' LIMIT $2"#,
        hex_pattern,
        each
      )
      .fetch_all(&self.pg.query)
      .await
    };

    // Same reasoning, and worse: the activity ordering is not covered by the
    // prefix index, so it heap-fetched every match — 75 ms against 0.09 ms for
    // a 4-character mainnet prefix. The index is text_pattern_ops, so the
    // unordered scan already emits addresses in byte order.
    let addrs_q = async {
      if !want_addr {
        return Ok::<Vec<String>, sqlx::Error>(Vec::new());
      }
      sqlx::query_scalar!(
        r#"SELECT address FROM address_stats
            WHERE address LIKE $1 ESCAPE '\' LIMIT $2"#,
        addr_pattern,
        each
      )
      .fetch_all(&self.pg.query)
      .await
    };

    // Validators are a few thousand rows; the ordering is free.
    let validators_q = async {
      if !want_hex {
        return Ok::<Vec<String>, sqlx::Error>(Vec::new());
      }
      sqlx::query_scalar!(
        r#"SELECT display_hex(protx_hash) AS "hex!" FROM validators
            WHERE display_hex(protx_hash) LIKE $1 ESCAPE '\'
            ORDER BY last_updated_height DESC, protx_hash ASC LIMIT $2"#,
        hex_pattern,
        each
      )
      .fetch_all(&self.pg.query)
      .await
    };

    let (blocks, txs, addrs, validators) = tokio::try_join!(blocks_q, txs_q, addrs_q, validators_q)
      .map_err(|e| HyDbError::pg("prefix search", e))?;

    // Merge with the most specific kinds first, so trimming to the total
    // limit sheds block hits (the broadest kind) before anything else.
    let mut hits: Vec<HyDbQrSearchHit> = Vec::new();
    hits.extend(txs.into_iter().map(|id| HyDbQrSearchHit {
      kind: HyDbSearchKind::Tx,
      id,
    }));
    hits.extend(addrs.into_iter().map(|id| HyDbQrSearchHit {
      kind: HyDbSearchKind::Address,
      id,
    }));
    hits.extend(validators.into_iter().map(|id| HyDbQrSearchHit {
      kind: HyDbSearchKind::Validator,
      id,
    }));
    hits.extend(blocks.into_iter().map(|id| HyDbQrSearchHit {
      kind: HyDbSearchKind::Block,
      id,
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
