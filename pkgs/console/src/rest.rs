//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Blocking REST worker for browse requests.

use std::sync::mpsc;
use std::thread::JoinHandle;

use serde_json::Value;

use crate::event::{AppEvent, RestRequest, RestResult};

/// Spawn the blocking REST worker used for browse pages.
pub fn spawn_rest_thread(
  api_url: &str,
  tx: mpsc::Sender<AppEvent>,
  rx: mpsc::Receiver<RestRequest>,
) -> JoinHandle<()> {
  let base_url = api_url.trim_end_matches('/').to_owned();
  return std::thread::spawn(move || {
    for request in rx {
      let result = match request {
        RestRequest::Browse {
          index,
          limit,
          cursor,
        } => browse_request(&base_url, index, limit, cursor.as_deref()),
      };

      if tx.send(AppEvent::RestResponse(result)).is_err() {
        break;
      }
    }
  });
}

/// Execute one browse request against the daemon REST API.
fn browse_request(
  base_url: &str,
  index: crate::event::BrowseIndex,
  limit: u32,
  cursor: Option<&str>,
) -> RestResult {
  let endpoint = index.endpoint();
  let url = match index {
    crate::event::BrowseIndex::Metrics => {
      let offset = cursor.and_then(decode_offset_cursor).unwrap_or(0);
      let points = offset.saturating_add(limit).clamp(1, 500);
      format!("{base_url}/api/v1/{endpoint}?blocks={points}&points={points}")
    }
    _ => {
      let mut url = format!("{base_url}/api/v1/{endpoint}?limit={limit}");
      if let Some(cursor) = cursor {
        url.push_str("&cursor=");
        url.push_str(cursor);
      }
      url
    }
  };

  match fetch_json(&url) {
    Ok(body) => parse_browse_response(body, index, limit, cursor),
    Err(error) => RestResult::Error(error),
  }
}

fn parse_browse_response(
  body: Value,
  index: crate::event::BrowseIndex,
  limit: u32,
  cursor: Option<&str>,
) -> RestResult {
  let Some(all_rows) = body.get("value").and_then(Value::as_array).cloned() else {
    return RestResult::Error("browse response missing array value field".to_owned());
  };
  let next_cursor = match index {
    crate::event::BrowseIndex::Metrics => {
      let offset = cursor.and_then(decode_offset_cursor).unwrap_or(0);
      let (rows, next_cursor) = metrics_page(all_rows, limit, offset);
      let as_of_height = match body.get("asOfHeight") {
        Some(Value::Number(value)) => value.as_i64().and_then(|height| i32::try_from(height).ok()),
        Some(Value::Null) | None => None,
        Some(_) => {
          return RestResult::Error(
            "browse response asOfHeight field is not an integer".to_owned(),
          );
        }
      };
      return RestResult::Browse {
        index,
        rows,
        next_cursor,
        as_of_height,
      };
    }
    _ => match body.get("nextCursor") {
      Some(Value::String(value)) => Some(value.clone()),
      Some(Value::Null) | None => None,
      Some(_) => {
        return RestResult::Error("browse response nextCursor field is not a string".to_owned());
      }
    },
  };
  let as_of_height = match body.get("asOfHeight") {
    Some(Value::Number(value)) => value.as_i64().and_then(|height| i32::try_from(height).ok()),
    Some(Value::Null) | None => None,
    Some(_) => {
      return RestResult::Error("browse response asOfHeight field is not an integer".to_owned());
    }
  };
  RestResult::Browse {
    index,
    rows: all_rows,
    next_cursor,
    as_of_height,
  }
}

/// Derive one page slice from the metrics-chart response.
fn metrics_page(all_rows: Vec<Value>, limit: u32, offset: u32) -> (Vec<Value>, Option<String>) {
  let start = offset as usize;
  let rows = all_rows
    .into_iter()
    .skip(start)
    .take(limit as usize)
    .collect::<Vec<_>>();
  let next_cursor = (rows.len() == limit as usize && offset.saturating_add(limit) < 500)
    .then(|| encode_offset_cursor(offset.saturating_add(limit)));
  (rows, next_cursor)
}

fn decode_offset_cursor(cursor: &str) -> Option<u32> {
  cursor.parse::<u32>().ok()
}

fn encode_offset_cursor(offset: u32) -> String {
  offset.to_string()
}

/// Fetch and decode one JSON response body.
fn fetch_json(url: &str) -> Result<Value, String> {
  let response = ureq::get(url).call().map_err(|error| error.to_string())?;
  let mut body = response.into_body();
  let text = body.read_to_string().map_err(|error| error.to_string())?;
  serde_json::from_str(&text).map_err(|error| error.to_string())
}

#[cfg(test)]
mod tests {
  use serde_json::json;

  use crate::event::BrowseIndex;

  use super::parse_browse_response;

  #[test]
  fn parse_browse_response_rejects_missing_value() {
    let result = parse_browse_response(json!({"nextCursor": "abc"}), BrowseIndex::Blocks, 25, None);
    assert!(matches!(
      result,
      crate::event::RestResult::Error(message)
        if message.contains("missing array value field")
    ));
  }

  #[test]
  fn parse_browse_response_rejects_malformed_next_cursor() {
    let result = parse_browse_response(
      json!({"value": [], "nextCursor": 7}),
      BrowseIndex::Blocks,
      25,
      None,
    );
    assert!(matches!(
      result,
      crate::event::RestResult::Error(message)
        if message.contains("nextCursor field is not a string")
    ));
  }
}
