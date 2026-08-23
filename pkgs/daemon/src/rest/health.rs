//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Liveness probe for a container healthcheck.
//!
//! `/api/v1/status` (see [`super::status`]) answers "what is the sync state,"
//! which is not what a healthcheck needs: it returns `200` with a JSON body
//! whatever that state is, so nothing about its status code can tell a
//! supervisor "wedged" from "working." Worse, answering it at all means
//! reaching a Postgres connection pool — and during the incident this
//! endpoint exists to prevent a repeat of, that pool was exactly what had
//! wedged: a pgdata volume dropped off the bus, some in-flight queries never
//! returned (no client-side timeout bounds a query already dispatched on an
//! already-acquired connection, only the wait for a *free* one), and the REST
//! API went completely dark for 24 hours while the process sat alive in `ps`.
//!
//! This endpoint is built to never repeat that: it reads only the ingest
//! loop's in-process atomics ([`crate::ingest::IngestStatus`]) and touches no
//! connection pool at all, so it structurally cannot wedge behind one. A
//! [`tokio::time::timeout`] still wraps the computation as a second,
//! belt-and-suspenders guarantee, even though nothing in [`evaluate`] can
//! actually block.

use std::time::Duration;

use axum::extract::State;
use axum::http::StatusCode;
use axum::Json;
use serde::Serialize;

use super::AppState;
use crate::ingest::IngestStatus;

/// Bounds the whole health computation, however unreachable that bound
/// should be in practice — see the module doc for why this is
/// belt-and-suspenders rather than the primary guarantee.
const HEALTH_CHECK_TIMEOUT: Duration = Duration::from_millis(500);

/// Body returned by `/healthz`. Deliberately not part of the versioned
/// `/api/v1` surface or the generated OpenAPI document: this is an
/// operational probe for a supervisor, not a client-facing API.
#[derive(Debug, Clone, Copy, Serialize, PartialEq, Eq)]
#[serde(rename_all = "camelCase")]
pub struct HealthResponse {
  pub healthy: bool,
  pub reason: &'static str,
}

/// `GET /healthz`. `200` when ingestion is doing its job (actively ingesting,
/// or legitimately caught up at the tip); a non-2xx status otherwise —
/// parked, wedged, or unable to make progress. Suitable as-is for a Docker
/// `HEALTHCHECK`/compose `healthcheck.test`.
pub async fn healthz(State(state): State<AppState>) -> (StatusCode, Json<HealthResponse>) {
  let report = tokio::time::timeout(HEALTH_CHECK_TIMEOUT, async {
    evaluate(&state.ingest_status)
  })
  .await
  .unwrap_or(HealthResponse {
    healthy: false,
    reason: "health check did not complete within its own timeout",
  });

  let status = if report.healthy {
    StatusCode::OK
  } else {
    StatusCode::SERVICE_UNAVAILABLE
  };
  (status, Json(report))
}

/// The actual decision, as a pure function of the ingest status's public
/// getters — testable without an HTTP layer, an Axum state, or a running
/// loop behind it.
fn evaluate(status: &IngestStatus) -> HealthResponse {
  if status.operator_pause_requested() {
    return HealthResponse {
      healthy: false,
      reason: "ingestion is parked pending operator intervention",
    };
  }
  if status.loop_heartbeat_stale() {
    return HealthResponse {
      healthy: false,
      reason: "ingest loop heartbeat is stale; the loop's task may be wedged",
    };
  }
  if status.stalled() {
    return HealthResponse {
      healthy: false,
      reason: "ingest is behind the tip and has made no progress recently",
    };
  }
  HealthResponse {
    healthy: true,
    reason: "ingesting, or caught up at the tip",
  }
}

#[cfg(test)]
mod tests {
  use std::sync::atomic::Ordering;

  use super::*;

  /// The four states the healthcheck exists to distinguish, checked directly
  /// against the pure decision function rather than over HTTP.
  #[test]
  fn healthy_when_ingesting_normally() {
    let status = IngestStatus::new();
    assert_eq!(
      evaluate(&status),
      HealthResponse {
        healthy: true,
        reason: "ingesting, or caught up at the tip",
      }
    );
  }

  /// The exact incident this endpoint exists to catch: a failure-induced
  /// park must read unhealthy, so a Docker healthcheck — which the daemon had
  /// none of — can see what a `200` from `/api/v1/status` could not show.
  #[test]
  fn unhealthy_when_parked() {
    let status = IngestStatus::new();
    status.request_operator_pause();
    let report = evaluate(&status);
    assert!(!report.healthy);
    assert!(report.reason.contains("parked"), "{}", report.reason);
  }

  /// A loop whose task has stopped iterating — wedged mid-cycle on something
  /// with no timeout of its own — must read unhealthy even though nothing set
  /// the pause flag. This is the "wedged" case the pause flag alone cannot
  /// see, and it is checked without ever touching a connection pool.
  #[test]
  fn unhealthy_when_the_loop_heartbeat_is_stale() {
    let status = IngestStatus::new();
    status.loop_heartbeat_handle().store(
      unix_now_for_test() - HEALTH_HEARTBEAT_STALE_SECS_FOR_TEST - 1,
      Ordering::Relaxed,
    );
    let report = evaluate(&status);
    assert!(!report.healthy);
    assert!(report.reason.contains("wedged"), "{}", report.reason);
  }

  /// Behind the tip with no progress for a while is exactly the shape a lost
  /// storage backend produces: every ingest cycle backs off and retries, so
  /// the loop heartbeat stays fresh, but nothing is actually landing.
  #[test]
  fn unhealthy_when_stalled_behind_the_tip() {
    let status = IngestStatus::new();
    status.force_stalled_for_test();
    let report = evaluate(&status);
    assert!(!report.healthy);
    assert!(report.reason.contains("no progress"), "{}", report.reason);
  }

  fn unix_now_for_test() -> u64 {
    std::time::SystemTime::now()
      .duration_since(std::time::UNIX_EPOCH)
      .unwrap_or_default()
      .as_secs()
  }

  // Mirrors the private constant in `crate::ingest`; duplicated rather than
  // exposed because the health endpoint's tests should keep working even if
  // the ingest crate renames or retunes its own internal threshold, as long
  // as the *behavior* (a sufficiently stale heartbeat reads unhealthy) holds.
  const HEALTH_HEARTBEAT_STALE_SECS_FOR_TEST: u64 = 700;
}
