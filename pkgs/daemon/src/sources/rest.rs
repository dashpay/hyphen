//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Node REST transport with exponential backoff and jitter.

use std::collections::HashMap;
use std::hash::{BuildHasher, Hasher, RandomState};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use futures::stream::{self, StreamExt, TryStreamExt};
use reqwest::Client;
use serde_json::Value;

use super::RestSource;
use crate::config::CoreConfig;
use crate::error::{DaemonError, RestErrorKind};
use crate::policy::api::{API_REQUEST_TIMEOUT_SECS, API_TCP_KEEPALIVE_SECS};

/// Build a [`DaemonError::RestSource`] from a [`reqwest::Error`], classifying
/// the transport failure into a [`RestErrorKind`].
pub(crate) fn rest_err(context: &str, error: reqwest::Error) -> DaemonError {
  let kind = if error.is_connect() {
    RestErrorKind::Connect
  } else if error.is_timeout() {
    RestErrorKind::Timeout
  } else {
    RestErrorKind::Other
  };
  DaemonError::RestSource {
    message: format!("{context}: {error}"),
    kind,
  }
}

const MAX_RETRIES: u32 = 3;
const INITIAL_RETRY_DELAY_MS: u64 = 250;
const JITTER_BUCKETS: u64 = 1000;

/// Add ±50% jitter to prevent thundering-herd retries.
fn jitter(base: Duration) -> Duration {
  let rand_bits = RandomState::new().build_hasher().finish();
  let factor = 0.5 + (rand_bits % JITTER_BUCKETS) as f64 / JITTER_BUCKETS as f64;
  Duration::from_secs_f64(base.as_secs_f64() * factor)
}

struct RestMetrics {
  total_requests: AtomicU64,
  total_errors: AtomicU64,
  last_request_epoch: AtomicU64,
  latency_sum_us: AtomicU64,
  error_counts: Mutex<HashMap<String, u64>>,
}

impl RestMetrics {
  fn new() -> Self {
    Self {
      total_requests: AtomicU64::new(0),
      total_errors: AtomicU64::new(0),
      last_request_epoch: AtomicU64::new(0),
      latency_sum_us: AtomicU64::new(0),
      error_counts: Mutex::new(HashMap::new()),
    }
  }

  fn record_success(&self, elapsed_us: u64) {
    self.total_requests.fetch_add(1, Ordering::Relaxed);
    self.latency_sum_us.fetch_add(elapsed_us, Ordering::Relaxed);
    let epoch = SystemTime::now()
      .duration_since(UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs();
    self.last_request_epoch.store(epoch, Ordering::Relaxed);
  }

  fn record_error(&self, category: &str) {
    self.total_requests.fetch_add(1, Ordering::Relaxed);
    self.total_errors.fetch_add(1, Ordering::Relaxed);
    if let Ok(mut map) = self.error_counts.lock() {
      *map.entry(category.to_owned()).or_insert(0) += 1;
    }
  }
}

impl std::fmt::Debug for RestMetrics {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("RestMetrics")
      .field(
        "total_requests",
        &self.total_requests.load(Ordering::Relaxed),
      )
      .field("total_errors", &self.total_errors.load(Ordering::Relaxed))
      .finish()
  }
}

#[derive(Debug)]
pub struct CoreRestClient {
  http: Client,
  rest_base: String,
  concurrency: usize,
  metrics: RestMetrics,
}

impl CoreRestClient {
  /// Build one node REST client from the configured source settings.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the underlying HTTP client cannot
  /// be constructed.
  pub fn new(config: &CoreConfig) -> Result<Self, DaemonError> {
    let rest_base = format!("{}/rest", config.url.trim_end_matches('/'));
    let concurrency = config.rest_concurrency;
    let http = Client::builder()
      .timeout(Duration::from_secs(API_REQUEST_TIMEOUT_SECS))
      .tcp_keepalive(Duration::from_secs(API_TCP_KEEPALIVE_SECS))
      .pool_max_idle_per_host(concurrency)
      .build()
      .map_err(|e| rest_err("build REST client", e))?;
    Ok(Self {
      http,
      rest_base,
      concurrency,
      metrics: RestMetrics::new(),
    })
  }

