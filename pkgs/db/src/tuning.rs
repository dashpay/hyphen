//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! `[db.ingest]`: the runtime tunables of the write path.
//!
//! These are the opposite of [`crate::layout`] in every respect that matters.
//! A layout parameter reinterprets bytes already written, so it is recorded
//! and enforced; an ingest tunable changes only how fast — and how gently —
//! the next block is written, so it is freely mutable and takes effect on the
//! next start with no consequence for what is already on disk.
//!
//! # Storage parameters are overrides, not settings
//!
//! Four of the keys (`vacuum_scale_factor`, `vacuum_analyze_factor`,
//! `vacuum_cost_delay`, `fillfactor`) name per-table storage parameters that
//! the schema already sets, with values that were measured rather than
//! guessed (see the commentary in `sql/0001_schema.sql`). Nothing here
//! replaces those numbers: the schema stays the baseline, and this module
//! emits `ALTER TABLE … SET` **only for a parameter whose resolved value
//! differs from the baseline**. A default configuration therefore emits no
//! DDL at all and the database is bit-for-bit what the migration created —
//! which is asserted by a test that reads the baseline back out of the
//! shipped `0001_schema.sql`.
//!
//! # Why a profile
//!
//! Vacuum behaviour is not four independent knobs. "Relax vacuum during the
//! initial sync" means a large scale factor *and* an analyze factor
//! that stays small enough to keep the planner honest, and getting one of the
//! two wrong is worse than changing neither. So `tuning_profile` resolves the
//! set coherently and the individual keys exist to override a member of the
//! resolved set, not to be the primary interface.

use std::sync::OnceLock;

use serde::{Deserialize, Serialize};
use sqlx::{Executor as _, PgPool};

use crate::error::{HyDbError, HyDbResult};

/// Enable stepped sync when the gap to the node's tip is large.
pub const DEFAULT_STEPPED_SYNC: bool = true;
/// Block gap at or above which stepped sync engages. Must stay below
/// `[db.layout] partition_span`: stepped mode defers the query-only indexes of
/// the *open* partition and builds them when it closes, so a gap that could
/// span more than one partition boundary has no single partition to close.
pub const DEFAULT_STEPPED_SYNC_GAP: i64 = 25_000;
/// Blocks connected per `advance` call.
pub const DEFAULT_MAX_ADVANCE_BLOCKS: usize = 2_000;
/// Deepest reorg the pipeline will unwind.
pub const DEFAULT_MAX_REORG_DEPTH: i64 = 1_000;
/// Blocks of undo pre-images retained below the tip.
pub const DEFAULT_UNDO_KEEP_DEPTH: i32 = 1_000;
/// Rows the writer will place in one multi-row statement.
pub const DEFAULT_BATCH_ROWS: usize = 10_000;
/// Row count at or above which a batch is loaded with `COPY`.
pub const DEFAULT_COPY_THRESHOLD: usize = 5_000;

/// `autovacuum_vacuum_scale_factor` for the vacuum-tuned tables under the
/// `sync` profile. Ten times the steady-state 0.02: during a cold sync the
/// churny table is being appended to far faster than a vacuum can keep pace
/// with, and the pass that matters is the one after the sync, not the twenty
/// during it.
pub const SYNC_VACUUM_SCALE_FACTOR: f64 = 0.2;
/// `autovacuum_analyze_scale_factor` under the `sync` profile. Deliberately
/// *not* relaxed alongside the vacuum factor: turning off statistics
/// collection during a sync is how a table arrives at the end of one with a
/// plan cliff. This is the half of the pair an operator setting knobs
/// individually gets wrong.
pub const SYNC_ANALYZE_SCALE_FACTOR: f64 = 0.05;

/// Which coherent set of vacuum/storage parameters to run.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HyDbTuningProfile {
  /// Cold-sync settings: vacuum runs rarely and finishes quickly, analyze
  /// keeps running.
  Sync,
  /// The measured steady-state settings the schema was committed with.
  Steady,
  /// Follow the ingest mode: `sync` while stepped sync is engaged, `steady`
  /// otherwise.
  ///
  /// `steady` is what a database that has never been in stepped mode runs,
  /// and it is exactly what the schema already carries, so it emits nothing.
  /// [`crate::stepped`] is what moves it: every mode transition re-applies
  /// the plan for the mode now in force.
  #[default]
  Auto,
}

