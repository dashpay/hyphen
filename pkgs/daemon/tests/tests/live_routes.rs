//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

use axum::extract::{Query, State};
use axum::http::StatusCode;

use hyphend::live::mempool::rest::{recent_txs, summary};
use hyphend::live::network::rest::snapshot;
use hyphend::live::stream::rest::events;
use hyphend::rest::{CursorParams, LiveCapabilities, OverlayCapabilities};
use hyphend::utils::{self, HashEndian};

#[tokio::test]
async fn network_snapshot_rejects_disabled_feature() {
  let err = snapshot(State(hyphend::tests::app_state()))
    .await
    .expect_err("disabled network feature should fail");
  assert_eq!(err.status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn network_snapshot_returns_persisted_live_network_payload_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;

  let mut tx = pool.begin().await?;
  hyphend::sql::live::network::upsert_snapshot(
    &mut tx,
    &hyphend::sql::live::network::NetworkSnapshotRow {
      payload: serde_json::json!({
        "networkInfo": { "version": 230000, "subversion": "/node:22.0.0/" },
        "peerInfo": [{ "id": 7 }],
        "peerCount": 1,
      }),
      updated_at: 42,
    },
  )
  .await?;
  tx.commit().await?;

  let value = snapshot(State(hyphend::tests::app_state_with_pool(
    pool,
    LiveCapabilities {
      mempool: false,
      islock: false,
      network: true,
      governance: false,
      stream: false,
    },
    OverlayCapabilities {
      protx: false,
      chainlock: false,
    },
    0,
  )))
  .await
  .expect("network snapshot should resolve")
  .0;
  assert_eq!(value.network_info.version, Some(230000));
  assert_eq!(
    value.network_info.subversion.as_deref(),
    Some("/node:22.0.0/")
  );
  assert_eq!(value.peer_info[0].id, Some(7));
  assert_eq!(value.peer_count, 1);
  assert_eq!(value.updated_at, "1970-01-01T00:00:42Z");
  Ok(())
}

#[tokio::test]
async fn mempool_handlers_reject_disabled_feature() {
  let err = summary(State(hyphend::tests::app_state()))
    .await
    .expect_err("disabled mempool feature should fail");
  assert_eq!(err.status, StatusCode::NOT_IMPLEMENTED);

  let err = recent_txs(
    State(hyphend::tests::app_state()),
    Query(CursorParams::default()),
  )
  .await
  .expect_err("disabled mempool feature should fail");
  assert_eq!(err.status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn mempool_handlers_return_persisted_live_rows_when_pg_is_configured(
) -> Result<(), Box<dyn std::error::Error>> {
  let Some(pool) = hyphend::tests::pg_pool().await? else {
    return Ok(());
  };
  hyphend::sql::bootstrap::reset(&pool).await?;

  let txid = vec![0x55; 32];
  let mut tx = pool.begin().await?;
  hyphend::sql::live::mempool::upsert_summary(
    &mut tx,
    &hyphend::sql::live::mempool::MempoolSummaryRow {
      tx_count: 3,
      total_bytes: 222,
      usage_bytes: Some(333),
      max_bytes: Some(444),
      updated_at: 12,
    },
  )
  .await?;
  hyphend::sql::live::mempool::upsert_recent_tx(
    &mut tx,
    &hyphend::sql::live::mempool::MempoolRecentTxRow {
      txid: txid.clone(),
      raw_tx: vec![0xaa, 0xbb],
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
      size: 111,
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
      raw_islock: vec![0xcc],
      seen_at: 21,
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
  let summary_value = summary(State(state.clone()))
    .await
    .expect("mempool summary should resolve")
    .0;
  assert_eq!(summary_value.count, 3);
  assert_eq!(
    summary_value.recent_hashes[0],
    utils::view_hex(&txid, HashEndian::Ser)
  );
  assert_eq!(summary_value.updated_at, "1970-01-01T00:00:12Z");

  let txs_value = recent_txs(
    State(state),
    Query(CursorParams {
      limit: 10,
      cursor: None,
    }),
  )
  .await
  .expect("mempool tx list should resolve")
  .0;
  assert_eq!(txs_value.total_count, 3);
  assert_eq!(txs_value.value.len(), 1);
  assert!(txs_value.next_cursor.is_none());
  assert_eq!(
    txs_value.value[0].txid,
    utils::view_hex(&txid, HashEndian::Ser)
  );
  assert_eq!(txs_value.value[0].value_out, 42);
  assert_eq!(txs_value.value[0].first_seen_at, "1970-01-01T00:00:10Z");
  assert_eq!(txs_value.value[0].last_seen_at, "1970-01-01T00:00:20Z");
  assert!(!txs_value.value[0].tags.mined);
  assert!(!txs_value.value[0].tags.coinbase);
  Ok(())
}

fn stream_state(live_stream: bool, with_hub: bool) -> hyphend::rest::AppState {
  let mut state = hyphend::tests::app_state_with_pool(
    hyphend::sql::SqlPool::connect_lazy("postgres://localhost/unused")
      .expect("lazy pool should build"),
    LiveCapabilities {
      mempool: false,
      islock: false,
      network: false,
      governance: false,
      stream: live_stream,
    },
    OverlayCapabilities {
      protx: false,
      chainlock: false,
    },
    0,
  );
  state.stream_hub =
    with_hub.then(|| std::sync::Arc::new(hyphend::live::stream::StreamHub::new(8)));
  state
}

#[tokio::test]
async fn stream_endpoint_rejects_disabled_feature() {
  let err = events(State(stream_state(false, false)))
    .await
    .expect_err("disabled stream should fail");
  assert_eq!(err.status, StatusCode::NOT_IMPLEMENTED);
}

#[tokio::test]
async fn stream_endpoint_requires_ready_hub() {
  let err = events(State(stream_state(true, false)))
    .await
    .expect_err("stream without hub should fail");
  assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn stream_endpoint_sets_ndjson_content_type() {
  let response = events(State(stream_state(true, true)))
    .await
    .expect("ready stream should succeed");
  assert_eq!(response.headers()["content-type"], "application/x-ndjson");
}
