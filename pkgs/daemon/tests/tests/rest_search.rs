//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use axum::extract::{Query, State};
use axum::http::StatusCode;

use hyphend::rest::openapi::{
  SearchAddressResult, SearchBlockResult, SearchResponse, SearchTxResult, SearchValidatorResult,
};
use hyphend::rest::search::{search, SearchParams};
use hyphend::rest::{LiveCapabilities, OverlayCapabilities};

#[tokio::test]
async fn search_rejects_empty_query() {
  let err = search(
    State(hyphend::tests::app_state()),
    Query(SearchParams {
      q: "   ".to_owned(),
    }),
  )
  .await
  .expect_err("empty search should fail");
  assert_eq!(err.status, StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn search_returns_empty_results_for_short_query() {
  let value = search(
    State(hyphend::tests::app_state()),
    Query(SearchParams { q: "ab".to_owned() }),
  )
  .await
  .expect("short autocomplete query should resolve")
  .0;
  assert!(value.value.is_empty());
}

#[tokio::test]
async fn search_resolves_exact_short_block_height_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;

  let canonical_hash = vec![0x11; 32];
  let mut tx = pool.begin().await?;
  hyphend::sql::index::block::insert_block(
    &mut tx,
    7,
    &canonical_hash,
    &[0; 32],
    &[1; 32],
    1,
    1,
    1,
    1,
    100,
    2,
    "01",
    &hyphend::index::block::util::CbTxFields::default(),
  )
  .await?;
  tx.commit().await?;

  let state = hyphend::tests::app_state_with_pool(
    pool,
    LiveCapabilities {
      mempool: false,
      islock: false,
      network: false,
      governance: false,
      stream: false,
    },
    OverlayCapabilities {
      protx: false,
      chainlock: false,
    },
    25,
  );
  let value = search(State(state), Query(SearchParams { q: "7".to_owned() }))
    .await
    .expect("short block height search should resolve")
    .0;
  assert_eq!(
    value.value,
    vec![SearchResponse::Block(SearchBlockResult {
      block_hash: hex::encode(&canonical_hash),
      status: "canonical".to_owned(),
      block_tags: hyphend::rest::openapi::BlockTags { orphan: false },
    })]
  );
  Ok(())
}

#[tokio::test]
async fn search_resolves_exact_address_before_autocomplete_cutoff_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;

  sqlx::query(
    "INSERT INTO address_stats (
       address, balance, total_received, tx_count, utxo_count, first_seen_height, last_seen_height
     ) VALUES ($1, 0, 0, 0, 0, 0, 0)",
  )
  .bind("Xy")
  .execute(&pool)
  .await?;

  let mut state = hyphend::tests::app_state_with_pool(
    pool,
    LiveCapabilities {
      mempool: false,
      islock: false,
      network: false,
      governance: false,
      stream: false,
    },
    OverlayCapabilities {
      protx: false,
      chainlock: false,
    },
    25,
  );
  state.indices.address = true;

  let value = search(State(state), Query(SearchParams { q: "Xy".to_owned() }))
    .await
    .expect("exact short address search should resolve")
    .0;
  assert_eq!(
    value.value,
    vec![SearchResponse::Address(SearchAddressResult {
      address: "Xy".to_owned(),
    })]
  );
  Ok(())
}

#[tokio::test]
async fn search_returns_live_mempool_tx_tags_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;

  let txid = (0u8..32).collect::<Vec<_>>();
  let mut tx = pool.begin().await?;
  hyphend::sql::live::mempool::upsert_recent_tx(
    &mut tx,
    &hyphend::sql::live::mempool::MempoolRecentTxRow {
      txid: txid.clone(),
      raw_tx: vec![0xde, 0xad, 0xbe, 0xef],
      first_seen_at: 10,
      last_seen_at: 20,
    },
  )
  .await?;
  hyphend::sql::live::mempool::upsert_tx_detail(
    &mut tx,
    &hyphend::sql::live::mempool::MempoolTxDetailRow {
      txid: txid.clone(),
      version: 3,
      tx_type: "normal".to_owned(),
      lock_time: 0,
      size: 225,
      value_out: 42,
      input_count: 1,
      output_count: 1,
      fee: Some(7),
      payload: None,
      first_seen_at: 10,
      last_seen_at: 20,
    },
    &[],
    &[],
    &[],
  )
  .await?;
  hyphend::sql::live::islock::upsert_current(
    &mut tx,
    &hyphend::sql::live::islock::IslockCurrentRow {
      txid: txid.clone(),
      raw_islock: vec![0xaa],
      seen_at: 30,
    },
  )
  .await?;
  tx.commit().await?;

  let state = hyphend::tests::app_state_with_pool(
    pool,
    LiveCapabilities {
      mempool: true,
      islock: true,
      network: false,
      governance: false,
      stream: false,
    },
    OverlayCapabilities {
      protx: false,
      chainlock: false,
    },
    25,
  );
  let reversed_query = hex::encode(txid.iter().rev().copied().collect::<Vec<_>>());
  let value = search(State(state), Query(SearchParams { q: reversed_query }))
    .await
    .expect("live mempool tx search should resolve")
    .0;
  assert_eq!(
    value.value,
    vec![SearchResponse::Tx(SearchTxResult {
      tx_hash: hyphend::utils::view_hex(&txid, hyphend::utils::HashEndian::Ser),
      tx_tags: hyphend::rest::openapi::TxTags {
        mined: false,
        coinbase: false,
      },
    })]
  );
  Ok(())
}