impl HyDbTuningProfile {
  /// The profile actually in force, given whether stepped sync is engaged.
  pub const fn effective(self, stepped: bool) -> Self {
    match self {
      Self::Auto if stepped => Self::Sync,
      Self::Auto => Self::Steady,
      other => other,
    }
  }
}

/// `[db.ingest]`: how the write path behaves. Runtime only — nothing here is
/// recorded, compared, or able to invalidate a byte already written.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HyDbIngestCfg {
  /// Defer the open partition's query-only indexes while far behind the tip.
  pub stepped_sync: bool,
  /// Block gap at which stepped sync engages. Checked against
  /// `[db.layout] partition_span` at load — see [`HyDbIngestCfg::validate`].
  pub stepped_sync_gap: i64,
  /// Blocks connected per `advance` call.
  pub max_advance_blocks: usize,
  /// Deepest reorg the pipeline will unwind. Must not exceed
  /// `undo_keep_depth`: a disconnect replays undo rows, and undo below the
  /// keep depth has been pruned.
  pub max_reorg_depth: i64,
  /// Blocks of undo pre-images retained below the tip.
  pub undo_keep_depth: i32,
  /// Rows per multi-row write statement.
  pub batch_rows: usize,
  /// Row count at or above which a batch is loaded with `COPY`.
  pub copy_threshold: usize,
  /// Which coherent set of vacuum/storage parameters to run.
  pub tuning_profile: HyDbTuningProfile,
  /// Override `autovacuum_vacuum_scale_factor` on the vacuum-tuned tables.
  /// Unset means "whatever the profile resolves", which for `steady` is the
  /// value the schema was committed with.
  pub vacuum_scale_factor: Option<f64>,
  /// Override `autovacuum_analyze_scale_factor` on the vacuum-tuned tables.
  pub vacuum_analyze_factor: Option<f64>,
  /// Override `autovacuum_vacuum_cost_delay`, in milliseconds.
  pub vacuum_cost_delay: Option<i64>,
  /// Override `fillfactor` on the tables that declare one. Blunt by
  /// construction: it applies to every table with a committed fillfactor, and
  /// only to those, so it can never introduce the parameter somewhere the
  /// schema deliberately left it alone.
  pub fillfactor: Option<i64>,
}

impl Default for HyDbIngestCfg {
  fn default() -> Self {
    Self {
      stepped_sync: DEFAULT_STEPPED_SYNC,
      stepped_sync_gap: DEFAULT_STEPPED_SYNC_GAP,
      max_advance_blocks: DEFAULT_MAX_ADVANCE_BLOCKS,
      max_reorg_depth: DEFAULT_MAX_REORG_DEPTH,
      undo_keep_depth: DEFAULT_UNDO_KEEP_DEPTH,
      batch_rows: DEFAULT_BATCH_ROWS,
      copy_threshold: DEFAULT_COPY_THRESHOLD,
      tuning_profile: HyDbTuningProfile::Auto,
      vacuum_scale_factor: None,
      vacuum_analyze_factor: None,
      vacuum_cost_delay: None,
      fillfactor: None,
    }
  }
}

