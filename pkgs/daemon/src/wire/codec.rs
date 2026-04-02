//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Length-prefixed JSON codec for IPC framing.
//!
//! Wire format: `[4-byte BE length][JSON payload]`.

use super::msg::{DaemonCommand, DaemonMessage};

/// Encode a message as length-prefixed JSON.
///
/// # Errors
///
/// Returns an error string when JSON serialization fails.
pub fn encode(msg: &DaemonMessage) -> Result<Vec<u8>, String> {
  let payload = serde_json::to_vec(msg).map_err(|e| e.to_string())?;
  let mut out = Vec::with_capacity(4 + payload.len());
  out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
  out.extend_from_slice(&payload);
  Ok(out)
}

#[cfg(feature = "wire")]
/// Decode a payload (without the length prefix) into a `DaemonMessage`.
///
/// # Errors
///
/// Returns an error string when the payload is not valid `DaemonMessage`
/// JSON.
pub fn decode(payload: &[u8]) -> Result<DaemonMessage, String> {
  serde_json::from_slice(payload).map_err(|e| e.to_string())
}

#[cfg(feature = "wire")]
/// Encode a command as length-prefixed JSON.
///
/// # Errors
///
/// Returns an error string when JSON serialization fails.
pub fn encode_command(cmd: &DaemonCommand) -> Result<Vec<u8>, String> {
  let payload = serde_json::to_vec(cmd).map_err(|e| e.to_string())?;
  let mut out = Vec::with_capacity(4 + payload.len());
  out.extend_from_slice(&(payload.len() as u32).to_be_bytes());
  out.extend_from_slice(&payload);
  Ok(out)
}

/// Decode a payload into a DaemonCommand.
///
/// # Errors
///
/// Returns an error string when the payload is not valid `DaemonCommand`
/// JSON.
pub fn decode_command(payload: &[u8]) -> Result<DaemonCommand, String> {
  serde_json::from_slice(payload).map_err(|e| e.to_string())
}
