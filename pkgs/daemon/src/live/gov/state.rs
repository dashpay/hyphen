//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live governance refresh backed by node JSON-RPC.

use std::sync::Arc;
use std::time::Duration;

use crate::sql::SqlPool;
use serde_json::Value;
use tokio::sync::broadcast;

use crate::error::DaemonError;
use crate::lifecycle::Lifecycle;
use crate::live::stream::{LiveEvent, StreamHub};
use crate::live::warn;
use crate::policy::api::API_REFRESH_WARN_INTERVAL_SECS;
use crate::sources::{CoreRpcClient, ZmqEvent};
use crate::sql;

const ZMQ_REFRESH_DEBOUNCE_MS: u64 = 500;

/// Refresh the persisted governance proposal and vote state once.
pub async fn refresh_once(
  pool: &SqlPool,
  rpc: &CoreRpcClient,
  stream_hub: Option<&StreamHub>,
) -> Result<(), DaemonError> {
  let now = now_epoch_secs();
  let object_rows = fetch_object_rows(rpc, now).await?;
  let vote_rows = fetch_all_vote_rows(rpc, &object_rows, now).await?;

  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin governance snapshot tx: {e}")))?;
  sql::live::gov::replace_objects(&mut tx, &object_rows).await?;
  sql::live::gov::replace_votes(&mut tx, &vote_rows).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit governance snapshot tx: {e}")))?;
  if let Some(stream_hub) = stream_hub {
    publish_snapshot_events(stream_hub, &object_rows, &vote_rows);
  }
  Ok(())
}

/// Periodically refresh governance state until shutdown.
pub async fn refresh_loop(
  pool: SqlPool,
  rpc: Arc<CoreRpcClient>,
  lifecycle: Arc<Lifecycle>,
  refresh_secs: u64,
  stream_hub: Option<Arc<StreamHub>>,
) {
  while lifecycle.running() {
    if let Err(err) = refresh_once(&pool, &rpc, stream_hub.as_deref()).await {
      warn::warn_every(
        "live_gov_refresh_failed",
        Duration::from_secs(API_REFRESH_WARN_INTERVAL_SECS),
        |suppressed| {
          tracing::warn!(suppressed, "governance refresh failed: {err}");
        },
      );
    }
    lifecycle
      .sleep(std::time::Duration::from_secs(refresh_secs))
      .await;
  }
}

async fn refresh_votes_once(
  pool: &SqlPool,
  rpc: &CoreRpcClient,
  object_rows: &[sql::live::gov::GovernanceObjectRow],
  stream_hub: Option<&StreamHub>,
) -> Result<(), DaemonError> {
  let vote_rows = fetch_all_vote_rows(rpc, object_rows, now_epoch_secs()).await?;
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin governance vote refresh tx: {e}")))?;
  sql::live::gov::replace_votes(&mut tx, &vote_rows).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit governance vote refresh tx: {e}")))?;
  if let Some(stream_hub) = stream_hub {
    publish_snapshot_events(stream_hub, object_rows, &vote_rows);
  }
  Ok(())
}

async fn refresh_votes_for_object_once(
  pool: &SqlPool,
  rpc: &CoreRpcClient,
  object_hash: &[u8],
  stream_hub: Option<&StreamHub>,
) -> Result<(), DaemonError> {
  let object_hash_hex = hex::encode(object_hash);
  let updated_at = now_epoch_secs();
  let object_value = rpc.get_governance_object(&object_hash_hex).await?;
  let object_row = build_object_row(&object_hash_hex, &object_value, updated_at)?;
  let vote_rows = fetch_vote_rows(rpc, &object_hash_hex, object_hash, updated_at).await?;
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin governance targeted vote refresh tx: {e}")))?;
  sql::live::gov::upsert_object(&mut tx, &object_row).await?;
  sql::live::gov::replace_votes_for_object(&mut tx, object_hash, &vote_rows).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit governance targeted vote refresh tx: {e}")))?;
  if let Some(stream_hub) = stream_hub {
    for vote in &vote_rows {
      stream_hub.publish(LiveEvent::governance_vote(
        Some(hex::encode(&vote.object_hash)),
        Some(hex::encode(&vote.vote_hash)),
        None,
        String::from_utf8(vote.raw_vote.clone())
          .map(Value::String)
          .unwrap_or(Value::Null),
      ));
    }
  }
  Ok(())
}