impl HyDbIngestCfg {
  /// Check the tunables in isolation, and against the one layout parameter
  /// they are coupled to.
  ///
  /// `partition_span` is passed in rather than read from
  /// [`crate::layout::resolved`] because the check has to run at *load*, before
  /// any connection exists and therefore before a layout has been resolved.
  /// [`crate::HyphenDb::connect`] runs it a second time against the resolved
  /// span, which is the authoritative one — the stored span can differ from
  /// the configured one, and the gap must be below whichever is in force.
  ///
  /// # Errors
  ///
  /// [`HyDbError::Config`] naming the offending key.
  pub fn validate(&self, partition_span: i32) -> HyDbResult<()> {
    fn positive(key: &str, value: i64) -> HyDbResult<()> {
      if value > 0 {
        return Ok(());
      }
      Err(HyDbError::Config(format!(
        "db.ingest.{key} must be at least 1, got {value}"
      )))
    }

    positive("max_advance_blocks", self.max_advance_blocks as i64)?;
    positive("max_reorg_depth", self.max_reorg_depth)?;
    positive("undo_keep_depth", i64::from(self.undo_keep_depth))?;
    positive("batch_rows", self.batch_rows as i64)?;
    positive("stepped_sync_gap", self.stepped_sync_gap)?;

    // The invariant the constants carried in a doc comment, now checkable:
    // a disconnect replays undo rows, and anything below the keep depth has
    // been pruned, so a reorg bound deeper than the retained window is a
    // promise the storage cannot keep.
    if self.max_reorg_depth > i64::from(self.undo_keep_depth) {
      return Err(HyDbError::Config(format!(
        "db.ingest.max_reorg_depth ({}) must not exceed undo_keep_depth ({}) — \
         a disconnect replays undo pre-images, and undo below the keep depth \
         has already been pruned",
        self.max_reorg_depth, self.undo_keep_depth
      )));
    }

    // Cross-section: stepped sync defers the indexes of the partition that is
    // open and rebuilds them when it closes. A gap wider than a partition has
    // no single open partition to close, so the mode would never complete a
    // build.
    if self.stepped_sync_gap >= i64::from(partition_span) {
      return Err(HyDbError::Config(format!(
        "db.ingest.stepped_sync_gap ({}) must be below db.layout.partition_span \
         ({partition_span}) — stepped sync defers the open partition's indexes \
         and builds them when that partition closes, so a gap that spans more \
         than one partition has no close to build at",
        self.stepped_sync_gap
      )));
    }

    if let Some(fillfactor) = self.fillfactor {
      if !(10..=100).contains(&fillfactor) {
        return Err(HyDbError::Config(format!(
          "db.ingest.fillfactor must be between 10 and 100, got {fillfactor}"
        )));
      }
    }
    for (key, value) in [
      ("vacuum_scale_factor", self.vacuum_scale_factor),
      ("vacuum_analyze_factor", self.vacuum_analyze_factor),
    ] {
      if let Some(value) = value {
        if !(0.0..=100.0).contains(&value) || !value.is_finite() {
          return Err(HyDbError::Config(format!(
            "db.ingest.{key} must be between 0 and 100, got {value}"
          )));
        }
      }
    }
    if let Some(delay) = self.vacuum_cost_delay {
      if !(0..=100).contains(&delay) {
        return Err(HyDbError::Config(format!(
          "db.ingest.vacuum_cost_delay is in milliseconds and must be between \
           0 and 100, got {delay}"
        )));
      }
    }
    Ok(())
  }
}

// ---------------------------------------------------------------------------
// Per-table storage parameters
// ---------------------------------------------------------------------------

/// A storage-parameter value. Numeric only, and rendered by this type rather
/// than taken from the configuration as text: these values are interpolated
/// into `ALTER TABLE … SET`, which takes no bind parameters, so the same rule
/// `[db.client]` follows applies — nothing reaches a statement that was not
/// parsed into a number first.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum StorageValue {
  Int(i64),
  Float(f64),
}

impl StorageValue {
  fn render(self) -> String {
    match self {
      Self::Int(v) => v.to_string(),
      Self::Float(v) => format!("{v}"),
    }
  }
}

/// One table's committed storage parameters, mirrored from
/// `sql/0001_schema.sql`. Mirrored rather than read from the file at runtime
/// because the schema is applied by a checksummed migration and cannot change
/// under a running binary; a test reads the file back and asserts the two
/// agree, which is what keeps the mirror honest.
#[derive(Debug)]
pub struct TunedTable {
  pub name: &'static str,
  pub baseline: &'static [(&'static str, StorageValue)],
  /// Whether the vacuum keys apply here. Set on the tables the schema already
  /// gives autovacuum parameters to; a table the schema deliberately left on
  /// the server defaults does not silently acquire per-table ones because an
  /// operator set a key.
  pub vacuum_tuned: bool,
}