#[tokio::test]
async fn search_returns_canonical_validator_id_when_query_uses_opposite_endian(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;

  let pro_tx_hash = (0x80u8..0xa0).collect::<Vec<_>>();
  let mut tx = pool.begin().await?;
  hyphend::sql::index::validator::insert_validator(
    &mut tx,
    hyphend::tx::ValidatorInsert {
      pro_tx_hash: pro_tx_hash.clone(),
      collateral_hash: vec![0; 32],
      collateral_index: 0,
      collateral_address: None,
      operator_pubkey: Vec::new(),
      owner_address: None,
      voting_address: None,
      operator_reward: 0,
      validator_type: "regular".to_owned(),
      payout_addr: None,
      operator_payout_addr: None,
      core_addrs: Vec::new(),
      plat_p2p_addrs: Vec::new(),
      plat_https_addrs: Vec::new(),
    },
    100,
    0,
  )
  .await?;
  tx.commit().await?;

  let mut state = hyphend::tests::app_state_with_pool(
    pool,
    LiveCapabilities {
      mempool: false,
      islock: false,
      network: false,
      governance: false,
      stream: false,
    },
    OverlayCapabilities {
      protx: false,
      chainlock: false,
    },
    25,
  );
  state.indices.validator = true;
  let reversed_query = hex::encode(pro_tx_hash.iter().rev().copied().collect::<Vec<_>>());
  let value = search(State(state), Query(SearchParams { q: reversed_query }))
    .await
    .expect("validator search should resolve")
    .0;
  assert_eq!(
    value.value,
    vec![SearchResponse::Validator(SearchValidatorResult {
      pro_tx_hash: hyphend::utils::view_hex(&pro_tx_hash, hyphend::utils::HashEndian::Ser),
      status: "registered".to_owned(),
    })]
  );
  Ok(())
}

#[tokio::test]
async fn search_distinguishes_canonical_and_orphan_blocks_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;

  let canonical_hash = vec![0x11; 32];
  let orphan_hash = vec![0x22; 32];
  let mut tx = pool.begin().await?;
  hyphend::sql::index::block::insert_block(
    &mut tx,
    150,
    &canonical_hash,
    &[0; 32],
    &[1; 32],
    1,
    1,
    1,
    1,
    100,
    2,
    "01",
    &hyphend::index::block::util::CbTxFields::default(),
  )
  .await?;
  hyphend::sql::index::block::insert_block(
    &mut tx,
    149,
    &orphan_hash,
    &[0; 32],
    &[2; 32],
    1,
    1,
    1,
    1,
    90,
    1,
    "00",
    &hyphend::index::block::util::CbTxFields::default(),
  )
  .await?;
  hyphend::sql::index::block::delete_block(&mut tx, 149).await?;
  tx.commit().await?;

  let state = hyphend::tests::app_state_with_pool(
    pool,
    LiveCapabilities {
      mempool: false,
      islock: false,
      network: false,
      governance: false,
      stream: false,
    },
    OverlayCapabilities {
      protx: false,
      chainlock: false,
    },
    25,
  );
  let canonical = search(
    State(state.clone()),
    Query(SearchParams {
      q: hex::encode(&canonical_hash),
    }),
  )
  .await
  .expect("canonical block search should resolve")
  .0;
  assert_eq!(
    canonical.value,
    vec![SearchResponse::Block(SearchBlockResult {
      block_hash: hex::encode(&canonical_hash),
      status: "canonical".to_owned(),
      block_tags: hyphend::rest::openapi::BlockTags { orphan: false },
    })]
  );

  let canonical_by_height = search(
    State(state.clone()),
    Query(SearchParams {
      q: "150".to_owned(),
    }),
  )
  .await
  .expect("block height search should resolve")
  .0;
  assert_eq!(
    canonical_by_height.value,
    vec![SearchResponse::Block(SearchBlockResult {
      block_hash: hex::encode(&canonical_hash),
      status: "canonical".to_owned(),
      block_tags: hyphend::rest::openapi::BlockTags { orphan: false },
    })]
  );

  let orphan = search(
    State(state),
    Query(SearchParams {
      q: hex::encode(&orphan_hash),
    }),
  )
  .await
  .expect("orphan block search should resolve")
  .0;
  assert_eq!(
    orphan.value,
    vec![SearchResponse::Block(SearchBlockResult {
      block_hash: hex::encode(&orphan_hash),
      status: "orphan".to_owned(),
      block_tags: hyphend::rest::openapi::BlockTags { orphan: true },
    })]
  );
  Ok(())
}
