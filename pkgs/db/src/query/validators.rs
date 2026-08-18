//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The validators (masternode / evonode) domain. Current state comes from
//! `validators`; a validator's update history is the chain of special
//! transactions in `sptx_payloads`; bucket counts come precomputed from
//! `validator_counts`.
//!
//! `list`/`get` decode straight into [`HyDbQrValidator`] with a runtime `query_as`
//! and [`sqlx::FromRow`]: columns are matched to fields by name, and the domain
//! newtypes carry their own `Decode`, so there is no intermediate row type. That
//! trades the `query!` macro's compile-time check for a name-keyed mapping with
//! no positional fragility; the round-trip is covered by an integration test.

use async_trait::async_trait;
use serde_json::Value;

use super::PgRead;
use crate::error::{HyDbError, HyDbResult};
use crate::types::{
  HyDbAddress, HyDbCursor, HyDbHash, HyDbHeight, HyDbPage, HyDbTxType, HyDbVlType,
};
use crate::util::hex_prefix_pattern;

/// A validator's current registry state. Decodes directly from a validator row
/// via [`sqlx::FromRow`]; field names match the aliased columns in [`Self::COLS`].
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct HyDbQrValidator {
  pub protx_hash: HyDbHash,
  pub collateral_hash: HyDbHash,
  pub collateral_index: i32,
  pub collateral_address: Option<HyDbAddress>,
  pub owner_address: Option<HyDbAddress>,
  pub voting_address: Option<HyDbAddress>,
  pub operator_pubkey: Option<Vec<u8>>,
  pub addr_core_p2p: Vec<String>,
  pub addr_platform_p2p: Vec<String>,
  pub addr_platform_https: Vec<String>,
  pub payout_address: Option<HyDbAddress>,
  pub operator_payout_address: Option<HyDbAddress>,
  pub operator_reward: i32,
  pub validator_type: HyDbVlType,
  pub status: String,
  pub registered_height: HyDbHeight,
  pub last_updated_height: HyDbHeight,
}

impl HyDbQrValidator {
  /// The column projection that decodes into this struct: each column is aliased
  /// to the matching field so `FromRow` maps them by name. Shared by `list`/`get`.
  const COLS: &str = "\
    protx_hash, collateral_hash, collateral_index, collateral_address, \
    owner_address, voting_address, operator_pubkey, \
    COALESCE(addr_core_p2p, '{}') AS addr_core_p2p, \
    COALESCE(addr_platform_p2p, '{}') AS addr_platform_p2p, \
    COALESCE(addr_platform_https, '{}') AS addr_platform_https, \
    payout_address, operator_payout_address, operator_reward, \
    validator_type, status, registered_height, last_updated_height";
}

/// A precomputed count of validators bucketed by (status, type).
#[derive(Debug, Clone)]
pub struct HyDbQrVlCounts {
  pub status: String,
  pub validator_type: HyDbVlType,
  pub total: i64,
}

/// One entry in a validator's update history — the special transaction that
/// registered or modified it.
#[derive(Debug, Clone)]
pub struct HyDbQrVlHistory {
  pub txid: HyDbHash,
  pub tx_type: HyDbTxType,
  pub block_height: HyDbHeight,
  pub payload: Value,
}