/// `address_stats`: fillfactor 80, measured (A1/B5). No autovacuum overrides —
/// its win is HOT updates, which need vacuum to keep *running*.
const ADDRESS_STATS: TunedTable = TunedTable {
  name: "address_stats",
  baseline: &[("fillfactor", StorageValue::Int(80))],
  vacuum_tuned: false,
};

/// `utxo_set`: the one table that churns (B3/B5).
const UTXO_SET: TunedTable = TunedTable {
  name: "utxo_set",
  baseline: &[
    ("fillfactor", StorageValue::Int(70)),
    (
      "autovacuum_vacuum_scale_factor",
      StorageValue::Float(0.02_f64),
    ),
    ("autovacuum_vacuum_cost_limit", StorageValue::Int(2000)),
    ("autovacuum_vacuum_cost_delay", StorageValue::Int(2)),
  ],
  vacuum_tuned: true,
};

/// Every table whose storage parameters `[db.ingest]` can override.
pub const TUNED_TABLES: [TunedTable; 2] = [ADDRESS_STATS, UTXO_SET];

/// One `ALTER TABLE` the resolved configuration calls for.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StorageChange {
  pub table: &'static str,
  pub sql: String,
}

/// The `ALTER TABLE … SET` statements needed to move the database from the
/// schema's committed storage parameters to the ones this configuration
/// resolves to.
///
/// Empty for a default configuration. That is the contract: the schema's
/// numbers were measured, so the configuration's job is to *depart* from them
/// on request, never to restate them.
pub fn storage_plan(cfg: &HyDbIngestCfg, stepped: bool) -> Vec<StorageChange> {
  let profile = cfg.tuning_profile.effective(stepped);
  let mut plan = Vec::new();

  for table in &TUNED_TABLES {
    let mut diffs: Vec<String> = Vec::new();
    let mut push = |param: &str, want: StorageValue| {
      let base = table
        .baseline
        .iter()
        .find(|(name, _)| *name == param)
        .map(|(_, value)| *value);
      if base != Some(want) {
        diffs.push(format!("{param} = {}", want.render()));
      }
    };

    if let Some(fillfactor) = cfg.fillfactor {
      // Only where the schema already declares one.
      if table.baseline.iter().any(|(name, _)| *name == "fillfactor") {
        push("fillfactor", StorageValue::Int(fillfactor));
      }
    }

    if table.vacuum_tuned {
      let scale = cfg.vacuum_scale_factor.or(match profile {
        HyDbTuningProfile::Sync => Some(SYNC_VACUUM_SCALE_FACTOR),
        _ => None,
      });
      if let Some(scale) = scale {
        push("autovacuum_vacuum_scale_factor", StorageValue::Float(scale));
      }
      let analyze = cfg.vacuum_analyze_factor.or(match profile {
        HyDbTuningProfile::Sync => Some(SYNC_ANALYZE_SCALE_FACTOR),
        _ => None,
      });
      if let Some(analyze) = analyze {
        push(
          "autovacuum_analyze_scale_factor",
          StorageValue::Float(analyze),
        );
      }
      if let Some(delay) = cfg.vacuum_cost_delay {
        push("autovacuum_vacuum_cost_delay", StorageValue::Int(delay));
      }
    }

    if !diffs.is_empty() {
      plan.push(StorageChange {
        table: table.name,
        sql: format!("ALTER TABLE {} SET ({})", table.name, diffs.join(", ")),
      });
    }
  }

  plan
}

/// Apply [`storage_plan`] against the database.
///
/// Runs on the admin pool at connect. A default configuration executes
/// nothing at all, so a database opened by a daemon that was never configured
/// for tuning is untouched.
///
/// # Errors
///
/// [`HyDbError::Pg`] if a statement fails.
pub(crate) async fn apply_storage(pool: &PgPool, cfg: &HyDbIngestCfg) -> HyDbResult<usize> {
  apply_storage_for(pool, cfg, false).await
}

