//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Transaction overlay helpers that enrich indexed routes with live facts.
//!
//! Overlay modules enrich indexed responses with nullable live fields only.
//! They must not duplicate facts already exposed by the indexed payload.

use crate::rest::error::ApiError;
use crate::rest::openapi::{
  RawTxResponse, RawValueSource, SearchResponse, SearchTxResult, TxDetail, TxLive, TxLiveTags,
  TxTags,
};
use crate::rest::{AppState, LiveFeature};
use crate::utils::{self, HashEndian, HashLookup};

pub(crate) async fn indexed_live(state: &AppState, txid: &[u8]) -> Result<TxLive, ApiError> {
  let mempool = if state.live.mempool {
    Some(false)
  } else {
    None
  };
  let islock = if state.live.islock {
    Some(is_islocked(state, txid))
  } else {
    None
  };
  Ok(TxLive {
    tags: live_tags(mempool, islock),
  })
}

pub(crate) async fn live_tx_detail(
  state: &AppState,
  txid_lookup: &HashLookup,
) -> Result<TxDetail, ApiError> {
  state.check_live_feature(LiveFeature::Mempool)?;
  let (primary_txid, secondary_txid) = txid_lookup.ordered_candidates(HashEndian::View);
  let tx = state
    .live_state
    .mempool_tx(&primary_txid)
    .or_else(|| {
      secondary_txid
        .as_ref()
        .and_then(|txid| state.live_state.mempool_tx(txid))
    })
    .ok_or_else(|| ApiError::not_found("transaction not found"))?;
  let islocked = is_islocked(state, &tx.txid);
  let txid_hex = utils::view_hex(&tx.txid, HashEndian::View);

  let mut value = crate::live::mempool::query::build_tx_value_from_persisted(
    &state.live_state,
    &txid_hex,
    &tx.txid,
    islocked,
  )
  .ok_or_else(|| ApiError::not_found("transaction not found"))?;
  value.live = TxLive {
    tags: live_tags(
      Some(true),
      if state.live.islock {
        Some(islocked)
      } else {
        None
      },
    ),
  };
  Ok(value)
}

pub(crate) async fn live_rawtx(
  state: &AppState,
  txid_lookup: &HashLookup,
) -> Result<RawTxResponse, ApiError> {
  state.check_live_feature(LiveFeature::Mempool)?;
  let (primary_txid, secondary_txid) = txid_lookup.ordered_candidates(HashEndian::View);
  let tx = state
    .live_state
    .mempool_tx(&primary_txid)
    .or_else(|| {
      secondary_txid
        .as_ref()
        .and_then(|txid| state.live_state.mempool_tx(txid))
    })
    .ok_or_else(|| ApiError::not_found("transaction not found"))?;

  Ok(RawTxResponse {
    as_of_height: None,
    source: RawValueSource::Live,
    rawtx: hex::encode(&tx.raw_tx),
  })
}

pub(crate) async fn live_search_response(
  state: &AppState,
  _query: &str,
  txid_lookup: &HashLookup,
) -> Result<Option<SearchResponse>, ApiError> {
  if !state.live.mempool {
    return Ok(None);
  }
  let (primary_txid, secondary_txid) = txid_lookup.ordered_candidates(HashEndian::View);
  let found = state.live_state.mempool_tx(&primary_txid).is_some()
    || secondary_txid
      .as_ref()
      .is_some_and(|txid| state.live_state.mempool_tx(txid).is_some());
  if !found {
    return Ok(None);
  }
  Ok(Some(SearchResponse::Tx(SearchTxResult {
    tx_hash: utils::view_hex(&primary_txid, HashEndian::View),
    tx_tags: TxTags {
      mined: false,
      coinbase: false,
    },
  })))
}

fn is_islocked(state: &AppState, txid: &[u8]) -> bool {
  state.live_state.islock(txid).is_some()
}

fn live_tags(mempool: Option<bool>, islock: Option<bool>) -> Option<TxLiveTags> {
  if mempool.is_none() && islock.is_none() {
    return None;
  }
  Some(TxLiveTags { mempool, islock })
}

#[cfg(test)]
mod tests {
  #[test]
  fn live_tags_omits_empty_overlay_and_keeps_explicit_flags() {
    assert_eq!(super::live_tags(None, None), None);
    assert_eq!(
      super::live_tags(Some(false), Some(true)),
      Some(crate::rest::openapi::TxLiveTags {
        mempool: Some(false),
        islock: Some(true),
      })
    );
  }
}