/// Query handle for validator data.
#[async_trait]
pub trait HyDbVlIdx: Send + Sync {
  /// Page of validators, optionally filtered by status and/or public type,
  /// newest-registered first.
  async fn list(
    &self,
    status: Option<String>,
    validator_type: Option<HyDbVlType>,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrValidator>>;

  /// A validator by protx_hash.
  async fn get(&self, protx_hash: HyDbHash) -> HyDbResult<Option<HyDbQrValidator>>;

  /// The validator's update history (its special transactions), oldest-first.
  async fn history(&self, protx_hash: HyDbHash) -> HyDbResult<Vec<HyDbQrVlHistory>>;

  /// Precomputed counts by (status, type).
  async fn counts(&self) -> HyDbResult<Vec<HyDbQrVlCounts>>;

  /// protx_hashes matching a forward-hex prefix.
  async fn search_prefix(&self, prefix: String, limit: u32) -> HyDbResult<Vec<HyDbHash>>;
}

#[derive(Debug)]
pub(crate) struct PgVlIdx {
  pub(crate) pg: PgRead,
}

#[async_trait]
impl HyDbVlIdx for PgVlIdx {
  async fn list(
    &self,
    status: Option<String>,
    validator_type: Option<HyDbVlType>,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrValidator>> {
    let limit = limit.clamp(1, 500) as i64;
    let (before_height, before_hash) = match before {
      None => (None, None),
      Some(HyDbCursor(s)) => {
        let (h, hash_hex) = s
          .split_once(':')
          .ok_or_else(|| HyDbError::Decode(format!("invalid validator cursor: {s}")))?;
        let h: i32 = h
          .parse()
          .map_err(|_| HyDbError::Decode(format!("invalid validator cursor height: {s}")))?;
        (Some(h), Some(hex::decode(hash_hex)?))
      }
    };

    // Runtime `query_as` lets the keyset predicate collapse into one nullable
    // guard (`$3 IS NULL` when there is no cursor) instead of two query arms.
    let sql = format!(
      "SELECT {} FROM validators \
        WHERE ($1::text IS NULL OR status = $1) \
          AND ($2::validator_type IS NULL OR validator_type = $2) \
          AND ($3::int IS NULL OR (registered_height, protx_hash) < ($3, $4)) \
        ORDER BY registered_height DESC, protx_hash DESC LIMIT $5",
      HyDbQrValidator::COLS
    );
    // Fetch limit+1: the extra row is only the has-more signal, so an exactly
    // full terminal page never hands out a cursor to an empty page.
    let mut items: Vec<HyDbQrValidator> = sqlx::query_as::<_, HyDbQrValidator>(&sql)
      .bind(status)
      .bind(validator_type)
      .bind(before_height)
      .bind(before_hash)
      .bind(limit + 1)
      .fetch_all(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("list validators", e))?;
    let has_more = items.len() as i64 > limit;
    items.truncate(limit as usize);

    let next = items.last().filter(|_| has_more).map(|v| {
      HyDbCursor(format!(
        "{}:{}",
        v.registered_height.0,
        hex::encode(v.protx_hash.as_bytes())
      ))
    });
    Ok(HyDbPage::new(items, next))
  }

  async fn get(&self, protx_hash: HyDbHash) -> HyDbResult<Option<HyDbQrValidator>> {
    let sql = format!(
      "SELECT {} FROM validators WHERE protx_hash = $1",
      HyDbQrValidator::COLS
    );
    sqlx::query_as::<_, HyDbQrValidator>(&sql)
      .bind(protx_hash.0.to_vec())
      .fetch_optional(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("validator by protx_hash", e))
  }

  async fn history(&self, protx_hash: HyDbHash) -> HyDbResult<Vec<HyDbQrVlHistory>> {
    let rows = sqlx::query!(
      r#"SELECT txid, tx_type AS "tx_type: HyDbTxType", block_height, payload
           FROM sptx_payloads
          WHERE protx_hash = $1
          ORDER BY block_height ASC"#,
      &protx_hash.0[..]
    )
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("validator history", e))?;

    let mut out = Vec::with_capacity(rows.len());
    for r in rows {
      out.push(HyDbQrVlHistory {
        txid: HyDbHash::from_slice(&r.txid)?,
        tx_type: r.tx_type,
        block_height: HyDbHeight::try_from_i32(r.block_height)?,
        payload: r.payload,
      });
    }
    Ok(out)
  }

  async fn counts(&self) -> HyDbResult<Vec<HyDbQrVlCounts>> {
    let rows = sqlx::query!(
      r#"SELECT status, validator_type AS "validator_type: HyDbVlType", total
           FROM validator_counts ORDER BY status, validator_type"#
    )
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("validator counts", e))?;
    Ok(
      rows
        .into_iter()
        .map(|r| HyDbQrVlCounts {
          status: r.status,
          validator_type: r.validator_type,
          total: r.total,
        })
        .collect(),
    )
  }

  async fn search_prefix(&self, prefix: String, limit: u32) -> HyDbResult<Vec<HyDbHash>> {
    let limit = limit.clamp(1, 100) as i64;
    // DISPLAY-order hex match (see the search contract in the schema):
    // lowercased and LIKE-escaped by the shared helper.
    let pattern = hex_prefix_pattern(&prefix);
    let rows = sqlx::query_scalar!(
      r#"SELECT protx_hash FROM validators
          WHERE display_hex(protx_hash) LIKE $1 ESCAPE '\'
          ORDER BY last_updated_height DESC, registered_height DESC, protx_hash ASC
          LIMIT $2"#,
      pattern,
      limit
    )
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("search validators by prefix", e))?;
    rows.iter().map(|h| HyDbHash::from_slice(h)).collect()
  }
}