/// [`apply_storage`], for a known stepped-sync state.
///
/// Called again on every mode transition, which is what makes
/// `tuning_profile = auto` mean anything: the vacuum settings that suit a
/// cold sync are not the ones that suit the tip.
///
/// # Errors
///
/// [`HyDbError::Pg`] if a statement fails.
pub(crate) async fn apply_storage_for(
  pool: &PgPool,
  cfg: &HyDbIngestCfg,
  stepped: bool,
) -> HyDbResult<usize> {
  let plan = storage_plan(cfg, stepped);
  for change in &plan {
    pool
      .execute(change.sql.as_str())
      .await
      .map_err(|e| HyDbError::pg("tuning: apply storage parameters", e))?;
    tracing::info!(
      table = change.table,
      statement = change.sql.as_str(),
      "applied [db.ingest] storage tuning"
    );
  }
  Ok(plan.len())
}

// ---------------------------------------------------------------------------
// Process-wide resolved tunables
// ---------------------------------------------------------------------------

/// The tunables in force for this process.
///
/// A `OnceLock`, for the same reason [`crate::layout`] uses one: the write
/// path is a single writer configured once at start, and threading the
/// tunables through every stage would imply two ingest paths could disagree
/// about the reorg bound. Unlike the layout, nothing here is compared against
/// the database — a restart with different values is simply a different, and
/// equally valid, way to write the next block. Reads fall back to the
/// built-in defaults when nothing was installed, which is what unit tests and
/// mock facades want.
static INGEST: OnceLock<HyDbIngestCfg> = OnceLock::new();

/// Install the tunables for this process. Called by
/// [`crate::HyphenDb::connect`]. Installing the same values twice is fine;
/// installing different ones is not, for the same reason as the layout.
///
/// # Errors
///
/// [`HyDbError::Config`] when a *different* set is already installed.
pub fn install(cfg: HyDbIngestCfg) -> HyDbResult<()> {
  match INGEST.set(cfg.clone()) {
    Ok(()) => Ok(()),
    Err(_) => {
      if *resolved() == cfg {
        Ok(())
      } else {
        Err(HyDbError::Config(
          "this process already resolved a different [db.ingest] configuration".to_owned(),
        ))
      }
    }
  }
}

/// The tunables in force, or the built-in defaults if none were installed.
pub fn resolved() -> &'static HyDbIngestCfg {
  INGEST.get_or_init(HyDbIngestCfg::default)
}

/// Blocks connected per `advance` call.
pub fn max_advance_blocks() -> usize {
  resolved().max_advance_blocks
}

/// Whether stepped sync may engage at all.
pub fn stepped_sync() -> bool {
  resolved().stepped_sync
}

/// Gap to the node tip at which stepped sync engages. The gap it
/// *disengages* at is derived from this one rather than configured
/// separately — see [`crate::stepped::SteppedSync::with_params`].
pub fn stepped_sync_gap() -> i64 {
  resolved().stepped_sync_gap
}

/// Deepest reorg the pipeline will unwind.
pub fn max_reorg_depth() -> i64 {
  resolved().max_reorg_depth
}

/// Blocks of undo pre-images retained below the tip.
pub fn undo_keep_depth() -> i32 {
  resolved().undo_keep_depth
}

// ---------------------------------------------------------------------------
// Data-side enforcement of the retention promise
// ---------------------------------------------------------------------------

