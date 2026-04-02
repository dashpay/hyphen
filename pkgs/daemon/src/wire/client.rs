//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Synchronous IPC client for blocking consumers (e.g. TUI).

use std::io::{ErrorKind, Read, Write};
use std::os::unix::net::UnixStream;
use std::path::Path;
use std::time::Duration;

use super::codec;
use super::msg::{DaemonCommand, DaemonMessage};

/// Blocking IPC client wrapping a Unix domain socket.
#[derive(Debug)]
pub struct IpcClient {
  stream: UnixStream,
}

impl IpcClient {
  /// Connect to the daemon socket, retrying up to `retries`
  /// times with 100ms between attempts.
  ///
  /// # Errors
  ///
  /// Returns the final socket error string when all retries are exhausted,
  /// or a local error when `retries` is zero.
  pub fn connect(socket_path: &Path, retries: usize) -> Result<Self, String> {
    for attempt in 0..retries {
      match UnixStream::connect(socket_path) {
        Ok(stream) => return Ok(Self { stream }),
        Err(e) if attempt + 1 == retries => return Err(e.to_string()),
        Err(_) => std::thread::sleep(Duration::from_millis(super::IPC_CLIENT_RETRY_DELAY_MS)),
      }
    }
    Err("connect: zero retries".to_owned())
  }

  /// Set the blocking read timeout on the underlying socket.
  ///
  /// # Errors
  ///
  /// Returns an error when the timeout cannot be applied to the socket.
  pub fn set_read_timeout(&self, timeout: Option<Duration>) -> Result<(), String> {
    self
      .stream
      .set_read_timeout(timeout)
      .map_err(|e| e.to_string())
  }

  /// Block until the next message arrives from the daemon.
  ///
  /// # Errors
  ///
  /// Returns an error when the socket closes, the frame exceeds the
  /// configured maximum size, or JSON decoding fails.
  pub fn recv(&mut self) -> Result<DaemonMessage, String> {
    let mut len_buf = [0u8; 4];
    self
      .stream
      .read_exact(&mut len_buf)
      .map_err(|e| e.to_string())?;
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > super::IPC_MAX_MESSAGE_BYTES {
      return Err(format!(
        "message too large ({len} bytes, max {})",
        super::IPC_MAX_MESSAGE_BYTES
      ));
    }
    let mut payload = vec![0u8; len];
    self
      .stream
      .read_exact(&mut payload)
      .map_err(|e| e.to_string())?;
    codec::decode(&payload)
  }

  /// Poll for the next message while allowing timeout wakeups.
  ///
  /// Returns `Ok(None)` when no bytes were available before the configured
  /// socket timeout elapsed.
  ///
  /// # Errors
  ///
  /// Returns an error when the socket closes, the frame exceeds the configured
  /// maximum size, or JSON decoding fails.
  pub fn recv_polled(&mut self) -> Result<Option<DaemonMessage>, String> {
    let mut len_buf = [0u8; 4];
    if !read_exact_polled(&mut self.stream, &mut len_buf)? {
      return Ok(None);
    }
    let len = u32::from_be_bytes(len_buf) as usize;
    if len > super::IPC_MAX_MESSAGE_BYTES {
      return Err(format!(
        "message too large ({len} bytes, max {})",
        super::IPC_MAX_MESSAGE_BYTES
      ));
    }
    let mut payload = vec![0u8; len];
    read_exact_polled(&mut self.stream, &mut payload)?
      .then_some(())
      .ok_or_else(|| "socket timed out mid-frame".to_owned())?;
    codec::decode(&payload).map(Some)
  }

  /// Send a command to the daemon.
  ///
  /// # Errors
  ///
  /// Returns an error when command encoding, socket writes, or flushing fail.
  pub fn send(&mut self, cmd: &DaemonCommand) -> Result<(), String> {
    let frame = codec::encode_command(cmd)?;
    self.stream.write_all(&frame).map_err(|e| e.to_string())?;
    self.stream.flush().map_err(|e| e.to_string())
  }
}

fn read_exact_polled(stream: &mut UnixStream, buf: &mut [u8]) -> Result<bool, String> {
  let mut filled = 0;
  while filled < buf.len() {
    match stream.read(&mut buf[filled..]) {
      Ok(0) => return Err("unexpected EOF".to_owned()),
      Ok(read) => filled += read,
      Err(error) if matches!(error.kind(), ErrorKind::TimedOut | ErrorKind::WouldBlock) => {
        if filled == 0 {
          return Ok(false);
        }
      }
      Err(error) => return Err(error.to_string()),
    }
  }
  Ok(true)
}
