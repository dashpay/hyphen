//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Typed global search endpoint over indexed and live-backed surfaces.

use axum::extract::{Query, State};
use axum::Json;
use serde::Deserialize;
use std::collections::HashSet;
use utoipa::IntoParams;

use super::error::ApiError;
use super::openapi::{ErrorResponse, SearchListResponse, SearchResponse};
use super::AppState;
use crate::utils::{self, HashEndian};

const MAX_SEARCH_RESULTS: usize = 10;
const MIN_AUTOCOMPLETE_QUERY_LEN: usize = 3;

/// Query parameters for global search.
#[derive(Debug, Deserialize, IntoParams)]
pub struct SearchParams {
  /// Search term for autocomplete suggestions and exact object lookups.
  pub q: String,
}

/// Search for up to 10 typed autocomplete suggestions.
///
/// # Errors
///
/// Returns `ApiError` when the search query is empty or a dependent index or
/// live source query fails.
#[utoipa::path(
  get,
  path = "/api/v1/search",
  description = "Server-specific cross-domain autocomplete over indexed and live-backed data. Exact full 64-hex queries may resolve either stored-endian form, but partial matching only uses canonical display identifiers. Suggestions are capped at 10 results and shorter than 3 characters return an empty list.",
  params(SearchParams),
  responses(
    (status = 200, description = "Autocomplete results", body = SearchListResponse),
    (status = 400, description = "Invalid search query", body = ErrorResponse),
    (status = 500, description = "Internal error", body = ErrorResponse),
  ),
  tag = "Search"
)]
pub async fn search(
  State(state): State<AppState>,
  Query(params): Query<SearchParams>,
) -> Result<Json<SearchListResponse>, ApiError> {
  let query = params.q.trim();
  if query.is_empty() {
    return Err(ApiError::bad_request("search query must not be empty"));
  }
  if query.chars().count() < MIN_AUTOCOMPLETE_QUERY_LEN {
    return Ok(Json(SearchListResponse { value: Vec::new() }));
  }

  let mut results = Vec::new();
  let mut seen = HashSet::new();
  if let Ok(height) = query.parse::<i32>() {
    if let Some(result) = crate::index::block::rest::search_by_height(&state, height).await? {
      push_unique(&mut results, &mut seen, result);
      return Ok(Json(SearchListResponse { value: results }));
    }
  }

  if query.len() == 64 {
    if let Ok(hash_lookup) = utils::HashLookup::from_hex(query) {
      for candidate in [HashEndian::Ser, HashEndian::View] {
        if let Some(result) =
          crate::index::block::rest::search_by_hash(&state, hash_lookup.bytes(candidate)).await?
        {
          push_unique(&mut results, &mut seen, result);
        }
      }
      for candidate in [HashEndian::Ser, HashEndian::View] {
        if let Some(result) =
          crate::index::tx::rest::search_by_txid(&state, query, hash_lookup.bytes(candidate))
            .await?
        {
          push_unique(&mut results, &mut seen, result);
        }
      }
      for candidate in [HashEndian::Ser, HashEndian::View] {
        if let Some(result) = crate::index::validator::rest::search_by_pro_tx_hash(
          &state,
          query,
          hash_lookup.bytes(candidate),
        )
        .await?
        {
          push_unique(&mut results, &mut seen, result);
        }
      }
      for candidate in [HashEndian::Ser, HashEndian::View] {
        if let Some(result) =
          crate::live::gov::rest::search_by_hash(&state, query, hash_lookup.bytes(candidate))
            .await?
        {
          push_unique(&mut results, &mut seen, result);
        }
      }
      return Ok(Json(SearchListResponse {
        value: cap_results(results),
      }));
    }
  }

  let hex_query = query.to_ascii_lowercase();
  if is_base16(query) {
    append_results(
      &mut results,
      &mut seen,
      crate::index::block::rest::search_by_hash_prefix(
        &state,
        &hex_query,
        MAX_SEARCH_RESULTS as i64,
      )
      .await?,
    );
    append_results(
      &mut results,
      &mut seen,
      crate::index::tx::rest::search_by_txid_prefix(&state, &hex_query, MAX_SEARCH_RESULTS as i64)
        .await?,
    );
    append_results(
      &mut results,
      &mut seen,
      crate::index::validator::rest::search_by_pro_tx_hash_prefix(
        &state,
        &hex_query,
        MAX_SEARCH_RESULTS as i64,
      )
      .await?,
    );
  }
  if is_base58(query) {
    if let Some(result) = crate::index::spent::rest::search_address(&state, query).await? {
      push_unique(&mut results, &mut seen, result);
    }
    append_results(
      &mut results,
      &mut seen,
      crate::index::spent::rest::search_addresses_by_prefix(
        &state,
        query,
        MAX_SEARCH_RESULTS as i64,
      )
      .await?,
    );
  }
  if is_ascii_printable(query) {
    append_results(
      &mut results,
      &mut seen,
      crate::live::gov::rest::search_by_query(&state, query, MAX_SEARCH_RESULTS as i64).await?,
    );
  }

  Ok(Json(SearchListResponse {
    value: cap_results(results),
  }))
}

