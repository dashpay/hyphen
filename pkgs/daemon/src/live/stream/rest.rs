//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Streamable HTTP endpoint for live NDJSON events.

use std::convert::Infallible;
use std::time::Duration;

use crate::live::stream::LiveEvent;
use crate::rest::error::ApiError;
use crate::rest::openapi::ErrorResponse;
use crate::rest::{AppState, LiveFeature};
use axum::body::Body;
use axum::extract::State;
use axum::http::header::{CACHE_CONTROL, CONTENT_TYPE};
use axum::http::{HeaderValue, StatusCode};
use axum::response::Response;
use bytes::Bytes;
use futures::stream;

/// Stream live NDJSON events over chunked HTTP.
///
/// # Errors
///
/// Returns `ApiError` when the live stream capability is disabled or the
/// stream hub has not been initialized yet.
#[utoipa::path(
  get,
  path = "/api/v1/stream",
  description = "Requires the live stream capability. Event payloads are server-specific live stream envelopes sourced from enabled ZMQ-fed and RPC-fed pipelines, not from any indexed snapshot. With the live ISLock capability enabled, a mempool_tx row carries live.tags.islock=true when an InstantSend lock for the transaction is known; otherwise, or with that capability disabled, the flag is omitted. A lock seen after the row was sent re-sends the row with the rawtxlock topic sequence number, so consumers should upsert mempool rows by txid rather than append.",
  responses(
    (
      status = 200,
      description = "Chunked NDJSON stream of live events",
      body = crate::live::stream::event::LiveEvent,
      content_type = "application/x-ndjson"
    ),
    (status = 501, description = "Stream feature disabled", body = ErrorResponse),
    (status = 503, description = "Stream source not ready yet", body = ErrorResponse),
  ),
  tag = "Live"
)]
pub async fn events(State(state): State<AppState>) -> Result<Response, ApiError> {
  state.check_live_feature(LiveFeature::Stream)?;

  let hub = state
    .stream_hub
    .clone()
    .ok_or_else(|| ApiError::service_unavailable("stream source is not ready"))?;
  let heartbeat_every = Duration::from_secs(state.stream_heartbeat_secs.max(1));
  let rx = hub.subscribe();
  let mut heartbeat = tokio::time::interval(heartbeat_every);
  heartbeat.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
  heartbeat.tick().await;

  let body_stream = stream::unfold((rx, heartbeat), |(mut rx, mut heartbeat)| async move {
    loop {
      let next_event = tokio::select! {
        recv = rx.recv() => match recv {
          Ok(event) => event,
          Err(tokio::sync::broadcast::error::RecvError::Lagged(skipped)) => {
            tracing::warn!(skipped, "stream client lagged behind live events");
            continue;
          }
          Err(tokio::sync::broadcast::error::RecvError::Closed) => return None,
        },
        _ = heartbeat.tick() => LiveEvent::heartbeat(),
      };

      let mut line = match serde_json::to_vec(&next_event) {
        Ok(line) => line,
        Err(err) => {
          tracing::warn!("stream event serialization failed: {err}");
          continue;
        }
      };
      line.push(b'\n');
      return Some((Ok::<Bytes, Infallible>(Bytes::from(line)), (rx, heartbeat)));
    }
  });

  let mut response = Response::new(Body::from_stream(body_stream));
  *response.status_mut() = StatusCode::OK;
  response.headers_mut().insert(
    CONTENT_TYPE,
    HeaderValue::from_static("application/x-ndjson"),
  );
  response
    .headers_mut()
    .insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
  Ok(response)
}
