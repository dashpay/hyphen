//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use axum::extract::{Query, State};

use hyphend::index::block::rest::{search_by_hash, search_by_height};
use hyphend::index::metrics::rest::{chart, latest, MetricsChartQuery};
use hyphend::index::validator::rest::search_by_pro_tx_hash;

/// Seed one canonical block row because block search helpers read indexed block state.
async fn seed_block(
  pool: &hyphend::sql::SqlPool,
  height: u32,
  hash: &[u8],
  prev_hash: &[u8],
) -> Result<(), Box<dyn std::error::Error>> {
  let mut tx = pool.begin().await?;
  hyphend::sql::index::block::insert_block(
    &mut tx,
    height,
    hash,
    prev_hash,
    &[0x03; 32],
    0,
    0,
    0,
    1,
    80,
    1,
    "00",
    &hyphend::index::block::util::CbTxFields::default(),
  )
  .await?;
  tx.commit().await?;
  Ok(())
}

/// Seed one metrics row because the metrics REST routes read persisted `block_metrics`.
async fn seed_metrics_row(
  pool: &hyphend::sql::SqlPool,
  height: u32,
  tx_count: i64,
) -> Result<(), Box<dyn std::error::Error>> {
  let mut tx = pool.begin().await?;
  hyphend::sql::index::metrics::insert_block_metrics(
    &mut tx,
    height,
    tx_count,
    tx_count,
    0,
    1024 * i64::from(height + 1),
    None,
    tx_count * 50,
    1.5,
    2.5,
  )
  .await?;
  tx.commit().await?;
  Ok(())
}

/// Build one representative validator insert because validator REST tests need a real indexed row.
fn sample_validator_insert() -> hyphend::tx::ValidatorInsert {
  hyphend::tx::ValidatorInsert {
    pro_tx_hash: vec![0x11; 32],
    collateral_hash: vec![0x22; 32],
    collateral_index: 1,
    collateral_address: Some("Xcollateral111111111111111111111111".to_owned()),
    operator_pubkey: vec![0x33; 48],
    owner_address: Some("Xowner11111111111111111111111111111".to_owned()),
    voting_address: Some("Xvote111111111111111111111111111111".to_owned()),
    operator_reward: 5_000,
    validator_type: "regular".to_owned(),
    payout_addr: Some("Xpayout111111111111111111111111111".to_owned()),
    operator_payout_addr: Some("Xpayee1111111111111111111111111111".to_owned()),
    core_addrs: vec!["1.2.3.4:9999".to_owned()],
    plat_p2p_addrs: vec!["5.6.7.8:26656".to_owned()],
    plat_https_addrs: vec!["5.6.7.8:443".to_owned()],
  }
}

/// Seed one validator row because validator REST helpers read indexed validator state.
async fn seed_validator(
  pool: &hyphend::sql::SqlPool,
) -> Result<hyphend::tx::ValidatorInsert, Box<dyn std::error::Error>> {
  let insert = sample_validator_insert();
  let mut conn = pool.acquire().await?;
  hyphend::sql::index::validator::insert_validator(&mut conn, insert.clone(), 12, 0).await?;
  Ok(insert)
}

#[tokio::test]
async fn block_search_helpers_return_canonical_and_orphan_matches_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  seed_block(&pool, 7, &[0x11; 32], &[0x22; 32]).await?;
  hyphend::sql::index::block::insert_orphan_block_for_tests(
    &pool,
    6,
    &[0x33; 32],
    &[0x44; 32],
    &[0x55; 32],
  )
  .await?;

  let mut state = hyphend::tests::app_state();
  state.pool = pool;

  let canonical = search_by_height(&state, 7)
    .await
    .expect("height lookup should succeed");
  assert!(matches!(
    canonical,
    Some(hyphend::rest::openapi::SearchResponse::Block(_))
  ));

  let orphan = search_by_hash(&state, &[0x33; 32])
    .await
    .expect("orphan lookup should succeed");
  match orphan {
    Some(hyphend::rest::openapi::SearchResponse::Block(row)) => {
      assert_eq!(row.status, "orphan");
      assert!(row.block_tags.orphan);
    }
    other => panic!("expected orphan block search result, got {other:?}"),
  }

  Ok(())
}

#[tokio::test]
async fn metrics_routes_read_seeded_rows_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  seed_metrics_row(&pool, 5, 10).await?;
  seed_metrics_row(&pool, 6, 11).await?;
  hyphend::sql::sync::update_indexed_stage_height(&pool, hyphend::index::Stage::Metrics, 6).await?;

  let mut state = hyphend::tests::app_state();
  state.pool = pool;
  state.indices.metrics = true;

  let latest_value = latest(State(state.clone()))
    .await
    .expect("latest metrics should resolve")
    .0;
  assert_eq!(latest_value.height, 6);
  assert_eq!(latest_value.tx_count, 11);

  let chart_value = chart(
    State(state),
    Query(MetricsChartQuery {
      blocks: 2,
      points: 2,
    }),
  )
  .await
  .expect("metrics chart should resolve")
  .0;
  assert_eq!(chart_value.as_of_height, 6);
  assert_eq!(chart_value.start_height, 5);
  assert_eq!(chart_value.end_height, 6);
  assert_eq!(chart_value.value.len(), 2);

  Ok(())
}

#[tokio::test]
async fn validator_search_reads_seeded_rows_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;
  let insert = seed_validator(&pool).await?;

  let mut state = hyphend::tests::app_state();
  state.pool = pool;
  state.indices.validator = true;

  let result = search_by_pro_tx_hash(&state, "", &insert.pro_tx_hash)
    .await
    .expect("validator search should resolve");
  match result {
    Some(hyphend::rest::openapi::SearchResponse::Validator(row)) => {
      assert_eq!(row.pro_tx_hash, "11".repeat(32));
      assert_eq!(row.status, "registered");
    }
    other => panic!("expected validator search result, got {other:?}"),
  }

  Ok(())
}
