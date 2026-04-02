//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! IPC threading: read daemon messages, send commands.

use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc;
use std::sync::Arc;
use std::thread::JoinHandle;
use std::time::Duration;

use hyphend::wire::client::IpcClient;
use hyphend::wire::msg::DaemonCommand;

use crate::event::AppEvent;

/// Spawn the long-lived daemon message reader thread.
pub fn spawn_ipc_thread(
  socket_path: &Path,
  tx: mpsc::Sender<AppEvent>,
  shutdown: Arc<AtomicBool>,
) -> JoinHandle<()> {
  let path = socket_path.to_owned();
  return std::thread::spawn(move || loop {
    if shutdown.load(Ordering::Relaxed) {
      return;
    }
    match IpcClient::connect(&path, 50) {
      Ok(mut client) => {
        if let Err(error) = client.set_read_timeout(Some(Duration::from_millis(250))) {
          tracing::warn!("IPC set_read_timeout failed: {error}");
        }
        tracing::info!(path = %path.display(), "IPC connected");
        loop {
          if shutdown.load(Ordering::Relaxed) {
            return;
          }
          match client.recv_polled() {
            Ok(Some(msg)) => {
              if tx.send(AppEvent::Ipc(msg)).is_err() {
                return;
              }
            }
            Ok(None) => {}
            Err(e) => {
              tracing::warn!("IPC recv error: {e}");
              let _ = tx.send(AppEvent::IpcDisconnected);
              break;
            }
          }
        }
      }
      Err(e) => {
        tracing::warn!(path = %path.display(), "IPC connect failed: {e}");
        let _ = tx.send(AppEvent::IpcDisconnected);
      }
    }
    for _ in 0..12 {
      if shutdown.load(Ordering::Relaxed) {
        return;
      }
      std::thread::sleep(Duration::from_millis(250));
    }
  });
}

/// Dedicated thread for sending IPC commands to the daemon.
/// Reuses a single connection, reconnecting only on failure.
pub fn spawn_ipc_cmd_thread(
  socket_path: &Path,
  cmd_rx: mpsc::Receiver<DaemonCommand>,
  tx: mpsc::Sender<AppEvent>,
) -> JoinHandle<()> {
  let path = socket_path.to_owned();
  return std::thread::spawn(move || {
    let mut client: Option<IpcClient> = None;
    for cmd in cmd_rx {
      let mut delivered = false;
      for attempt in 0..2 {
        let c = match client.as_mut() {
          Some(c) => c,
          None => match IpcClient::connect(&path, 3) {
            Ok(c) => client.insert(c),
            Err(e) => {
              tracing::warn!("IPC command connect failed: {e}");
              if attempt == 1 {
                let _ = tx.send(AppEvent::IpcCommandFailed {
                  id: command_id(&cmd),
                  message: format!(
                    "{} delivery failed: could not connect to daemon IPC: {e}",
                    command_name(&cmd)
                  ),
                });
              }
              continue;
            }
          },
        };
        match c.send(&cmd) {
          Ok(()) => {
            delivered = true;
            break;
          }
          Err(e) => {
            tracing::warn!("IPC command send error: {e}");
            client = None;
            if attempt == 1 {
              let _ = tx.send(AppEvent::IpcCommandFailed {
                id: command_id(&cmd),
                message: format!(
                  "{} delivery failed: daemon IPC send error: {e}",
                  command_name(&cmd)
                ),
              });
            }
          }
        }
      }
      if !delivered {
        tracing::warn!(
          id = command_id(&cmd),
          command = command_name(&cmd),
          "IPC command dropped after retry"
        );
      }
    }
  });
}

fn command_id(cmd: &DaemonCommand) -> u64 {
  match cmd {
    DaemonCommand::Rewind { id, .. }
    | DaemonCommand::DisconnectTip { id }
    | DaemonCommand::ForceReindex { id, .. }
    | DaemonCommand::ForceCompact { id, .. } => *id,
  }
}

fn command_name(cmd: &DaemonCommand) -> &'static str {
  match cmd {
    DaemonCommand::Rewind { .. } => "rewind",
    DaemonCommand::DisconnectTip { .. } => "disconnect_tip",
    DaemonCommand::ForceReindex { .. } => "force_reindex",
    DaemonCommand::ForceCompact { .. } => "force_compact",
  }
}