async fn refresh_object_once(
  pool: &SqlPool,
  rpc: &CoreRpcClient,
  object_hash: &[u8],
  stream_hub: Option<&StreamHub>,
) -> Result<(), DaemonError> {
  let object_hash_hex = hex::encode(object_hash);
  let updated_at = now_epoch_secs();
  let object_value = rpc.get_governance_object(&object_hash_hex).await?;
  let object_row = build_object_row(&object_hash_hex, &object_value, updated_at)?;
  let vote_rows = fetch_vote_rows(rpc, &object_hash_hex, object_hash, updated_at).await?;

  let mut tx = pool
    .begin()
    .await
    .map_err(|e| DaemonError::Pg(format!("begin governance object refresh tx: {e}")))?;
  sql::live::gov::upsert_object(&mut tx, &object_row).await?;
  sql::live::gov::replace_votes_for_object(&mut tx, object_hash, &vote_rows).await?;
  tx.commit()
    .await
    .map_err(|e| DaemonError::Pg(format!("commit governance object refresh tx: {e}")))?;
  if let Some(stream_hub) = stream_hub {
    publish_snapshot_events(stream_hub, &[object_row], &vote_rows);
  }
  Ok(())
}

async fn fetch_object_rows(
  rpc: &CoreRpcClient,
  updated_at: i64,
) -> Result<Vec<sql::live::gov::GovernanceObjectRow>, DaemonError> {
  let objects = rpc.list_governance_proposals().await?;
  let mut object_rows = Vec::with_capacity(objects.len());
  for (hash_hex, value) in objects {
    object_rows.push(build_object_row(&hash_hex, &value, updated_at)?);
  }
  Ok(object_rows)
}

async fn fetch_all_vote_rows(
  rpc: &CoreRpcClient,
  object_rows: &[sql::live::gov::GovernanceObjectRow],
  updated_at: i64,
) -> Result<Vec<sql::live::gov::GovernanceVoteRow>, DaemonError> {
  let mut vote_rows = Vec::new();
  for object_row in object_rows {
    vote_rows.extend(
      fetch_vote_rows(
        rpc,
        &hex::encode(&object_row.object_hash),
        &object_row.object_hash,
        updated_at,
      )
      .await?,
    );
  }
  Ok(vote_rows)
}

fn build_object_row(
  hash_hex: &str,
  value: &Value,
  updated_at: i64,
) -> Result<sql::live::gov::GovernanceObjectRow, DaemonError> {
  let object_hash = hex::decode(hash_hex)
    .map_err(|e| DaemonError::RestSource(format!("decode governance hash {hash_hex}: {e}")))?;
  Ok(sql::live::gov::GovernanceObjectRow {
    object_hash,
    parent_hash: None,
    revision: None,
    object_time: value.get("CreationTime").and_then(Value::as_i64),
    object_type: value
      .get("ObjectType")
      .and_then(Value::as_i64)
      .unwrap_or_default() as i32,
    data_json: Some(value.clone()),
    raw_object: serde_json::to_vec(value)
      .map_err(|e| DaemonError::RestSource(format!("encode governance object: {e}")))?,
    updated_at,
  })
}

