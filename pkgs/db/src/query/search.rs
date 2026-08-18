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
//! Each entity kind gets its own quota of the requested limit and the four
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
    // Per-kind quota: no kind can starve another. The remainder (if the
    // quotas overshoot) is trimmed at the end, blocks last.
    let each = i64::from(limit.div_ceil(4).max(1));
    let hex_pattern = hex_prefix_pattern(&query);
    let addr_pattern = format!("{}%", escape_like(&query));

    let blocks_q = sqlx::query_scalar!(
      r#"SELECT display_hex(hash) AS "hex!" FROM blocks
          WHERE display_hex(hash) LIKE $1 ESCAPE '\' ORDER BY height DESC LIMIT $2"#,
      hex_pattern,
      each
    )
    .fetch_all(&self.pg.query);

    let txs_q = sqlx::query_scalar!(
      r#"SELECT display_hex(txid) AS "hex!" FROM transactions
          WHERE display_hex(txid) LIKE $1 ESCAPE '\'
          ORDER BY block_height DESC, block_pos DESC, txid ASC LIMIT $2"#,
      hex_pattern,
      each
    )
    .fetch_all(&self.pg.query);

    let addrs_q = sqlx::query_scalar!(
      r#"SELECT address FROM address_stats
          WHERE address LIKE $1 ESCAPE '\'
          ORDER BY last_seen_height DESC NULLS LAST, tx_count DESC, address ASC LIMIT $2"#,
      addr_pattern,
      each
    )
    .fetch_all(&self.pg.query);

    let validators_q = sqlx::query_scalar!(
      r#"SELECT display_hex(protx_hash) AS "hex!" FROM validators
          WHERE display_hex(protx_hash) LIKE $1 ESCAPE '\'
          ORDER BY last_updated_height DESC, protx_hash ASC LIMIT $2"#,
      hex_pattern,
      each
    )
    .fetch_all(&self.pg.query);

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
