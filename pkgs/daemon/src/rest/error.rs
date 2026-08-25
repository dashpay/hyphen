//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! API error type with HTTP status mapping.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::error::{DaemonError, Outcome};

/// One API error response paired with its HTTP status code.
pub struct ApiError {
  pub status: StatusCode,
  pub code: &'static str,
  pub message: String,
  pub target: Option<String>,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct ApiErrorBody {
  code: &'static str,
  message: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  target: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiErrorEnvelope {
  error: ApiErrorBody,
}

impl std::fmt::Debug for ApiError {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("ApiError")
      .field("status", &self.status)
      .field("code", &self.code)
      .field("message", &self.message)
      .field("target", &self.target)
      .finish()
  }
}

impl ApiError {
  /// Construct a `404 not found` response.
  pub fn not_found(msg: impl Into<String>) -> Self {
    Self {
      status: StatusCode::NOT_FOUND,
      code: "NotFound",
      message: msg.into(),
      target: None,
    }
  }

  /// Construct a `500 internal server error` response.
  ///
  /// The caller-provided detail is logged, while the client receives a
  /// stable generic message.
  pub fn internal(msg: impl Into<String>) -> Self {
    let detail = msg.into();
    tracing::error!(detail, "internal API error");
    Self {
      status: StatusCode::INTERNAL_SERVER_ERROR,
      code: "InternalError",
      message: "internal server error".to_string(),
      target: None,
    }
  }

  /// Reject client text that PostgreSQL cannot accept as a parameter.
  ///
  /// A `text` parameter may not contain a NUL byte. Passed through, the driver
  /// raises SQLSTATE 22021, which grades as a permanent storage fault and
  /// surfaces as `500 InternalError` — so `?status=%00weird` was logged as a
  /// server defect and counted against the error budget, when it is simply
  /// malformed input. Other control characters cannot break a parameter but
  /// have no business in an address, a status or a hash, and rejecting them
  /// keeps the log readable.
  ///
  /// # Errors
  ///
  /// [`ApiError::bad_request`] naming `field`, when `value` is unusable.
  pub fn reject_untypable_text(field: &str, value: &str) -> Result<(), Self> {
    if let Some(bad) = value.chars().find(|c| *c == '\0' || c.is_control()) {
      return Err(Self::bad_request(format!(
        "{field} contains a character that cannot be queried (U+{:04X})",
        bad as u32
      )));
    }
    Ok(())
  }

  /// Construct a `400 bad request` response.
  pub fn bad_request(msg: impl Into<String>) -> Self {
    Self {
      status: StatusCode::BAD_REQUEST,
      code: "BadRequest",
      message: msg.into(),
      target: None,
    }
  }

  /// Construct a `501 not implemented` response.
  pub fn not_implemented(msg: impl Into<String>) -> Self {
    Self {
      status: StatusCode::NOT_IMPLEMENTED,
      code: "NotImplemented",
      message: msg.into(),
      target: None,
    }
  }

  /// Construct a `503 service unavailable` response.
  pub fn service_unavailable(msg: impl Into<String>) -> Self {
    Self {
      status: StatusCode::SERVICE_UNAVAILABLE,
      code: "ServiceUnavailable",
      message: msg.into(),
      target: None,
    }
  }

  /// Map one daemon query error into a public REST error response.
  ///
  /// The status comes from the error's own classification, not from reading
  /// its message: anything the storage crate grades transient (a deadlock, an
  /// exhausted pool, a Postgres still coming up) is a retriable `503`, and
  /// everything else is a `500`. Grading by structure is what makes this path
  /// reachable at all — the previous substring test only fired for one
  /// spelling of one pool error.
  pub fn from_daemon(err: DaemonError, context: &str) -> Self {
    match err.classify() {
      Outcome::Warning => {
        tracing::warn!(context, detail = %err, "storage temporarily unavailable");
        Self::service_unavailable("service is temporarily unavailable, retry shortly")
      }
      Outcome::Error | Outcome::Fatal => Self::internal(format!("{context}: {err}")),
    }
  }
}

impl IntoResponse for ApiError {
  fn into_response(self) -> Response {
    let body = ApiErrorEnvelope {
      error: ApiErrorBody {
        code: self.code,
        message: self.message,
        target: self.target,
      },
    };
    (self.status, axum::Json(body)).into_response()
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use hyphen_db::HyDbError;

  fn pg(code: &str) -> HyDbError {
    HyDbError::Pg {
      message: "irrelevant".to_owned(),
      code: Some(code.to_owned()),
    }
  }

  /// The retriable path is reachable for every transient SQLSTATE, not just
  /// for one spelling of "pool timed out". Before the storage seam carried the
  /// typed error this branch could never fire from a real query failure.
  #[test]
  fn transient_storage_failures_are_retriable_503() {
    for code in ["40P01", "53300", "57P03", "08006", "55006"] {
      let err = ApiError::from_daemon(DaemonError::storage("block by hash", pg(code)), "block");
      assert_eq!(
        err.status,
        StatusCode::SERVICE_UNAVAILABLE,
        "sqlstate {code} should be retriable"
      );
      assert_eq!(err.code, "ServiceUnavailable");
    }
  }

  /// A permanent storage failure must not advertise itself as retriable.
  #[test]
  fn permanent_storage_failures_are_500() {
    for code in ["23505", "42P01"] {
      let err = ApiError::from_daemon(DaemonError::storage("block by hash", pg(code)), "block");
      assert_eq!(
        err.status,
        StatusCode::INTERNAL_SERVER_ERROR,
        "sqlstate {code} should not be retriable"
      );
    }
  }

  /// The whole chain, as a handler sees it: a `hyphen-db` error goes into
  /// `hydb_err` and comes out as a 503 because its SQLSTATE survived.
  #[test]
  fn hydb_err_preserves_retriability_end_to_end() {
    let err = crate::rest::hydb::hydb_err(pg("40P01"), "block by hash");
    assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);

    let err = crate::rest::hydb::hydb_err(pg("42P01"), "block by hash");
    assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
  }

  /// A pool failure never reaches the engine and so has no SQLSTATE; the
  /// message fallback in the storage crate must still make it retriable.
  #[test]
  fn pool_exhaustion_without_sqlstate_is_still_503() {
    let err = crate::rest::hydb::hydb_err(
      HyDbError::Pg {
        message: "pool timed out while waiting for an open connection".to_owned(),
        code: None,
      },
      "list blocks",
    );
    assert_eq!(err.status, StatusCode::SERVICE_UNAVAILABLE);
  }

  /// Non-storage errors keep their previous 500 behaviour.
  #[test]
  fn non_storage_errors_stay_500() {
    let err = ApiError::from_daemon(DaemonError::Index("stale".to_owned()), "ctx");
    assert_eq!(err.status, StatusCode::INTERNAL_SERVER_ERROR);
  }
}
