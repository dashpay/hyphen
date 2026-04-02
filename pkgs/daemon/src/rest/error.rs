//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! API error type with HTTP status mapping.

use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use serde::Serialize;

use crate::error::DaemonError;

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
  pub fn from_daemon(err: DaemonError, context: &str) -> Self {
    match err {
      DaemonError::Pg(message) if pg_pool_timeout(&message) => {
        tracing::warn!(context, detail = %message, "postgres query pool exhausted");
        Self::service_unavailable("query capacity is temporarily exhausted")
      }
      other => Self::internal(format!("{context}: {other}")),
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

fn pg_pool_timeout(message: &str) -> bool {
  let lowered = message.to_ascii_lowercase();
  lowered.contains("timed out while waiting for an open connection")
    || lowered.contains("pool timed out")
    || lowered.contains("acquire timeout")
}
