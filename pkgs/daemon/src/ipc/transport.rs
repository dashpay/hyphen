//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Unix domain socket IPC server.

use std::path::Path;

use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::UnixListener;
use tokio::sync::{broadcast, mpsc};
use tokio::task::JoinSet;

use hyphend::wire::codec;
use hyphend::wire::msg::{DaemonCommand, DaemonMessage};

/// Bind a Unix socket, broadcast `DaemonMessage`s to clients,
/// and forward inbound `DaemonCommand` frames to `cmd_tx`.
///
/// # Errors
///
/// Returns an error string when the socket cannot be removed or bound.
pub async fn serve(
  socket_path: &Path,
  tx: broadcast::Sender<DaemonMessage>,
  cmd_tx: mpsc::Sender<DaemonCommand>,
  mut shutdown_rx: tokio::sync::watch::Receiver<bool>,
) -> Result<(), String> {
  let _ = std::fs::remove_file(socket_path);
  let listener = UnixListener::bind(socket_path).map_err(|e| e.to_string())?;
  let mut clients = JoinSet::new();
  tracing::info!(path = %socket_path.display(), "IPC socket listening");

  loop {
    tokio::select! {
      result = listener.accept() => {
        match result {
          Ok((stream, _)) => {
            if *shutdown_rx.borrow() {
              tracing::info!("IPC server shutting down");
              break;
            }
            let rx = tx.subscribe();
            let cmd = cmd_tx.clone();
            clients.spawn(handle_client(stream, rx, cmd));
          }
          Err(e) => tracing::warn!("IPC accept error: {e}"),
        }
      }
      _ = shutdown_rx.changed() => {
        tracing::info!("IPC server shutting down");
        break;
      }
    }
  }

  clients.abort_all();
  while clients.join_next().await.is_some() {}

  Ok(())
}

async fn handle_client(
  stream: tokio::net::UnixStream,
  mut rx: broadcast::Receiver<DaemonMessage>,
  cmd_tx: mpsc::Sender<DaemonCommand>,
) {
  let (mut reader, mut writer) = stream.into_split();

  // Spawn a task to read commands from the client
  let reader_handle = tokio::spawn(async move {
    loop {
      // Read 4-byte length prefix
      let mut len_buf = [0u8; 4];
      if reader.read_exact(&mut len_buf).await.is_err() {
        break;
      }
      let len = u32::from_be_bytes(len_buf) as usize;
      if len > hyphend::wire::IPC_MAX_MESSAGE_BYTES {
        tracing::warn!("IPC command too large ({len} bytes), dropping client");
        break;
      }
      let mut payload = vec![0u8; len];
      if reader.read_exact(&mut payload).await.is_err() {
        break;
      }
      match codec::decode_command(&payload) {
        Ok(cmd) => {
          tracing::info!(?cmd, "IPC command received");
          if cmd_tx.send(cmd).await.is_err() {
            break;
          }
        }
        Err(e) => {
          tracing::warn!("IPC decode error: {e}");
        }
      }
    }
  });

  // Write broadcast messages to the client
  loop {
    match rx.recv().await {
      Ok(msg) => {
        let frame = match codec::encode(&msg) {
          Ok(f) => f,
          Err(e) => {
            tracing::warn!("IPC encode error: {e}");
            continue;
          }
        };
        if writer.write_all(&frame).await.is_err() {
          break;
        }
      }
      Err(broadcast::error::RecvError::Lagged(n)) => {
        tracing::warn!("IPC client lagged, dropped {n} messages");
      }
      Err(broadcast::error::RecvError::Closed) => break,
    }
  }

  reader_handle.abort();
  if let Err(error) = reader_handle.await {
    if !error.is_cancelled() {
      tracing::warn!(?error, "IPC client reader task join failed");
    }
  }
}