fn append_results(
  results: &mut Vec<SearchResponse>,
  seen: &mut HashSet<String>,
  new_results: Vec<SearchResponse>,
) {
  for result in new_results {
    push_unique(results, seen, result);
    if results.len() >= MAX_SEARCH_RESULTS {
      break;
    }
  }
}

fn push_unique(
  results: &mut Vec<SearchResponse>,
  seen: &mut HashSet<String>,
  result: SearchResponse,
) {
  let key = match &result {
    SearchResponse::Block(row) => format!("block:{}", row.id),
    SearchResponse::Tx(row) => format!("tx:{}", row.id),
    SearchResponse::Address(row) => format!("address:{}", row.id),
    SearchResponse::Validator(row) => format!("validator:{}", row.id),
    SearchResponse::Proposal(row) => format!("proposal:{}", row.id),
  };
  if seen.insert(key) {
    results.push(result);
  }
}

fn cap_results(mut results: Vec<SearchResponse>) -> Vec<SearchResponse> {
  results.truncate(MAX_SEARCH_RESULTS);
  results
}

fn is_base16(query: &str) -> bool {
  query.chars().all(|ch| ch.is_ascii_hexdigit())
}

fn is_base58(query: &str) -> bool {
  query
    .chars()
    .all(|ch| matches!(ch, '1'..='9' | 'A'..='H' | 'J'..='N' | 'P'..='Z' | 'a'..='k' | 'm'..='z'))
}

fn is_ascii_printable(query: &str) -> bool {
  query
    .chars()
    .all(|ch| ch.is_ascii() && !ch.is_ascii_control())
}

#[cfg(test)]
mod tests {
  use crate::rest::openapi::{
    SearchAddressResult, SearchBlockResult, SearchProposalResult, SearchResponse,
    SearchValidatorResult,
  };

  fn canonical_block_tags() -> crate::rest::openapi::BlockTags {
    crate::index::block::rest::block_tags(false)
  }

  fn tx_tags(mined: bool, coinbase: bool) -> crate::rest::openapi::TxTags {
    crate::rest::openapi::TxTags { mined, coinbase }
  }

  #[test]
  fn tx_tags_encode_summary_state() {
    assert_eq!(
      tx_tags(false, false),
      crate::rest::openapi::TxTags {
        mined: false,
        coinbase: false,
      }
    );
  }

  #[test]
  fn canonical_block_tags_are_snapshot_bound() {
    assert_eq!(
      canonical_block_tags(),
      crate::rest::openapi::BlockTags { orphan: false }
    );
  }

  #[test]
  fn search_response_variants_keep_kind_specific_fields() {
    assert_eq!(
      SearchResponse::Block(SearchBlockResult {
        id: "a".to_owned(),
        status: "canonical".to_owned(),
        block_tags: canonical_block_tags(),
      }),
      SearchResponse::Block(SearchBlockResult {
        id: "a".to_owned(),
        status: "canonical".to_owned(),
        block_tags: canonical_block_tags(),
      })
    );
    let _ = SearchResponse::Address(SearchAddressResult {
      id: "Xabc".to_owned(),
    });
    let _ = SearchResponse::Validator(SearchValidatorResult {
      id: "11".repeat(32),
      status: "registered".to_owned(),
    });
    let _ = SearchResponse::Proposal(SearchProposalResult {
      id: "22".repeat(32),
    });
  }
}
