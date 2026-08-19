//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Startup review of the Postgres server settings the daemon depends on but
//! cannot set (task F4).
//!
//! The daemon does not own `postgresql.conf` and must not pretend to. What it
//! can do is read the settings back and say, once, at startup, when they
//! contradict the way it is about to drive the server — the same principle as
//! reading pool session settings back rather than echoing the config.
//!
//! Every finding here is a warning, never a refusal: an under-sized
//! `shared_buffers` is slow, not incorrect, and an operator who has decided to
//! run that way must not be locked out of their own database.

use hyphen_db::query::HyDbQrStorageDiag;
use tracing::{info, warn};

/// Connections kept aside for superusers, autovacuum workers and the
/// operator's own `psql`. `superuser_reserved_connections` defaults to 3;
/// autovacuum takes up to `autovacuum_max_workers` (3 by default) more.
const RESERVED_CONNECTIONS: i64 = 10;

/// One thing worth telling the operator about their server configuration.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SettingWarning {
  /// The GUC (or the pair of them) the finding is about.
  pub setting: &'static str,
  /// What is wrong, in the operator's terms, with the numbers that show it.
  pub message: String,
}

/// Parse a Postgres memory setting as `current_setting` renders it.
///
/// Postgres returns these with a unit suffix (`8GB`, `64MB`, `16384kB`); a
/// bare number is in bytes. Returns `None` for anything unrecognised rather
/// than guessing, because a wrong parse here would produce a confidently wrong
/// warning, which is worse than no warning.
pub fn parse_pg_bytes(raw: &str) -> Option<u64> {
  let trimmed = raw.trim();
  let split = trimmed
    .find(|c: char| !c.is_ascii_digit())
    .unwrap_or(trimmed.len());
  let (digits, unit) = trimmed.split_at(split);
  let value: u64 = digits.parse().ok()?;
  let multiplier = match unit.trim().to_ascii_lowercase().as_str() {
    "" | "b" => 1,
    "kb" => 1024,
    "mb" => 1024 * 1024,
    "gb" => 1024 * 1024 * 1024,
    "tb" => 1024_u64.pow(4),
    _ => return None,
  };
  value.checked_mul(multiplier)
}

/// Render a byte count the way an operator reads it back into a config file.
fn human_bytes(bytes: u64) -> String {
  const GB: u64 = 1024 * 1024 * 1024;
  const MB: u64 = 1024 * 1024;
  if bytes >= GB {
    format!("{:.1}GB", bytes as f64 / GB as f64)
  } else if bytes >= MB {
    format!("{:.0}MB", bytes as f64 / MB as f64)
  } else {
    format!("{bytes}B")
  }
}

