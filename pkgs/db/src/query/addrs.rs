//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The addresses domain: precomputed per-address stats, keyset-paginated
//! history, the current UTXO set, and the rich list. Balances and counts are
//! read straight from the precomputed `address_stats` — never summed at query
//! time — so a whale address costs a single-row read.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};

use super::PgRead;
use crate::error::{HyDbError, HyDbResult};
use crate::types::{
  HyDbAddress, HyDbCursor, HyDbHash, HyDbHeight, HyDbOutpoint, HyDbPage, HyDbTxType,
};
use crate::util::escape_like;

/// Precomputed per-address summary — a single-row read even for whales.
#[derive(Debug, Clone)]
pub struct HyDbQrAddrSummary {
  pub address: HyDbAddress,
  pub balance: i64,
  pub total_received: i64,
  pub total_sent: i64,
  pub tx_count: i32,
  pub utxo_count: i32,
  pub first_seen_height: Option<HyDbHeight>,
  pub last_seen_height: Option<HyDbHeight>,
}

/// One transaction in an address's history, with the address-relative net
/// value and direction computed for display.
#[derive(Debug, Clone)]
pub struct HyDbQrAddrTx {
  pub txid: HyDbHash,
  pub block_height: HyDbHeight,
  pub block_pos: i32,
  pub block_time: i64,
  pub version: i32,
  pub tx_type: HyDbTxType,
  pub size: i32,
  pub value_out: i64,
  pub net_value: i64,
  pub direction: HyDbTxDir,
  pub fee: Option<i64>,
}

/// A rich-list entry (address ranked by balance).
#[derive(Debug, Clone)]
pub struct HyDbQrRichEntry {
  pub address: HyDbAddress,
  pub balance: i64,
  pub total_received: i64,
  pub total_sent: i64,
  pub tx_count: i32,
}

/// Chain-wide supply accumulation.
#[derive(Debug, Clone, Copy)]
pub struct HyDbQrSupply {
  pub total: i64,
  pub included_supply: i64,
}

/// Whether a transaction moved value into or out of an address.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum HyDbTxDir {
  In,
  Out,
}

/// One unspent output owned by an address. `outpoint` packs its txid/vout.
#[derive(Debug, Clone)]
pub struct HyDbQrUtxo {
  pub outpoint: HyDbOutpoint,
  pub value: i64,
  pub block_height: HyDbHeight,
  pub script_pubkey: Vec<u8>,
}

/// Query handle for address data.
#[async_trait]
pub trait HyDbAddrIdx: Send + Sync {
  /// Precomputed summary for an address, or `None` if never seen.
  async fn summary(&self, address: HyDbAddress) -> HyDbResult<Option<HyDbQrAddrSummary>>;

  /// Whether an address has ever been seen.
  async fn exists(&self, address: HyDbAddress) -> HyDbResult<bool>;