  /// Send one raw REST request with retry and backoff handling.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the request exhausts retries, the
  /// response status is non-success, or the HTTP transport fails permanently.
  pub async fn rest_get(&self, path: &str) -> Result<reqwest::Response, DaemonError> {
    let url = format!("{}{}", self.rest_base, path);
    let mut delay = Duration::from_millis(INITIAL_RETRY_DELAY_MS);
    let start = Instant::now();
    let mut last_kind = RestErrorKind::Other;

    for attempt in 0..MAX_RETRIES {
      let result = self.http.get(&url).send().await;

      let resp = match result {
        Ok(r) => r,
        Err(e) if e.is_connect() || e.is_timeout() => {
          let category = if e.is_timeout() { "timeout" } else { "connect" };
          last_kind = if e.is_timeout() {
            RestErrorKind::Timeout
          } else {
            RestErrorKind::Connect
          };
          self.metrics.record_error(category);
          tracing::debug!(attempt, %url, %e, "REST request failed, retrying");
          tokio::time::sleep(jitter(delay)).await;
          delay *= 2;
          continue;
        }
        Err(e) => {
          self.metrics.record_error("other");
          return Err(rest_err("REST request", e));
        }
      };

      if resp.status() == reqwest::StatusCode::NOT_FOUND {
        self.metrics.record_error("404");
        return Err(DaemonError::RestSource {
          message: format!("REST 404 for {url} -- is `-rest=1` set in dash.conf?"),
          kind: RestErrorKind::Http(404),
        });
      }
      if resp.status() == reqwest::StatusCode::SERVICE_UNAVAILABLE
        || resp.status() == reqwest::StatusCode::TOO_MANY_REQUESTS
      {
        last_kind = RestErrorKind::Http(resp.status().as_u16());
        self
          .metrics
          .record_error(&resp.status().as_u16().to_string());
        tracing::debug!(
          attempt, %url, status = %resp.status(),
          "REST transient error, retrying"
        );
        tokio::time::sleep(jitter(delay)).await;
        delay *= 2;
        continue;
      }
      if !resp.status().is_success() {
        self
          .metrics
          .record_error(&resp.status().as_u16().to_string());
        return Err(DaemonError::RestSource {
          message: format!("REST HTTP {}", resp.status()),
          kind: RestErrorKind::Http(resp.status().as_u16()),
        });
      }
      self
        .metrics
        .record_success(start.elapsed().as_micros() as u64);
      return Ok(resp);
    }

    self.metrics.record_error("max_retries");
    Err(DaemonError::RestSource {
      message: format!("REST request failed after {MAX_RETRIES} attempts: {url}"),
      kind: last_kind,
    })
  }

  /// Fetch and decode one REST JSON payload.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the request fails or the response
  /// body is not valid JSON.
  pub async fn rest_json(&self, path: &str) -> Result<Value, DaemonError> {
    let resp = self.rest_get(path).await?;
    resp
      .json()
      .await
      .map_err(|e| rest_err("REST JSON parse", e))
  }

  /// Fetch one REST payload as raw bytes.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the request fails or the body
  /// cannot be read.
  pub async fn rest_bytes(&self, path: &str) -> Result<Vec<u8>, DaemonError> {
    let resp = self.rest_get(path).await?;
    resp
      .bytes()
      .await
      .map(|b| b.to_vec())
      .map_err(|e| rest_err("REST bytes", e))
  }

  /// Fetch one REST payload as trimmed text.
  ///
  /// # Errors
  ///
  /// Returns `DaemonError::RestSource` when the request fails or the body
  /// cannot be read as text.
  pub async fn rest_text(&self, path: &str) -> Result<String, DaemonError> {
    let resp = self.rest_get(path).await?;
    let text = resp
      .text()
      .await
      .map_err(|e| rest_err("REST text", e))?;
    Ok(text.trim().to_owned())
  }
}

#[async_trait]
impl RestSource for CoreRestClient {
  async fn block_count(&self) -> Result<u32, DaemonError> {
    let v = self.rest_json("/chaininfo.json").await?;
    v.get("blocks")
      .and_then(|n| n.as_u64())
      .map(|n| n as u32)
      .ok_or_else(|| DaemonError::RestSource {
        message: "chaininfo: missing blocks field".to_owned(),
        kind: RestErrorKind::Other,
      })
  }

  async fn block_hash(&self, height: u32) -> Result<String, DaemonError> {
    self
      .rest_text(&format!("/blockhashbyheight/{height}.hex"))
      .await
  }

  async fn raw_block(&self, hash: &str) -> Result<Vec<u8>, DaemonError> {
    self.rest_bytes(&format!("/block/{hash}.bin")).await
  }

  async fn fetch_blocks_batch(
    &self,
    heights: &[u32],
  ) -> Result<Vec<(String, Vec<u8>)>, DaemonError> {
    if heights.is_empty() {
      return Ok(vec![]);
    }
    stream::iter(heights.iter().copied())
      .map(|h| async move {
        let hash = self.block_hash(h).await?;
        let raw = self.raw_block(&hash).await?;
        Ok::<_, DaemonError>((hash, raw))
      })
      .buffered(self.concurrency)
      .try_collect()
      .await
  }
}