/// Review the server settings against how this daemon is configured to drive
/// it. Pure, so the thresholds are testable without a server.
///
/// `pool_total` is the sum of the configured pool sizes: those connections are
/// the daemon's steady-state claim on `max_connections`.
pub fn review(diag: &HyDbQrStorageDiag, pool_total: usize) -> Vec<SettingWarning> {
  let mut out = Vec::new();
  let pool_total = i64::try_from(pool_total).unwrap_or(i64::MAX);

  // 1. The pools must fit, with room left for a human and for autovacuum.
  if diag.max_connections > 0 {
    let usable = diag.max_connections - RESERVED_CONNECTIONS;
    if pool_total > diag.max_connections {
      out.push(SettingWarning {
        setting: "max_connections",
        message: format!(
          "the configured pools total {pool_total} connections but the server \
           allows {}: the daemon cannot open its own pools",
          diag.max_connections
        ),
      });
    } else if pool_total > usable {
      out.push(SettingWarning {
        setting: "max_connections",
        message: format!(
          "the configured pools total {pool_total} of {} connections, leaving \
           under {RESERVED_CONNECTIONS} for autovacuum, superusers and psql",
          diag.max_connections
        ),
      });
    }
  }

  let shared_buffers = parse_pg_bytes(&diag.shared_buffers);
  let cache_size = parse_pg_bytes(&diag.effective_cache_size);
  let work_mem = parse_pg_bytes(&diag.work_mem);

  // 2. `effective_cache_size` is the operator's statement of how much memory
  // the machine has for caching. Below `shared_buffers` it is a contradiction;
  // barely above it, the planner will prefer sequential scans over the index
  // scans this schema is built around.
  if let (Some(buffers), Some(cache)) = (shared_buffers, cache_size) {
    if cache < buffers {
      out.push(SettingWarning {
        setting: "effective_cache_size",
        message: format!(
          "effective_cache_size ({}) is below shared_buffers ({}), which cannot \
           be true: the planner is being told the machine has less memory than \
           Postgres has already reserved",
          human_bytes(cache),
          human_bytes(buffers)
        ),
      });
    } else if cache < buffers.saturating_mul(2) {
      out.push(SettingWarning {
        setting: "effective_cache_size",
        message: format!(
          "effective_cache_size ({}) is under twice shared_buffers ({}); the \
           usual ratio is 3x, and a low value pushes the planner off index \
           scans",
          human_bytes(cache),
          human_bytes(buffers)
        ),
      });
    }
  }

  // 3. Worst-case sort memory. Every pooled backend may claim `work_mem` (more
  // than once, for a query with several sorts), so the product is what the
  // machine has to survive, not the single value.
  if let (Some(work_mem), Some(cache)) = (work_mem, cache_size) {
    let worst_case = work_mem.saturating_mul(pool_total.max(1) as u64);
    if worst_case > cache / 2 {
      out.push(SettingWarning {
        setting: "work_mem",
        message: format!(
          "work_mem ({}) across {pool_total} pooled connections is up to {}, \
           over half of effective_cache_size ({}) — and one query can claim it \
           several times over",
          human_bytes(work_mem),
          human_bytes(worst_case),
          human_bytes(cache)
        ),
      });
    }
  }

  // 4. Settings the daemon could not read at all. Named explicitly: a check
  // that silently does not run is how the disk-space check went missing.
  if shared_buffers.is_none() {
    out.push(SettingWarning {
      setting: "shared_buffers",
      message: format!(
        "could not parse the server's value {:?}",
        diag.shared_buffers
      ),
    });
  }
  if cache_size.is_none() {
    out.push(SettingWarning {
      setting: "effective_cache_size",
      message: format!(
        "could not parse the server's value {:?}",
        diag.effective_cache_size
      ),
    });
  }

  out
}

/// Peak WAL one deep reorg is projected to write, in bytes.
///
/// A reorg at `max_reorg_depth` rewrites the projection for that many blocks
/// *and* replays their undo rows, so the write volume is about twice the
/// steady-state bytes per block. `bytes_per_block` is measured, not assumed:
/// database size over tip height.
pub fn projected_reorg_wal_bytes(bytes_per_block: u64, max_reorg_depth: i64) -> u64 {
  let depth = u64::try_from(max_reorg_depth.max(0)).unwrap_or(0);
  bytes_per_block.saturating_mul(depth).saturating_mul(2)
}