  /// Newest-first, keyset-paginated transaction history with direction.
  async fn history(
    &self,
    address: HyDbAddress,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrAddrTx>>;

  /// The address's current unspent outputs, newest-first.
  async fn utxos(
    &self,
    address: HyDbAddress,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrUtxo>>;

  /// Rich list: addresses ranked by balance, highest first.
  async fn rich_list(
    &self,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrRichEntry>>;

  /// Number of addresses holding a positive balance.
  async fn bal_count(&self) -> HyDbResult<u64>;

  /// Chain-wide total and included supply.
  async fn supply(&self) -> HyDbResult<HyDbQrSupply>;

  /// Addresses matching a prefix, most-recently-active first.
  async fn search_prefix(&self, prefix: String, limit: u32) -> HyDbResult<Vec<HyDbAddress>>;
}

#[derive(Debug)]
pub(crate) struct PgAddrIdx {
  pub(crate) pg: PgRead,
}

#[async_trait]
impl HyDbAddrIdx for PgAddrIdx {
  async fn summary(&self, address: HyDbAddress) -> HyDbResult<Option<HyDbQrAddrSummary>> {
    let row = sqlx::query!(
      r#"SELECT balance, total_received,
                (total_received - balance)::bigint AS "total_sent!",
                tx_count, utxo_count, first_seen_height, last_seen_height
           FROM address_stats WHERE address = $1"#,
      address.0
    )
    .fetch_optional(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("address summary", e))?;

    Ok(row.map(|r| {
      HyDbQrAddrSummary {
        address,
        balance: r.balance,
        total_received: r.total_received,
        total_sent: r.total_sent,
        tx_count: r.tx_count,
        utxo_count: r.utxo_count,
        first_seen_height: r
          .first_seen_height
          .and_then(|h| HyDbHeight::try_from_i32(h).ok()),
        last_seen_height: r
          .last_seen_height
          .and_then(|h| HyDbHeight::try_from_i32(h).ok()),
      }
    }))
  }

  async fn exists(&self, address: HyDbAddress) -> HyDbResult<bool> {
    let row = sqlx::query_scalar!(
      "SELECT 1 FROM address_stats WHERE address = $1 LIMIT 1",
      address.0
    )
    .fetch_optional(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("address exists", e))?;
    Ok(row.is_some())
  }

  async fn history(
    &self,
    address: HyDbAddress,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrAddrTx>> {
    let limit = limit.clamp(1, 200) as i64;
    let fetch = limit + 1;
    let before = match before {
      None => None,
      Some(HyDbCursor(s)) => {
        let parts: Vec<&str> = s.splitn(3, ':').collect();
        let [h, p, txid_hex] = parts.as_slice() else {
          return Err(HyDbError::Decode(format!("invalid address-tx cursor: {s}")));
        };
        let h: i32 = h
          .parse()
          .map_err(|_| HyDbError::Decode(format!("invalid address-tx cursor height: {s}")))?;
        let p: i32 = p
          .parse()
          .map_err(|_| HyDbError::Decode(format!("invalid address-tx cursor pos: {s}")))?;
        Some((h, p, hex::decode(txid_hex)?))
      }
    };

    // Keyset the address's tx index, then decorate each with block time and the
    // address-relative received/spent totals (LATERAL sums), giving net value
    // and direction.
    type Row = (
      Vec<u8>,
      i32,
      i32,
      i64,
      i32,
      HyDbTxType,
      i32,
      i64,
      i64,
      String,
      Option<i64>,
    );
    let rows: Vec<Row> = match &before {
      Some((h, p, txid)) => sqlx::query!(
        r#"WITH page AS (
             SELECT txid, block_height, block_pos FROM address_txs
              WHERE address = $1 AND (block_height, block_pos, txid) < ($2, $3, $4)
              ORDER BY block_height DESC, block_pos DESC, txid DESC LIMIT $5
           )  -- fetches limit+1; the extra row is only the has-more signal
           SELECT t.txid, p.block_height, p.block_pos, b.time::bigint AS "block_time!",
                  t.version, t.tx_type AS "tx_type: HyDbTxType", t.size, t.value_out,
                  (COALESCE(received.value, 0) - COALESCE(sent.value, 0))::bigint AS "net_value!",
                  CASE WHEN (COALESCE(received.value, 0) - COALESCE(sent.value, 0)) > 0
                       THEN 'in' ELSE 'out' END AS "direction!",
                  t.fee
             FROM page AS p
             JOIN transactions AS t ON t.block_height = p.block_height AND t.txid = p.txid
             JOIN blocks AS b ON b.height = p.block_height
             LEFT JOIN LATERAL (
               SELECT COALESCE(SUM(o.value), 0)::bigint AS value FROM tx_outputs AS o
                WHERE o.block_height = p.block_height AND o.txid = p.txid AND o.address = $1
             ) AS received ON TRUE
             LEFT JOIN LATERAL (
               SELECT COALESCE(SUM(i.prev_value), 0)::bigint AS value FROM tx_inputs AS i
                WHERE i.block_height = p.block_height AND i.txid = p.txid
                  AND i.prev_address = $1 AND i.prev_value IS NOT NULL
             ) AS sent ON TRUE
            ORDER BY p.block_height DESC, p.block_pos DESC, p.txid DESC"#,
        address.0,
        h,
        p,
        &txid[..],
        fetch
      )
      .fetch_all(&self.pg.heavy)
      .await
      .map_err(|e| HyDbError::pg("address history", e))?
      .into_iter()
      .map(|r| {
        (
          r.txid,
          r.block_height,
          r.block_pos,
          r.block_time,
          r.version,
          r.tx_type,
          r.size,
          r.value_out,
          r.net_value,
          r.direction,
          r.fee,
        )
      })
      .collect(),
      None => sqlx::query!(
        r#"WITH page AS (
             SELECT txid, block_height, block_pos FROM address_txs
              WHERE address = $1
              ORDER BY block_height DESC, block_pos DESC, txid DESC LIMIT $2
           )  -- fetches limit+1; the extra row is only the has-more signal
           SELECT t.txid, p.block_height, p.block_pos, b.time::bigint AS "block_time!",
                  t.version, t.tx_type AS "tx_type: HyDbTxType", t.size, t.value_out,
                  (COALESCE(received.value, 0) - COALESCE(sent.value, 0))::bigint AS "net_value!",
                  CASE WHEN (COALESCE(received.value, 0) - COALESCE(sent.value, 0)) > 0
                       THEN 'in' ELSE 'out' END AS "direction!",
                  t.fee
             FROM page AS p
             JOIN transactions AS t ON t.block_height = p.block_height AND t.txid = p.txid
             JOIN blocks AS b ON b.height = p.block_height
             LEFT JOIN LATERAL (
               SELECT COALESCE(SUM(o.value), 0)::bigint AS value FROM tx_outputs AS o
                WHERE o.block_height = p.block_height AND o.txid = p.txid AND o.address = $1
             ) AS received ON TRUE
             LEFT JOIN LATERAL (
               SELECT COALESCE(SUM(i.prev_value), 0)::bigint AS value FROM tx_inputs AS i
                WHERE i.block_height = p.block_height AND i.txid = p.txid
                  AND i.prev_address = $1 AND i.prev_value IS NOT NULL
             ) AS sent ON TRUE
            ORDER BY p.block_height DESC, p.block_pos DESC, p.txid DESC"#,
        address.0,
        fetch
      )
      .fetch_all(&self.pg.heavy)
      .await
      .map_err(|e| HyDbError::pg("address history", e))?
      .into_iter()
      .map(|r| {
        (
          r.txid,
          r.block_height,
          r.block_pos,
          r.block_time,
          r.version,
          r.tx_type,
          r.size,
          r.value_out,
          r.net_value,
          r.direction,
          r.fee,
        )
      })
      .collect(),
    };

    let has_more = rows.len() as i64 > limit;
    let mut items = Vec::with_capacity(rows.len().min(limit as usize));
    for r in rows.iter().take(limit as usize) {
      items.push(HyDbQrAddrTx {
        txid: HyDbHash::from_slice(&r.0)?,
        block_height: HyDbHeight::try_from_i32(r.1)?,
        block_pos: r.2,
        block_time: r.3,
        version: r.4,
        tx_type: r.5,
        size: r.6,
        value_out: r.7,
        net_value: r.8,
        direction: if r.9 == "in" {
          HyDbTxDir::In
        } else {
          HyDbTxDir::Out
        },
        fee: r.10,
      });
    }
    let next = items.last().filter(|_| has_more).map(|t| {
      HyDbCursor(format!(
        "{}:{}:{}",
        t.block_height.0,
        t.block_pos,
        hex::encode(t.txid.as_bytes())
      ))
    });
    Ok(HyDbPage::new(items, next))
  }

  async fn utxos(
    &self,
    address: HyDbAddress,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrUtxo>> {
    let limit = limit.clamp(1, 500) as i64;
    let fetch = limit + 1;
    let before = match before {
      None => None,
      Some(HyDbCursor(s)) => {
        let (h, op_hex) = s
          .split_once(':')
          .ok_or_else(|| HyDbError::Decode(format!("invalid utxo cursor: {s}")))?;
        let h: i32 = h
          .parse()
          .map_err(|_| HyDbError::Decode(format!("invalid utxo cursor height: {s}")))?;
        Some((h, hex::decode(op_hex)?))
      }
    };

    type Row = (Vec<u8>, i64, i32, Vec<u8>);
    let rows: Vec<Row> = match &before {
      Some((h, op)) => sqlx::query!(
        r#"SELECT outpoint, value, block_height, script_pubkey FROM utxo_set
            WHERE address = $1 AND (block_height, outpoint) < ($2, $3)
            ORDER BY block_height DESC, outpoint DESC LIMIT $4"#,
        address.0,
        h,
        &op[..],
        fetch
      )
      .fetch_all(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("address utxos", e))?
      .into_iter()
      .map(|r| (r.outpoint, r.value, r.block_height, r.script_pubkey))
      .collect(),
      None => sqlx::query!(
        r#"SELECT outpoint, value, block_height, script_pubkey FROM utxo_set
            WHERE address = $1
            ORDER BY block_height DESC, outpoint DESC LIMIT $2"#,
        address.0,
        fetch
      )
      .fetch_all(&self.pg.query)
      .await
      .map_err(|e| HyDbError::pg("address utxos", e))?
      .into_iter()
      .map(|r| (r.outpoint, r.value, r.block_height, r.script_pubkey))
      .collect(),
    };

    let has_more = rows.len() as i64 > limit;
    let mut items = Vec::with_capacity(rows.len().min(limit as usize));
    for r in rows.iter().take(limit as usize) {
      items.push(HyDbQrUtxo {
        outpoint: HyDbOutpoint::from_packed(&r.0)?,
        value: r.1,
        block_height: HyDbHeight::try_from_i32(r.2)?,
        script_pubkey: r.3.clone(),
      });
    }
    let next = items.last().filter(|_| has_more).map(|u| {
      HyDbCursor(format!(
        "{}:{}",
        u.block_height.0,
        hex::encode(u.outpoint.packed())
      ))
    });
    Ok(HyDbPage::new(items, next))
  }

  async fn rich_list(
    &self,
    limit: u32,
    before: Option<HyDbCursor>,
  ) -> HyDbResult<HyDbPage<HyDbQrRichEntry>> {
    let limit = limit.clamp(1, 500) as i64;
    let fetch = limit + 1;
    let before = match before {
      None => None,
      Some(HyDbCursor(s)) => {
        let (bal, addr) = s
          .split_once(':')
          .ok_or_else(|| HyDbError::Decode(format!("invalid rich-list cursor: {s}")))?;
        let bal: i64 = bal
          .parse()
          .map_err(|_| HyDbError::Decode(format!("invalid rich-list cursor balance: {s}")))?;
        Some((bal, addr.to_owned()))
      }
    };

    // Keyset on (balance DESC, address ASC). The equal-balance boundary needs
    // address > cursor; the strictly-lower boundary is any address.
    type Row = (String, i64, i64, i64, i32);
    let rows: Vec<Row> = match &before {
      Some((bal, addr)) => sqlx::query!(
        r#"SELECT address, balance, total_received,
                  (total_received - balance)::bigint AS "total_sent!", tx_count
             FROM address_stats
            WHERE balance > 0
              AND (balance < $1 OR (balance = $1 AND address > $2))
            ORDER BY balance DESC, address ASC LIMIT $3"#,
        bal,
        addr,
        fetch
      )
      .fetch_all(&self.pg.heavy)
      .await
      .map_err(|e| HyDbError::pg("rich list", e))?
      .into_iter()
      .map(|r| {
        (
          r.address,
          r.balance,
          r.total_received,
          r.total_sent,
          r.tx_count,
        )
      })
      .collect(),
      None => sqlx::query!(
        r#"SELECT address, balance, total_received,
                  (total_received - balance)::bigint AS "total_sent!", tx_count
             FROM address_stats
            WHERE balance > 0
            ORDER BY balance DESC, address ASC LIMIT $1"#,
        fetch
      )
      .fetch_all(&self.pg.heavy)
      .await
      .map_err(|e| HyDbError::pg("rich list", e))?
      .into_iter()
      .map(|r| {
        (
          r.address,
          r.balance,
          r.total_received,
          r.total_sent,
          r.tx_count,
        )
      })
      .collect(),
    };

    let has_more = rows.len() as i64 > limit;
    let items: Vec<HyDbQrRichEntry> = rows
      .into_iter()
      .take(limit as usize)
      .map(|r| HyDbQrRichEntry {
        address: HyDbAddress(r.0),
        balance: r.1,
        total_received: r.2,
        total_sent: r.3,
        tx_count: r.4,
      })
      .collect();
    let next = items
      .last()
      .filter(|_| has_more)
      .map(|e| HyDbCursor(format!("{}:{}", e.balance, e.address.0)));
    Ok(HyDbPage::new(items, next))
  }

  async fn bal_count(&self) -> HyDbResult<u64> {
    let count =
      sqlx::query_scalar!(r#"SELECT COUNT(*) AS "count!" FROM address_stats WHERE balance > 0"#)
        .fetch_one(&self.pg.query)
        .await
        .map_err(|e| HyDbError::pg("positive-balance address count", e))?;
    Ok(count.max(0) as u64)
  }

  async fn supply(&self) -> HyDbResult<HyDbQrSupply> {
    let row =
      sqlx::query!("SELECT total, included_supply FROM supply_summary WHERE singleton = TRUE")
        .fetch_one(&self.pg.query)
        .await
        .map_err(|e| HyDbError::pg("supply summary", e))?;
    Ok(HyDbQrSupply {
      total: row.total,
      included_supply: row.included_supply,
    })
  }

  async fn search_prefix(&self, prefix: String, limit: u32) -> HyDbResult<Vec<HyDbAddress>> {
    let limit = limit.clamp(1, 100) as i64;
    // Addresses are case-sensitive (base58), so no lowercasing — but the
    // prefix is LIKE-escaped so a bare `%` cannot match everything. Backed by
    // the text_pattern_ops index on address_stats(address).
    let pattern = format!("{}%", escape_like(&prefix));
    let rows = sqlx::query_scalar!(
      r#"SELECT address AS "address!" FROM address_stats
          WHERE address LIKE $1 ESCAPE '\'
          ORDER BY last_seen_height DESC NULLS LAST, tx_count DESC, address ASC LIMIT $2"#,
      pattern,
      limit
    )
    .fetch_all(&self.pg.query)
    .await
    .map_err(|e| HyDbError::pg("search addresses by prefix", e))?;
    Ok(rows.into_iter().map(HyDbAddress).collect())
  }
}