/// Verify that the undo rows **on disk** cover every height in `lo..=hi`,
/// before a rollback disconnects any of them.
///
/// [`HyDbIngestCfg::validate`] refuses `max_reorg_depth > undo_keep_depth` at
/// load, but that is a statement about the configuration, not about the
/// database the configuration was pointed at. The two diverge easily and
/// silently: lower `undo_span` and restart against an existing database, race
/// a prune against a config change, or simply raise `max_reorg_depth` on a
/// database that was synced under a shallower one. In every case the config
/// check passes and the promise is still unkeepable.
///
/// Without this, the first thing to notice is the disconnect trap in the
/// `stats` stage ("no `undo_blocks` row captured for height N") or the
/// `balances` stage ("moved address balances but has no undo rows") — and both
/// of those fire *mid-rollback*, with every height above the exhaustion point
/// already disconnected. This check moves the same refusal to before the first
/// disconnect, where it costs one query and nothing else.
///
/// `undo_blocks` is the right table to ask: its primary key is `height`, every
/// connect writes exactly one row, and the sibling undo tables are written in
/// the same transaction and pruned by the same partition drop — so a height
/// present here is a height whose undo survived, and a height missing here is
/// exactly the height a tip-first rollback would trap on. It is also what
/// makes this stronger than `status().undo_span()`, which reads MIN/MAX and so
/// can bound the retained range but cannot see a hole inside it.
///
/// The reported height is the **highest** missing one, because a rollback runs
/// tip-first and that is the height it would reach first.
///
/// # Errors
///
/// [`HyDbError::Derive`] naming the highest uncovered height, or
/// [`HyDbError::Pg`] if the query fails. `lo > hi` is an empty span and always
/// succeeds.
pub(crate) async fn verify_undo_coverage(pool: &PgPool, lo: i32, hi: i32) -> HyDbResult<()> {
  if lo > hi {
    return Ok(());
  }
  let row = sqlx::query!(
    r#"SELECT COUNT(*)::bigint AS "missing!", MAX(g.h) AS highest_missing
         FROM generate_series($1::int, $2::int) AS g(h)
        WHERE NOT EXISTS (SELECT 1 FROM undo_blocks u WHERE u.height = g.h)"#,
    lo,
    hi
  )
  .fetch_one(pool)
  .await
  .map_err(|e| HyDbError::pg("verify undo coverage", e))?;

  if row.missing != 0 {
    let at = row.highest_missing.unwrap_or(hi);
    return Err(HyDbError::Derive(format!(
      "undo coverage is incomplete for heights {lo}..={hi}: {} of {} heights have no \
       undo_blocks row, the highest at {at} — the rollback would have trapped there, \
       part-committed. Either the undo below it was pruned (raise [db.ingest] \
       undo_keep_depth, or target height {at} or above) or the projection was built \
       by a daemon that retained less than this one promises.",
      row.missing,
      i64::from(hi - lo) + 1
    )));
  }

  verify_undo_partitions(pool, lo, hi).await
}