/// Read the server settings back and report on them, once, at startup.
///
/// `max_wal_size` is the setting this check was raised for — production runs
/// 4GB, a quarter of the test rig, and requested checkpoints tracked block
/// density exactly. The storage crate does not expose it (`HyDbQrStorageDiag`
/// carries `shared_buffers`, `work_mem`, `effective_cache_size` and
/// `max_connections` only), so the projection is logged for the operator to
/// compare by hand until it does.
pub async fn review_at_startup(
  db: &hyphen_db::HyphenDb,
  pool_total: usize,
  tip_height: Option<u32>,
) {
  let diag = match db.status().storage_diag().await {
    Ok(diag) => diag,
    Err(error) => {
      warn!(%error, "could not read the server settings; skipping the startup review");
      return;
    }
  };

  info!(
    server_version = %diag.server_version,
    shared_buffers = %diag.shared_buffers,
    effective_cache_size = %diag.effective_cache_size,
    work_mem = %diag.work_mem,
    max_connections = diag.max_connections,
    pool_total,
    "server settings in effect"
  );

  for finding in review(&diag, pool_total) {
    warn!(setting = finding.setting, "{}", finding.message);
  }

  // The WAL projection, with its inputs, so the number can be checked rather
  // than believed.
  let max_reorg_depth = hyphen_db::tuning::max_reorg_depth();
  if let Some(tip) = tip_height.filter(|tip| *tip > 0) {
    let bytes_per_block = u64::try_from(diag.database_bytes).unwrap_or(0) / u64::from(tip);
    let projected = projected_reorg_wal_bytes(bytes_per_block, max_reorg_depth);
    if projected > 0 {
      info!(
        max_reorg_depth,
        bytes_per_block,
        projected_peak_wal = %human_bytes(projected),
        "a reorg at max_reorg_depth projects this much WAL; \
         max_wal_size should exceed it (the daemon cannot read max_wal_size — \
         the storage crate does not expose it)"
      );
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;

  fn diag(shared: &str, cache: &str, work: &str, max_conn: i64) -> HyDbQrStorageDiag {
    HyDbQrStorageDiag {
      server_version: "18.0".to_owned(),
      database_bytes: 0,
      wal_bytes: 0,
      connections: 0,
      max_connections: max_conn,
      autovacuum_running: 0,
      shared_buffers: shared.to_owned(),
      work_mem: work.to_owned(),
      effective_cache_size: cache.to_owned(),
    }
  }

  #[test]
  fn pg_memory_settings_parse_in_every_unit_postgres_uses() {
    assert_eq!(parse_pg_bytes("8GB"), Some(8 * 1024 * 1024 * 1024));
    assert_eq!(parse_pg_bytes("64MB"), Some(64 * 1024 * 1024));
    assert_eq!(parse_pg_bytes("16384kB"), Some(16_384 * 1024));
    assert_eq!(parse_pg_bytes("1TB"), Some(1024_u64.pow(4)));
    assert_eq!(parse_pg_bytes(" 512 "), Some(512));
    // Unrecognised shapes yield nothing rather than a confident wrong number.
    assert_eq!(parse_pg_bytes("some"), None);
    assert_eq!(parse_pg_bytes("8 pages"), None);
    assert_eq!(parse_pg_bytes(""), None);
  }

  /// The test rig's settings must review clean, or the warnings are noise.
  #[test]
  fn the_test_rig_settings_review_clean() {
    let findings = review(&diag("8GB", "24GB", "64MB", 200), 40);
    assert!(findings.is_empty(), "unexpected findings: {findings:?}");
  }

  #[test]
  fn pools_that_cannot_fit_are_reported() {
    let findings = review(&diag("8GB", "24GB", "64MB", 200), 260);
    assert!(findings
      .iter()
      .any(|f| f.setting == "max_connections" && f.message.contains("cannot open")));

    // Fitting, but with no headroom left for autovacuum or a human.
    let findings = review(&diag("8GB", "24GB", "64MB", 200), 195);
    assert!(findings.iter().any(|f| f.setting == "max_connections"));
  }

  #[test]
  fn a_cache_size_below_shared_buffers_is_a_contradiction() {
    let findings = review(&diag("8GB", "4GB", "64MB", 200), 40);
    assert!(findings
      .iter()
      .any(|f| f.setting == "effective_cache_size" && f.message.contains("cannot")));
  }

  #[test]
  fn worst_case_work_mem_is_measured_against_the_pools() {
    // 64MB x 300 = 18.75GB, over half of 24GB.
    let findings = review(&diag("8GB", "24GB", "64MB", 400), 300);
    assert!(findings.iter().any(|f| f.setting == "work_mem"));
  }

  #[test]
  fn an_unparseable_setting_is_reported_not_skipped() {
    let findings = review(&diag("lots", "24GB", "64MB", 200), 40);
    assert!(findings
      .iter()
      .any(|f| f.setting == "shared_buffers" && f.message.contains("could not parse")));
  }

  /// F4's motivating case: production ran max_wal_size=4GB against a reorg
  /// depth whose projected WAL is far larger.
  #[test]
  fn the_reorg_wal_projection_exceeds_a_4gb_max_wal_size() {
    // ~2.5 MiB of database per block, at max_reorg_depth = 2000.
    let projected = projected_reorg_wal_bytes(2_500_000, 2_000);
    assert_eq!(projected, 10_000_000_000);
    assert!(
      projected > 4 * 1024 * 1024 * 1024,
      "the projection must exceed production's 4GB max_wal_size"
    );
    // Degenerate inputs do not produce a scary number.
    assert_eq!(projected_reorg_wal_bytes(0, 2_000), 0);
    assert_eq!(projected_reorg_wal_bytes(2_500_000, -1), 0);
  }
}