async fn fetch_vote_rows(
  rpc: &CoreRpcClient,
  hash_hex: &str,
  object_hash: &[u8],
  updated_at: i64,
) -> Result<Vec<sql::live::gov::GovernanceVoteRow>, DaemonError> {
  let votes = rpc.get_governance_votes(hash_hex).await?;
  let mut rows = Vec::with_capacity(votes.len());
  for (vote_hash_hex, vote_text) in votes {
    let vote_hash = hex::decode(&vote_hash_hex).map_err(|e| {
      DaemonError::RestSource(format!("decode governance vote hash {vote_hash_hex}: {e}"))
    })?;
    let (signal, outcome, vote_time) = parse_vote_fields(&vote_text);
    rows.push(sql::live::gov::GovernanceVoteRow {
      vote_hash,
      object_hash: object_hash.to_vec(),
      signal,
      outcome,
      vote_time,
      raw_vote: vote_text.into_bytes(),
      updated_at,
    });
  }
  Ok(rows)
}

fn publish_snapshot_events(
  stream_hub: &StreamHub,
  object_rows: &[sql::live::gov::GovernanceObjectRow],
  vote_rows: &[sql::live::gov::GovernanceVoteRow],
) {
  for object in object_rows {
    stream_hub.publish(LiveEvent::governance_object(
      Some(hex::encode(&object.object_hash)),
      None,
      serde_json::from_slice(&object.raw_object).unwrap_or(Value::Null),
    ));
  }
  for vote in vote_rows {
    stream_hub.publish(LiveEvent::governance_vote(
      Some(hex::encode(&vote.object_hash)),
      Some(hex::encode(&vote.vote_hash)),
      None,
      String::from_utf8(vote.raw_vote.clone())
        .map(Value::String)
        .unwrap_or(Value::Null),
    ));
  }
}

/// Trigger governance refreshes from ZMQ governance deltas until shutdown.
pub async fn refresh_from_zmq_loop(
  pool: SqlPool,
  rpc: Arc<CoreRpcClient>,
  mut rx: broadcast::Receiver<ZmqEvent>,
  lifecycle: Arc<Lifecycle>,
  stream_hub: Option<Arc<StreamHub>>,
) {
  let mut shutdown = lifecycle.subscribe();
  while lifecycle.running() {
    match tokio::select! {
      result = rx.recv() => Some(result),
      _ = shutdown.changed() => None,
    } {
      Some(Ok(ZmqEvent::RawGovernanceObject { object, .. })) => {
        lifecycle
          .sleep(Duration::from_millis(ZMQ_REFRESH_DEBOUNCE_MS))
          .await;
        if let Ok(object_hash) = crate::consensus::gov_object_hash(&object) {
          if let Err(err) =
            refresh_object_once(&pool, &rpc, &object_hash, stream_hub.as_deref()).await
          {
            tracing::warn!("governance object refresh from ZMQ delta failed: {err}");
          }
        } else if let Err(err) = refresh_once(&pool, &rpc, stream_hub.as_deref()).await {
          tracing::warn!("governance refresh from ZMQ object delta failed: {err}");
        }
      }
      Some(Ok(ZmqEvent::RawGovernanceVote { vote, .. })) => {
        lifecycle
          .sleep(Duration::from_millis(ZMQ_REFRESH_DEBOUNCE_MS))
          .await;
        let object_hash = crate::consensus::gov_vote_parent_hash(&vote).ok();
        match object_hash {
          Some(object_hash) => {
            if let Err(err) =
              refresh_votes_for_object_once(&pool, &rpc, &object_hash, stream_hub.as_deref()).await
            {
              tracing::warn!("governance vote refresh from ZMQ vote delta failed: {err}");
            }
          }
          None => match sql::live::gov::all_objects(&pool).await {
            Ok(object_rows) => {
              if let Err(err) =
                refresh_votes_once(&pool, &rpc, &object_rows, stream_hub.as_deref()).await
              {
                tracing::warn!("governance vote refresh from ZMQ vote delta failed: {err}");
              }
            }
            Err(err) => tracing::warn!("load governance objects for vote refresh failed: {err}"),
          },
        }
      }
      Some(Ok(_)) => {}
      Some(Err(broadcast::error::RecvError::Lagged(skipped))) => {
        tracing::warn!(skipped, "governance ZMQ consumer lagged");
      }
      Some(Err(broadcast::error::RecvError::Closed)) | None => break,
    }
  }
}