/// Check that the whole undo family retains the span, not just `undo_blocks`.
///
/// The per-height probe above only works for `undo_blocks`, which carries a row
/// per block. `undo_address_stats` and `undo_validators` legitimately hold no
/// row for a height whose block touched no address or validator, so their
/// retention is a question about partitions rather than rows.
///
/// It has to be asked. The three families share one horizon, and a span covered
/// in `undo_blocks` but pruned in `undo_address_stats` reads as admissible here
/// and is then refused by the balance stage part-way down the disconnect —
/// exactly the stage trap this pre-flight exists to turn into a refusal.
///
/// # Errors
///
/// [`HyDbError::Derive`] naming the families and partitions that are missing,
/// or [`HyDbError::Pg`] if the probe fails.
async fn verify_undo_partitions(pool: &PgPool, lo: i32, hi: i32) -> HyDbResult<()> {
  let span = crate::layout::undo_span();
  let wanted: Vec<String> = (lo.max(0) / span..=hi.max(0) / span)
    .flat_map(|idx| crate::layout::UNDO_TABLES.map(move |parent| format!("{parent}_p{idx}")))
    .collect();

  let present: Vec<String> = sqlx::query_scalar!(
    r#"SELECT n AS "n!" FROM UNNEST($1::text[]) AS n WHERE to_regclass(n) IS NOT NULL"#,
    &wanted
  )
  .fetch_all(pool)
  .await
  .map_err(|e| HyDbError::pg("verify undo partitions", e))?;

  let missing: Vec<&String> = wanted.iter().filter(|n| !present.contains(n)).collect();
  if missing.is_empty() {
    return Ok(());
  }
  Err(HyDbError::Derive(format!(
    "undo coverage for heights {lo}..={hi} is complete in undo_blocks but {} undo \
     partition(s) are missing: {} — the rollback would have trapped where the \
     pre-images it needs were already dropped. Either re-sync, or target a height \
     above the range those partitions covered",
    missing.len(),
    missing
      .iter()
      .map(|n| n.as_str())
      .collect::<Vec<_>>()
      .join(", ")
  )))
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::layout::DEFAULT_PARTITION_SPAN;

  #[test]
  fn default_config_emits_no_ddl() {
    let plan = storage_plan(&HyDbIngestCfg::default(), false);
    assert!(
      plan.is_empty(),
      "a default [db.ingest] must leave the committed schema untouched, got {plan:?}"
    );
    // ...and explicitly restating the committed values is still no-op.
    let restated = HyDbIngestCfg {
      tuning_profile: HyDbTuningProfile::Steady,
      vacuum_scale_factor: Some(0.02),
      vacuum_cost_delay: Some(2),
      ..HyDbIngestCfg::default()
    };
    assert!(storage_plan(&restated, false).is_empty());
  }

  /// The baseline mirrored here must be the schema that actually ships. The
  /// test reads the migration rather than trusting the mirror, so a future
  /// edit to either one that forgets the other fails here instead of silently
  /// making `steady` mean something the database is not already at.
  #[test]
  fn baseline_matches_the_committed_schema() {
    let schema = std::fs::read_to_string(
      std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sql/0001_schema.sql"),
    )
    .unwrap_or_default();
    assert!(!schema.is_empty(), "0001_schema.sql should be readable");
    // Strip line comments: the file discusses alternative values it did not
    // take, at length.
    let ddl: String = schema
      .lines()
      .map(|line| line.split("--").next().unwrap_or(""))
      .collect::<Vec<_>>()
      .join("\n");
    for table in &TUNED_TABLES {
      for (param, value) in table.baseline {
        let needle = format!("{param} = {}", value.render());
        assert!(
          ddl.contains(&needle),
          "`{needle}` (baseline for {}) is not in the committed schema",
          table.name
        );
      }
    }
    // And nothing in the schema sets a storage parameter this module does not
    // know about, which would make `steady` an incomplete description.
    let declared = ddl.matches("fillfactor = ").count() + ddl.matches("autovacuum_").count();
    let mirrored: usize = TUNED_TABLES.iter().map(|t| t.baseline.len()).sum();
    assert_eq!(
      declared, mirrored,
      "the schema declares {declared} storage parameters but {mirrored} are mirrored here"
    );
  }

  #[test]
  fn sync_and_steady_resolve_differently() {
    let sync = HyDbIngestCfg {
      tuning_profile: HyDbTuningProfile::Sync,
      ..HyDbIngestCfg::default()
    };
    let steady = HyDbIngestCfg {
      tuning_profile: HyDbTuningProfile::Steady,
      ..HyDbIngestCfg::default()
    };
    let sync_plan = storage_plan(&sync, false);
    assert!(storage_plan(&steady, false).is_empty());
    assert_eq!(sync_plan.len(), 1, "{sync_plan:?}");
    let sql = &sync_plan[0].sql;
    assert!(
      sql.contains("autovacuum_vacuum_scale_factor = 0.2"),
      "{sql}"
    );
    // The pair, not the knob: relaxing vacuum without keeping analyze is the
    // failure mode the profile exists to prevent.
    assert!(
      sql.contains("autovacuum_analyze_scale_factor = 0.05"),
      "{sql}"
    );
    assert_ne!(sync_plan, storage_plan(&steady, false));
  }

  #[test]
  fn auto_follows_the_stepped_boundary_and_is_steady_today() {
    let cfg = HyDbIngestCfg::default();
    assert_eq!(cfg.tuning_profile, HyDbTuningProfile::Auto);
    assert_eq!(
      cfg.tuning_profile.effective(false),
      HyDbTuningProfile::Steady
    );
    assert_eq!(cfg.tuning_profile.effective(true), HyDbTuningProfile::Sync);
    assert!(storage_plan(&cfg, false).is_empty());
    assert!(!storage_plan(&cfg, true).is_empty());
  }

  #[test]
  fn an_override_beats_the_profile_but_only_on_tuned_tables() {
    let cfg = HyDbIngestCfg {
      tuning_profile: HyDbTuningProfile::Sync,
      vacuum_scale_factor: Some(0.5),
      fillfactor: Some(60),
      ..HyDbIngestCfg::default()
    };
    let plan = storage_plan(&cfg, false);
    assert_eq!(plan.len(), 2, "{plan:?}");
    let address_stats = &plan[0];
    assert_eq!(address_stats.table, "address_stats");
    assert_eq!(
      address_stats.sql, "ALTER TABLE address_stats SET (fillfactor = 60)",
      "a table the schema gives no autovacuum parameters must not acquire any"
    );
    assert!(plan[1].sql.contains("autovacuum_vacuum_scale_factor = 0.5"));
  }

  #[test]
  fn stepped_sync_gap_must_be_below_the_partition_span() {
    let cfg = HyDbIngestCfg {
      stepped_sync_gap: DEFAULT_PARTITION_SPAN.into(),
      ..HyDbIngestCfg::default()
    };
    let err = cfg.validate(DEFAULT_PARTITION_SPAN);
    assert!(
      matches!(&err, Err(HyDbError::Config(msg)) if msg.contains("stepped_sync_gap")
        && msg.contains("partition_span")),
      "a gap at the partition span must be refused, got {err:?}"
    );
    // A smaller stored span refuses a gap the default span would have allowed.
    assert!(HyDbIngestCfg::default().validate(10_000).is_err());
    assert!(HyDbIngestCfg::default()
      .validate(DEFAULT_PARTITION_SPAN)
      .is_ok());
  }

  #[test]
  fn reorg_bound_may_not_exceed_the_retained_undo() {
    let cfg = HyDbIngestCfg {
      max_reorg_depth: 2_000,
      undo_keep_depth: 1_000,
      ..HyDbIngestCfg::default()
    };
    assert!(cfg.validate(DEFAULT_PARTITION_SPAN).is_err());
  }

  #[test]
  fn validate_rejects_out_of_range_storage_parameters() {
    for cfg in [
      HyDbIngestCfg {
        fillfactor: Some(5),
        ..HyDbIngestCfg::default()
      },
      HyDbIngestCfg {
        vacuum_scale_factor: Some(-1.0),
        ..HyDbIngestCfg::default()
      },
      HyDbIngestCfg {
        vacuum_cost_delay: Some(1_000),
        ..HyDbIngestCfg::default()
      },
    ] {
      assert!(cfg.validate(DEFAULT_PARTITION_SPAN).is_err(), "{cfg:?}");
    }
  }

  #[test]
  fn defaults_reproduce_the_constants_they_replaced() {
    let cfg = HyDbIngestCfg::default();
    assert_eq!(cfg.max_advance_blocks, 2_000);
    assert_eq!(cfg.max_reorg_depth, 1_000);
    assert_eq!(cfg.undo_keep_depth, 1_000);
  }

  #[test]
  fn every_ingest_key_fits_the_24_character_budget() {
    const KEYS: [&str; 12] = [
      "stepped_sync",
      "stepped_sync_gap",
      "max_advance_blocks",
      "max_reorg_depth",
      "undo_keep_depth",
      "batch_rows",
      "copy_threshold",
      "tuning_profile",
      "vacuum_scale_factor",
      "vacuum_analyze_factor",
      "vacuum_cost_delay",
      "fillfactor",
    ];
    for key in KEYS {
      assert!(key.len() <= 24, "config key `{key}` is {} chars", key.len());
    }
  }

  #[test]
  fn profile_parses_from_the_three_documented_spellings() {
    for (text, want) in [
      ("sync", HyDbTuningProfile::Sync),
      ("steady", HyDbTuningProfile::Steady),
      ("auto", HyDbTuningProfile::Auto),
    ] {
      let parsed: Result<HyDbIngestCfg, _> =
        serde_json::from_str(&format!(r#"{{"tuning_profile":"{text}"}}"#));
      assert!(parsed.is_ok(), "`{text}` should parse");
      assert_eq!(parsed.unwrap_or_default().tuning_profile, want, "{text}");
    }
    assert!(serde_json::from_str::<HyDbIngestCfg>(r#"{"tuning_profile":"fast"}"#).is_err());
    assert!(serde_json::from_str::<HyDbIngestCfg>(r#"{"nonesuch":1}"#).is_err());
  }
}
