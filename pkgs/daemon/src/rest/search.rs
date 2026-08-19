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
/// Minimum query length for every arm except the numeric block-height lookup.
///
/// Four, not three: at mainnet scale a 3-character hex prefix matches ~17,200
/// transactions against ~1,080 at four, and every one of those rows is a wasted
/// index entry the user will never see past `MAX_SEARCH_RESULTS`.
const MIN_AUTOCOMPLETE_QUERY_LEN: usize = 4;

/// Which pools a query could possibly match, derived from its shape.
///
/// This is a CANDIDATE SET, not a precedence winner. Hex and base58 overlap on
/// `[1-9a-f]`, and mainnet p2sh addresses start with `7` (testnet `8`/`9`) —
/// all hex digits — so a query can legitimately be both a hash prefix and an
/// address prefix, and both arms must run. Only `X`/`y` (p2pkh) and the
/// non-hex base58 letters are unambiguous.
///
/// `other` (governance) is served from the in-memory live state, not a
/// Postgres pool, and matches both object hashes and free text; it therefore
/// stays on for any printable query rather than only the "neither" shape,
/// which costs no database work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SearchArms {
  /// Numeric block height: a primary-key hit, so no length minimum.
  height: bool,
  /// Hex charset: blocks + transactions + validators.
  hex: bool,
  /// Base58 charset: addresses.
  address: bool,
  /// Governance and other free-text surfaces (in-memory).
  other: bool,
}

impl SearchArms {
  /// Whether any arm past the free height lookup is reachable.
  fn has_prefix_arm(self) -> bool {
    self.hex || self.address || self.other
  }
}

/// Classify a query into the set of arms that could possibly match it.
fn classify(query: &str) -> SearchArms {
  let mut arms = SearchArms::default();
  // All digits and inside `i32`: a block-height candidate. The `0 <= n <= tip`
  // bound is enforced by the lookup itself (genesis is a real block, and a
  // height past the tip simply misses), and the probe is a primary-key hit, so
  // it is always worth including regardless of length.
  if !query.is_empty() && query.bytes().all(|b| b.is_ascii_digit()) && query.parse::<i32>().is_ok()
  {
    arms.height = true;
  }
  if query.chars().count() < MIN_AUTOCOMPLETE_QUERY_LEN {
    return arms;
  }
  arms.hex = is_base16(query);
  arms.address = is_base58(query);
  arms.other = is_ascii_printable(query);
  arms
}

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
  description = "Server-specific cross-domain autocomplete over indexed and live-backed data. Exact full 64-hex queries may resolve either stored-endian form, but partial matching only uses canonical display identifiers. Queries are dispatched by shape so only the pools that could match are probed; hex and base58 overlap, so both arms may run. Suggestions are capped at 10 results and, apart from numeric block heights, queries shorter than 4 characters are rejected.",
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
  let arms = classify(query);
  let mut results = Vec::new();
  let mut seen = HashSet::new();
  // The height probe is a primary-key lookup: free, and a hit is an
  // unambiguous answer, so it short-circuits the fan-out entirely.
  if arms.height {
    if let Ok(height) = query.parse::<i32>() {
      if let Some(result) = crate::index::block::rest::search_by_height(&state, height).await? {
        push_unique(&mut results, &mut seen, result);
        return Ok(Json(SearchListResponse { value: results }));
      }
      // A number is a whole height, not a truncated prefix, so a miss is an
      // empty result. Falling through to the minimum-length refusal below told
      // a client that asked about a block which does not exist that its request
      // was malformed, which it was not.
      if !arms.has_prefix_arm() {
        return Ok(Json(SearchListResponse { value: results }));
      }
    }
  }
  if !arms.has_prefix_arm() {
    return Err(ApiError::bad_request(&format!(
      "search query must be at least {MIN_AUTOCOMPLETE_QUERY_LEN} characters"
    )));
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
  if arms.hex {
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
  if arms.address {
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
  if arms.other {
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
    SearchResponse::Block(row) => format!("block:{}", row.block_hash),
    SearchResponse::Tx(row) => format!("tx:{}", row.tx_hash),
    SearchResponse::Address(row) => format!("address:{}", row.address),
    SearchResponse::Validator(row) => format!("validator:{}", row.pro_tx_hash),
    SearchResponse::Proposal(row) => format!("proposal:{}", row.hash),
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
  use super::{classify, MIN_AUTOCOMPLETE_QUERY_LEN};
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
  fn numeric_queries_reach_the_height_arm_at_any_length() {
    // Genesis: one character, and the pkey probe is free.
    let zero = classify("0");
    assert!(zero.height, "q=0 must still resolve as a block height");
    assert!(
      !zero.has_prefix_arm(),
      "a one-character query must not fan out to prefix pools"
    );
    assert!(classify("967013").height);
    // Wider than i32: not a height, but still a hex prefix.
    let huge = classify("99999999999999999999");
    assert!(!huge.height);
    assert!(huge.hex);
  }

  #[test]
  fn p2sh_and_p2pkh_prefixes_both_reach_the_address_arm() {
    // Mainnet p2sh starts with `7`, a hex digit: strict precedence would send
    // this to the hash pools only and silently drop the address.
    let p2sh = classify("7abc");
    assert!(
      p2sh.address,
      "a `7`-prefixed p2sh query must reach addresses"
    );
    assert!(p2sh.hex, "it is also a valid hash prefix, so hex runs too");
    // Testnet p2sh: `8`/`9` are hex digits as well.
    assert!(classify("8abc").address);
    assert!(classify("9abc").address);
    // Mainnet p2pkh `X` and testnet `y` are unambiguous: address only.
    let p2pkh = classify("Xabc");
    assert!(p2pkh.address);
    assert!(!p2pkh.hex);
    let testnet = classify("yabc");
    assert!(testnet.address);
    assert!(!testnet.hex);
  }

  #[test]
  fn unambiguous_and_ambiguous_shapes_pick_the_right_pools() {
    // Contains `0`, which is hex but not base58: hash pools only.
    let hex_only = classify("dead0");
    assert!(hex_only.hex);
    assert!(!hex_only.address);
    // The overlap charset [1-9a-f]: both arms.
    let ambiguous = classify("dead");
    assert!(ambiguous.hex && ambiguous.address);
    // Neither charset: governance / other only, no database pool.
    let text = classify("treasury proposal");
    assert!(!text.hex && !text.address);
    assert!(text.other);
  }

  #[test]
  fn short_non_numeric_queries_reach_no_arm() {
    for query in ["abc", "Xa", "%", "de"] {
      let arms = classify(query);
      assert!(
        !arms.has_prefix_arm(),
        "{query:?} is under the {MIN_AUTOCOMPLETE_QUERY_LEN}-character minimum"
      );
      assert!(!arms.height, "{query:?} is not numeric");
    }
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
        block_hash: "a".to_owned(),
        status: "canonical".to_owned(),
        block_tags: canonical_block_tags(),
      }),
      SearchResponse::Block(SearchBlockResult {
        block_hash: "a".to_owned(),
        status: "canonical".to_owned(),
        block_tags: canonical_block_tags(),
      })
    );
    let _ = SearchResponse::Address(SearchAddressResult {
      address: "Xabc".to_owned(),
    });
    let _ = SearchResponse::Validator(SearchValidatorResult {
      pro_tx_hash: "11".repeat(32),
      status: "registered".to_owned(),
    });
    let _ = SearchResponse::Proposal(SearchProposalResult {
      hash: "22".repeat(32),
    });
  }
}