fn parse_vote_fields(vote_text: &str) -> (Option<i32>, Option<i32>, Option<i64>) {
  let parts: Vec<&str> = vote_text.split(':').collect();
  if parts.len() < 4 {
    return (None, None, None);
  }
  let vote_time = parts.get(1).and_then(|s| s.parse::<i64>().ok());
  let outcome = parts.get(2).and_then(|s| match *s {
    "yes" => Some(1),
    "no" => Some(2),
    "abstain" => Some(3),
    _ => None,
  });
  let signal = parts.get(3).and_then(|s| match *s {
    "funding" => Some(1),
    "valid" => Some(2),
    "delete" => Some(3),
    "endorsed" => Some(4),
    _ => None,
  });
  (signal, outcome, vote_time)
}

fn now_epoch_secs() -> i64 {
  std::time::SystemTime::now()
    .duration_since(std::time::UNIX_EPOCH)
    .unwrap_or_default()
    .as_secs() as i64
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use super::{build_object_row, parse_vote_fields};

  #[test]
  fn parse_vote_fields_extracts_signal_outcome_and_time() {
    assert_eq!(
      parse_vote_fields("abc-1:1670019339:yes:funding:4"),
      (Some(1), Some(1), Some(1670019339))
    );
  }

  #[test]
  fn build_object_row_preserves_hash_and_payload() {
    let row = build_object_row(
      &"11".repeat(32),
      &json!({
        "ObjectType": 1,
        "CreationTime": 123,
        "DataString": "{\"name\":\"proposal\"}",
      }),
      456,
    )
    .expect("object row should build");
    assert_eq!(hex::encode(row.object_hash), "11".repeat(32));
    assert_eq!(row.object_time, Some(123));
    assert_eq!(row.updated_at, 456);
  }

  #[test]
  fn parse_vote_parent_hash_reads_uint256_display_order() {
    let mut raw_vote = Vec::new();
    raw_vote.extend_from_slice(&[0x55; 32]);
    raw_vote.extend_from_slice(&7u32.to_le_bytes());
    raw_vote.extend((0u8..32).collect::<Vec<_>>().iter().rev().copied());
    raw_vote.extend_from_slice(&1i32.to_le_bytes());
    raw_vote.extend_from_slice(&2i32.to_le_bytes());
    raw_vote.extend_from_slice(&3i64.to_le_bytes());
    raw_vote.push(0);

    let parent = crate::consensus::gov_vote_parent_hash(&raw_vote)
      .expect("parent hash should decode");
    assert_eq!(
      hex::encode(parent),
      "000102030405060708090a0b0c0d0e0f101112131415161718191a1b1c1d1e1f"
    );
  }

  #[test]
  fn parse_object_hash_matches_dash_hashing_rules() {
    let data_hex = "7b226e616d65223a2270726f706f73616c227d";
    let data = hex::decode(data_hex).expect("object data hex should decode");
    let signature = vec![0xaa, 0xbb, 0xcc];

    let mut raw = Vec::new();
    raw.extend_from_slice(&[0u8; 32]);
    raw.extend_from_slice(&1i32.to_le_bytes());
    raw.extend_from_slice(&123i64.to_le_bytes());
    raw.extend_from_slice(&[0u8; 32]);
    raw.push(data.len() as u8);
    raw.extend_from_slice(&data);
    raw.extend_from_slice(&1i32.to_le_bytes());
    raw.extend_from_slice(&[0x11; 32]);
    raw.extend_from_slice(&7u32.to_le_bytes());
    raw.push(signature.len() as u8);
    raw.extend_from_slice(&signature);

    let parsed = crate::consensus::gov_object_hash(&raw)
      .expect("object hash should decode");
    assert_eq!(
      hex::encode(parsed),
      "4a37b1a47d9b1137b2af4d7d36ad757c3c80d5fcb9c24ac80807fab24d2e18e3"
    );
  }
}
