//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! IPC verb handlers dispatched from the control plane.
//!
//! The storage engine advances all derivation atomically per block, so the
//! old pause/fence/suspend choreography is gone: a verb simply calls the
//! single-writer ingest handle, which serializes against the ingest loop
//! internally.

pub mod reindex;
pub mod rewind;

use std::sync::Arc;

use hyphen_db::{HyDbIngest, HyphenDb};
use hyphend::wire::msg::{DaemonCommand, DaemonMessage};

/// Resources the command handler needs.
#[derive(Clone)]
pub struct CommandCtx {
  /// The storage facade, for tip reads.
  pub hydb: HyphenDb,
  /// The single-writer ingest handle shared with the ingest loop.
  pub ingest: Arc<HyDbIngest>,
  pub event_tx: tokio::sync::broadcast::Sender<DaemonMessage>,
  /// The ingest loop's operator-pause flag. Every verb here is an operator
  /// acting on the database, which is exactly the intervention a paused loop
  /// is waiting for, so dispatching any of them lifts the pause.
  pub ingest_pause: Arc<std::sync::atomic::AtomicBool>,
}

impl std::fmt::Debug for CommandCtx {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("CommandCtx").finish_non_exhaustive()
  }
}

pub(crate) fn emit_command_progress(
  event_tx: &tokio::sync::broadcast::Sender<DaemonMessage>,
  id: u64,
  message: impl Into<String>,
) {
  let _ = event_tx.send(DaemonMessage::CommandProgress {
    id,
    message: message.into(),
  });
}

pub(crate) fn emit_command_result(
  event_tx: &tokio::sync::broadcast::Sender<DaemonMessage>,
  id: u64,
  success: bool,
  message: impl Into<String>,
) {
  let _ = event_tx.send(DaemonMessage::CommandResult {
    id,
    success,
    message: message.into(),
  });
}

/// Lift an operator pause set by a non-transient ingest failure (E2.19).
///
/// Returns whether a pause was actually lifted, so the caller can say so
/// rather than logging on every command.
fn resume_ingest(ctx: &CommandCtx) -> bool {
  ctx
    .ingest_pause
    .swap(false, std::sync::atomic::Ordering::Relaxed)
}

/// Drain the inbound command channel and dispatch each verb handler.
pub async fn handle_commands(
  mut cmd_rx: tokio::sync::mpsc::Receiver<DaemonCommand>,
  ctx: CommandCtx,
  lifecycle: std::sync::Arc<crate::lifecycle::Lifecycle>,
) {
  let mut shutdown_rx = lifecycle.subscribe();
  while lifecycle.running() {
    let cmd = tokio::select! {
      maybe_cmd = cmd_rx.recv() => maybe_cmd,
      changed = shutdown_rx.changed() => {
        if changed.is_err() || *shutdown_rx.borrow_and_update() {
          None
        } else {
          continue;
        }
      }
    };
    let Some(cmd) = cmd else {
      break;
    };
    if resume_ingest(&ctx) {
      tracing::info!("operator command received; lifting the ingest pause");
    }
    match cmd {
      DaemonCommand::Rewind { id, target_height } => {
        rewind::rewind(&ctx, id, target_height).await;
      }
      DaemonCommand::DisconnectTip { id } => {
        rewind::disconnect_tip(&ctx, id).await;
      }
      DaemonCommand::ForceReindex { id, ref stage } => {
        reindex::reindex(&ctx, id, stage.clone());
      }
    }
  }
}
