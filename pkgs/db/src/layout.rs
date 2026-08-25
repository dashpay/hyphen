//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Storage-layout parameters: the values that shape what the database
//! *contains*, as opposed to how it is connected to.
//!
//! A layout parameter is fixed for the life of a database. Changing one does
//! not reconfigure anything — it invalidates data that has already been
//! written, because the bytes on disk were produced under the old value. So
//! the contract here is not "read a setting", it is:
//!
//! 1. every parameter is written into `init_params` the first time a database
//!    is used, together with the daemon version that wrote it;
//! 2. on every later startup the configured set is compared against the
//!    stored set, and a disagreement refuses to start;
//! 3. the refusal names *every* differing key, not the first, and states the
//!    remedy that actually applies to that key.
//!
//! # Grades
//!
//! Not every immutable parameter is immutable in the same way, and telling an
//! operator to resync 84 GB when a single side table needs rebuilding is a
//! false alarm that trains them to ignore the real ones.
//!
//! - [`HyDbLayoutGrade::Hard`] — the stored bytes were physically laid out or
//!   encoded under the old value. Repartitioning every height-partitioned
//!   table is a full rewrite; re-encoding every address is a full re-derive.
//!   There is no path but a resync into a fresh database.
//! - [`HyDbLayoutGrade::Rebuildable`] — the affected object is *derived*, so
//!   it can be recomputed offline without touching the base facts. Cheap,
//!   local, and emphatically not a resync.
//! - [`HyDbLayoutGrade::Managed`] — not operator-owned. `schema_version`
//!   tracks the embedded migration set and is advanced by the migrator; only
//!   a *downgrade* (database newer than the binary) is refused.
//!
//! # `defaultable`
//!
//! A key present in the configuration but absent from the store means the
//! store predates the key. Adopting the configured value silently is only
//! safe when a wrong adoption cannot misdescribe facts already written. That
//! is exactly the rebuildable set: a rebuildable key's worst case is a stale
//! derived table, and the grade already names the repair. Hard keys describe
//! how base facts were written, so they are refused and the operator is asked
//! to state the value explicitly.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::OnceLock;

use sqlx::PgPool;

use crate::error::{HyDbError, HyDbResult};

/// Default network name. The layout records it so a testnet database can
/// never be opened by a mainnet-configured daemon (and vice versa).
pub const DEFAULT_NETWORK: &str = "mainnet";

/// Default height span of one *base* partition, in blocks.
///
/// The five height-partitioned base tables (`transactions`, `tx_inputs`,
/// `tx_outputs`, `address_txs`, `spends`) are created ahead of the tip at this
/// span. It is a hard parameter: changing it after any block is written means
/// rewriting every row into differently-bounded relations.
pub const DEFAULT_PARTITION_SPAN: i32 = 100_000;

/// Default Dash mainnet P2PKH address version byte.
pub const DEFAULT_P2PKH_VERSION: u8 = 0x4c;

/// Default Dash mainnet P2SH address version byte.
pub const DEFAULT_P2SH_VERSION: u8 = 0x10;

/// Default minimum balance (in duffs) for an address to be carried in
/// `address_rich`, the maintained rich-list side table.
///
/// The rich list is served by ranking *within* this set, so the floor must sit
/// far enough below any depth a caller can reach that truncation is not
/// observable in practice; it must never be "the Nth balance", because that
/// value moves with every block. At 10 DASH the mainnet set is ~33,200
/// addresses (measured at height 967,013) against ~785,600 with any positive
/// balance — 66 full pages at the rich-list API's 500-row page cap, and 4% of
/// the rows a balance-indexed `address_stats` would have had to carry.
pub const DEFAULT_RICH_LIST_FLOOR: i64 = 1_000_000_000;

/// Default height span of one *undo* partition.
///
/// The undo family is pruned by dropping whole partitions rather than by
/// deleting rows, so the span is a straight trade between retention and DDL
/// rate — nothing else. A partition may only be dropped once its highest
/// possible height is below the reorg horizon, so the retained window is
/// `keep_depth` blocks plus up to one span of overshoot.
///
/// At 500 against the 1000-block horizon that is 1000–1499 blocks retained
/// (~1.5x the strict minimum, ~145 MB at mainnet's measured 97 MB/1000
/// blocks) held in three or four live partitions, with one create and one
/// drop per 500 blocks. Matching the span to the horizon (1000) would double
/// retention for no reduction in work worth having; a quarter of it (250)
/// would buy 0.25x of retention back at double the DDL rate and twice the
/// relation count on a table the ingest inserts into on every block.
///
/// It is deliberately *not* derived from the keep depth: the drop rule is
/// keyed on the prune cutoff, so a larger configured depth lowers the cutoff
/// and drops strictly fewer partitions. Span and depth cannot conspire to
/// drop a partition early.
pub const DEFAULT_UNDO_SPAN: i32 = 500;

/// How badly a changed layout parameter breaks an existing database, and
/// therefore what the operator is told to do about it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HyDbLayoutGrade {
  /// Already-written base facts were laid out or encoded under the stored
  /// value. Only a resync into a fresh database can honour a new one.
  Hard,
  /// Only a *derived* object depends on the value; it can be rebuilt offline
  /// from the base facts, with no resync.
  Rebuildable,
  /// Maintained by the daemon, not the operator.
  Managed,
}

impl HyDbLayoutGrade {
  /// Short label used in refusal messages and `--show-layout`.
  pub const fn label(self) -> &'static str {
    match self {
      Self::Hard => "hard",
      Self::Rebuildable => "rebuildable",
      Self::Managed => "managed",
    }
  }
}

/// Static description of one layout key: its name, its grade, whether it may
/// be adopted silently when missing from the store, and the remedy sentence
/// printed when it disagrees with the store.
#[derive(Debug, Clone, Copy)]
pub struct HyDbLayoutKey {
  pub name: &'static str,
  pub grade: HyDbLayoutGrade,
  pub defaultable: bool,
  pub remedy: &'static str,
}

pub const KEY_NETWORK: HyDbLayoutKey = HyDbLayoutKey {
  name: "network",
  grade: HyDbLayoutGrade::Hard,
  defaultable: false,
  remedy: "every hash, address and height in this database belongs to the stored \
           network; point the daemon at a database for that network, or resync \
           into a fresh one",
};

pub const KEY_PARTITION_SPAN: HyDbLayoutKey = HyDbLayoutKey {
  name: "partition_span",
  grade: HyDbLayoutGrade::Hard,
  defaultable: false,
  remedy: "the five height-partitioned base tables already have partitions bounded \
           at the stored span; repartitioning is a full rewrite of every row, so \
           the only path to a new span is a resync into a fresh database",
};

pub const KEY_P2PKH_VERSION: HyDbLayoutKey = HyDbLayoutKey {
  name: "p2pkh_version",
  grade: HyDbLayoutGrade::Hard,
  defaultable: false,
  remedy: "every address string stored by the address index was encoded with the \
           stored version byte; re-encoding them is a full re-derive, so the only \
           path to a new version is a resync into a fresh database",
};

pub const KEY_P2SH_VERSION: HyDbLayoutKey = HyDbLayoutKey {
  name: "p2sh_version",
  grade: HyDbLayoutGrade::Hard,
  defaultable: false,
  remedy: "every address string stored by the address index was encoded with the \
           stored version byte; re-encoding them is a full re-derive, so the only \
           path to a new version is a resync into a fresh database",
};

pub const KEY_RICH_LIST_FLOOR: HyDbLayoutKey = HyDbLayoutKey {
  name: "rich_list_floor",
  grade: HyDbLayoutGrade::Rebuildable,
  // Not defaultable, unlike every other `Rebuildable` key, because data was
  // already written against it. `address_rich` was materialised at whatever
  // floor was in force, so adopting a configured floor for a database that
  // never recorded one leaves every address between the two floors in the
  // table, served on the rich list, with nothing to notice — which is the
  // offline rebuild the remedy below describes, silently skipped.
  defaultable: false,
  remedy: "NO RESYNC IS NEEDED: `address_rich` holds no facts of its own — every \
           row is a projection of `address_stats`, so a new floor is an offline \
           rebuild of that one table in a single scan (repopulate `address_rich` \
           from `address_stats` at the new floor, then set the stored value), or \
           restore the stored floor to start now",
};

pub const KEY_UNDO_SPAN: HyDbLayoutKey = HyDbLayoutKey {
  name: "undo_span",
  grade: HyDbLayoutGrade::Rebuildable,
  defaultable: false,
  remedy: "NO RESYNC IS NEEDED: the live undo partitions are bounded at the stored \
           span and new ones at the configured span would overlap them; the undo \
           family carries only reorg-reversal pre-images, so dropping the undo \
           partitions and setting the stored value is enough — the cost is reorg \
           depth, which is re-accumulated as the tip advances, not chain data",
};

pub const KEY_SCHEMA_VERSION: HyDbLayoutKey = HyDbLayoutKey {
  name: "schema_version",
  grade: HyDbLayoutGrade::Managed,
  defaultable: true,
  remedy: "this database was written by a newer schema than this binary embeds; \
           upgrade hyphend rather than downgrading the database",
};

/// Every layout key, in report order.
pub const LAYOUT_KEYS: [HyDbLayoutKey; 7] = [
  KEY_NETWORK,
  KEY_PARTITION_SPAN,
  KEY_P2PKH_VERSION,
  KEY_P2SH_VERSION,
  KEY_RICH_LIST_FLOOR,
  KEY_UNDO_SPAN,
  KEY_SCHEMA_VERSION,
];

/// The `[db.layout]` section as the operator wrote it. Every field is
/// optional, because *absence* is meaningful: a key the operator did not
/// state is taken from the store (case 5) rather than silently overridden by
/// a built-in default.
#[derive(Debug, Clone, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct HyDbLayoutCfg {
  pub network: Option<String>,
  pub partition_span: Option<i32>,
  pub p2pkh_version: Option<u8>,
  pub p2sh_version: Option<u8>,
  pub rich_list_floor: Option<i64>,
  pub undo_span: Option<i32>,
  /// Normally left unset: the daemon fills it from the embedded migration set.
  pub schema_version: Option<i64>,
}

impl HyDbLayoutCfg {
  fn configured(&self, key: &HyDbLayoutKey) -> Option<String> {
    match key.name {
      "network" => self.network.clone(),
      "partition_span" => self.partition_span.map(|v| v.to_string()),
      "p2pkh_version" => self.p2pkh_version.map(|v| v.to_string()),
      "p2sh_version" => self.p2sh_version.map(|v| v.to_string()),
      "rich_list_floor" => self.rich_list_floor.map(|v| v.to_string()),
      "undo_span" => self.undo_span.map(|v| v.to_string()),
      "schema_version" => self.schema_version.map(|v| v.to_string()),
      _ => None,
    }
  }
}

/// The resolved layout: what this database actually is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HyDbLayout {
  pub network: String,
  pub partition_span: i32,
  pub p2pkh_version: u8,
  pub p2sh_version: u8,
  pub rich_list_floor: i64,
  pub undo_span: i32,
  pub schema_version: i64,
}

impl Default for HyDbLayout {
  fn default() -> Self {
    Self {
      network: DEFAULT_NETWORK.to_owned(),
      partition_span: DEFAULT_PARTITION_SPAN,
      p2pkh_version: DEFAULT_P2PKH_VERSION,
      p2sh_version: DEFAULT_P2SH_VERSION,
      rich_list_floor: DEFAULT_RICH_LIST_FLOOR,
      undo_span: DEFAULT_UNDO_SPAN,
      schema_version: 0,
    }
  }
}

impl HyDbLayout {
  fn value(&self, key: &HyDbLayoutKey) -> String {
    match key.name {
      "network" => self.network.clone(),
      "partition_span" => self.partition_span.to_string(),
      "p2pkh_version" => self.p2pkh_version.to_string(),
      "p2sh_version" => self.p2sh_version.to_string(),
      "rich_list_floor" => self.rich_list_floor.to_string(),
      "undo_span" => self.undo_span.to_string(),
      "schema_version" => self.schema_version.to_string(),
      _ => String::new(),
    }
  }

  fn set(&mut self, key: &HyDbLayoutKey, raw: &str) -> HyDbResult<()> {
    fn bad(name: &str, raw: &str, what: &str) -> HyDbError {
      HyDbError::Layout(format!(
        "layout key `{name}` has value `{raw}`, which is not a valid {what}"
      ))
    }
    match key.name {
      "network" => self.network = raw.to_owned(),
      "partition_span" => {
        self.partition_span = raw.parse().map_err(|_| bad(key.name, raw, "block count"))?;
      }
      "p2pkh_version" => {
        self.p2pkh_version = raw
          .parse()
          .map_err(|_| bad(key.name, raw, "version byte"))?;
      }
      "p2sh_version" => {
        self.p2sh_version = raw
          .parse()
          .map_err(|_| bad(key.name, raw, "version byte"))?;
      }
      "rich_list_floor" => {
        self.rich_list_floor = raw.parse().map_err(|_| bad(key.name, raw, "duff amount"))?;
      }
      "undo_span" => {
        self.undo_span = raw.parse().map_err(|_| bad(key.name, raw, "block count"))?;
      }
      "schema_version" => {
        self.schema_version = raw
          .parse()
          .map_err(|_| bad(key.name, raw, "migration version"))?;
      }
      other => return Err(HyDbError::Layout(format!("unknown layout key `{other}`"))),
    }
    Ok(())
  }

  /// Validate the resolved values in isolation (before any store comparison),
  /// so a nonsense configuration is rejected with a config error rather than
  /// a partition-creation failure a thousand blocks later.
  fn validate(&self) -> HyDbResult<()> {
    if self.partition_span <= 0 {
      return Err(HyDbError::Layout(format!(
        "db.layout.partition_span must be positive, got {}",
        self.partition_span
      )));
    }
    if self.undo_span <= 0 {
      return Err(HyDbError::Layout(format!(
        "db.layout.undo_span must be positive, got {}",
        self.undo_span
      )));
    }
    if self.rich_list_floor < 0 {
      return Err(HyDbError::Layout(format!(
        "db.layout.rich_list_floor must not be negative, got {}",
        self.rich_list_floor
      )));
    }
    if self.network.trim().is_empty() {
      return Err(HyDbError::Layout(
        "db.layout.network must not be empty".to_owned(),
      ));
    }
    Ok(())
  }
}

// ---------------------------------------------------------------------------
// Process-wide resolved layout
// ---------------------------------------------------------------------------

/// The layout resolved against the store on connect.
///
/// A process-wide `OnceLock` rather than a threaded parameter, and the reason
/// is the contract this module exists to enforce: the layout is a property of
/// *the database*, one per process, and it cannot change while the process
/// runs. Threading it would suggest two ingest paths could disagree about the
/// partition span, which is precisely the state [`verify`] refuses to allow.
/// Reads fall back to the built-in defaults when nothing has been installed,
/// which is what unit tests and mock facades want.
static RESOLVED: OnceLock<HyDbLayout> = OnceLock::new();

/// Storage for the fallback the reads above use.
///
/// Held apart from [`RESOLVED`] on purpose. Serving the default out of
/// `RESOLVED` itself — via `get_or_init` — meant any read before
/// [`install`] latched the default permanently, so the layout agreed with the
/// store was then refused as a conflicting second one, and every partition name
/// derived in between had been built from the wrong span.
static DEFAULTS: OnceLock<HyDbLayout> = OnceLock::new();

/// Whether a read has already been answered with the defaults.
///
/// [`install`] consults this: a caller that computed a partition boundary from
/// the default span before the real layout arrived did so against a database
/// that may not use it, and that cannot be repaired after the fact.
static DEFAULTS_SERVED: AtomicBool = AtomicBool::new(false);

/// Install the resolved layout for this process. Called by
/// [`crate::HyphenDb::connect`] once [`verify`] has agreed it with the store;
/// kept separate from `verify` so the contract itself can be tested against a
/// real database without mutating process state.
///
/// # Errors
///
/// [`HyDbError::Layout`] when a *different* layout is already installed: two
/// databases with different physical layouts cannot be served by one process.
pub fn install(layout: HyDbLayout) -> HyDbResult<()> {
  // A read answered from the defaults before the store was consulted is only
  // harmless while the two agree. Where they differ, something has already
  // derived a partition name or an undo horizon from a span this database does
  // not use, and no later install can undo that.
  if DEFAULTS_SERVED.load(Ordering::Acquire) && layout != HyDbLayout::default() {
    return Err(HyDbError::Layout(format!(
      "the built-in storage layout was read before this database's own layout \
       was resolved, and they differ (resolved={layout:?}); a partition \
       boundary derived from the wrong span cannot be corrected afterwards"
    )));
  }
  match RESOLVED.set(layout.clone()) {
    Ok(()) => Ok(()),
    Err(_) => {
      let current = resolved();
      if *current == layout {
        Ok(())
      } else {
        Err(HyDbError::Layout(format!(
          "this process already resolved a different storage layout \
           (installed={current:?}, requested={layout:?})"
        )))
      }
    }
  }
}

/// The resolved layout, or the built-in defaults if none was installed.
///
/// The fallback does not become the resolved layout. It is served from its own
/// cell and recorded, so [`install`] can still agree a real layout with the
/// store afterwards and can refuse the one case where the fallback has already
/// done damage.
pub fn resolved() -> &'static HyDbLayout {
  match RESOLVED.get() {
    Some(layout) => layout,
    None => {
      DEFAULTS_SERVED.store(true, Ordering::Release);
      DEFAULTS.get_or_init(HyDbLayout::default)
    }
  }
}

/// Height span of one base partition, in blocks.
pub fn partition_span() -> i32 {
  resolved().partition_span
}

/// Height span of one undo partition, in blocks.
pub fn undo_span() -> i32 {
  resolved().undo_span
}

/// The height-partitioned undo tables, pruned by dropping a whole partition.
///
/// Kept beside [`undo_span`] because the two are read together wherever undo
/// retention is decided or checked, and a caller that has one without the other
/// can only reason about part of the family.
pub(crate) const UNDO_TABLES: [&str; 3] = ["undo_address_stats", "undo_blocks", "undo_validators"];

/// Minimum balance (duffs) for `address_rich` membership.
pub fn rich_list_floor() -> i64 {
  resolved().rich_list_floor
}

// ---------------------------------------------------------------------------
// Store I/O
// ---------------------------------------------------------------------------

/// One stored row of `init_params`.
#[derive(Debug, Clone)]
pub struct HyDbInitParam {
  pub key: String,
  pub value: String,
  pub created_by: String,
  pub created_at: String,
}

/// Read `init_params`. `Ok(None)` means the table itself is absent — a
/// database written before the layout contract existed.
///
/// # Errors
///
/// [`HyDbError::Pg`] if the read fails.
pub async fn read_store(pool: &PgPool) -> HyDbResult<Option<Vec<HyDbInitParam>>> {
  let present: bool = sqlx::query_scalar("SELECT to_regclass('public.init_params') IS NOT NULL")
    .fetch_one(pool)
    .await
    .map_err(|e| HyDbError::pg("layout: probe init_params", e))?;
  if !present {
    return Ok(None);
  }
  let rows = sqlx::query_as::<_, (String, String, String, String)>(
    "SELECT key, value, created_by, to_char(created_at, 'YYYY-MM-DD\"T\"HH24:MI:SSOF') \
       FROM init_params ORDER BY key",
  )
  .fetch_all(pool)
  .await
  .map_err(|e| HyDbError::pg("layout: read init_params", e))?;
  Ok(Some(
    rows
      .into_iter()
      .map(|(key, value, created_by, created_at)| HyDbInitParam {
        key,
        value,
        created_by,
        created_at,
      })
      .collect(),
  ))
}

/// Record every layout key in one transaction.
///
/// Atomic because a partial layout is not classifiable. `plan` reads
/// `init_params` and decides first-run against established from what it finds;
/// a run that failed part-way through writing leaves a row set that is neither,
/// and the next start has to guess which keys were the database's own and which
/// were half-written.
///
/// # Errors
///
/// [`HyDbError::Pg`] if the transaction cannot be opened, written, or
/// committed. Nothing is recorded unless all of it is.
async fn write_params(
  pool: &PgPool,
  params: &[(&str, String)],
  created_by: &str,
) -> HyDbResult<()> {
  if params.is_empty() {
    return Ok(());
  }
  let mut tx = pool
    .begin()
    .await
    .map_err(|e| HyDbError::pg("layout: begin init_params write", e))?;
  for (key, value) in params {
    sqlx::query(
      "INSERT INTO init_params (key, value, created_by) VALUES ($1, $2, $3) \
         ON CONFLICT (key) DO UPDATE SET value = EXCLUDED.value",
    )
    .bind(key)
    .bind(value)
    .bind(created_by)
    .execute(&mut *tx)
    .await
    .map_err(|e| HyDbError::pg("layout: write init_params", e))?;
  }
  tx.commit()
    .await
    .map_err(|e| HyDbError::pg("layout: commit init_params write", e))?;
  Ok(())
}

// ---------------------------------------------------------------------------
// The five startup cases
// ---------------------------------------------------------------------------

/// What [`plan`] decided for one key.
#[derive(Debug, Clone, PartialEq, Eq)]
enum Decision {
  /// First run: record the value.
  Record(String),
  /// Stored and configured agree (or only the store has an opinion).
  Keep(String),
  /// Configured key absent from the store, and adoptable.
  Adopt(String),
  /// `schema_version` moved forward with the embedded migration set.
  Advance { stored: String, to: String },
  /// Stored value used because the operator did not state one.
  Dropped(String),
  /// Refusal, with the message to print.
  Refuse(String),
}

struct Planned {
  key: HyDbLayoutKey,
  decision: Decision,
}

/// Decide, key by key, what the resolved layout is and what to write back.
/// Pure: no I/O, so all five cases are unit-testable without a server.
fn plan(cfg: &HyDbLayoutCfg, stored: Option<&[HyDbInitParam]>) -> Vec<Planned> {
  let defaults = HyDbLayout::default();
  let first_run = stored.is_none_or(<[HyDbInitParam]>::is_empty);
  LAYOUT_KEYS
    .iter()
    .map(|key| {
      let configured = cfg.configured(key);
      let stored_value = stored
        .and_then(|rows| rows.iter().find(|r| r.key == key.name))
        .map(|r| r.value.clone());
      let fallback = defaults.value(key);
      let decision = match (configured, stored_value) {
        // Case 1 — nothing on record: this is a first run.
        (configured, None) if first_run => Decision::Record(configured.unwrap_or(fallback)),

        // Case 2 — both sides agree.
        (Some(c), Some(s)) if c == s => Decision::Keep(s),

        // Case 3 — both sides have an opinion and they differ.
        (Some(c), Some(s)) => match key.grade {
          // `schema_version` is managed: forward is an upgrade, back is not.
          HyDbLayoutGrade::Managed => {
            let (sv, cv) = (s.parse::<i64>().unwrap_or(0), c.parse::<i64>().unwrap_or(0));
            if cv >= sv {
              Decision::Advance { stored: s, to: c }
            } else {
              Decision::Refuse(mismatch_message(key, &s, &c))
            }
          }
          _ => Decision::Refuse(mismatch_message(key, &s, &c)),
        },

        // Case 4 — in the configuration, absent from a non-empty store.
        (Some(c), None) => {
          if key.defaultable {
            Decision::Adopt(c)
          } else {
            Decision::Refuse(unrecorded_message(key, &c))
          }
        }

        // Case 5 — in the store, absent from the configuration.
        (None, Some(s)) => Decision::Dropped(s),

        // Neither side states it and the store is not empty: the key was
        // introduced by this binary and nobody has an opinion. Same rule as
        // case 4, with the built-in default standing in for the config.
        (None, None) => {
          if key.defaultable {
            Decision::Adopt(fallback)
          } else {
            Decision::Refuse(unrecorded_message(key, &fallback))
          }
        }
      };
      Planned {
        key: *key,
        decision,
      }
    })
    .collect()
}

fn mismatch_message(key: &HyDbLayoutKey, stored: &str, configured: &str) -> String {
  format!(
    "  {name} [{grade}]: stored={stored} configured={configured}\n      {remedy}",
    name = key.name,
    grade = key.grade.label(),
    remedy = key.remedy,
  )
}

fn unrecorded_message(key: &HyDbLayoutKey, configured: &str) -> String {
  format!(
    "  {name} [{grade}]: stored=<absent> configured={configured}\n      \
     this database predates the `{name}` layout key, and adopting a value that \
     changes how already-written data is interpreted is not something the daemon \
     will do on its own. State the value this database was built with in \
     [db.layout], or {remedy}",
    name = key.name,
    grade = key.grade.label(),
    remedy = key.remedy,
  )
}

/// Resolve the configured layout against `init_params`, writing the store on a
/// first run and refusing to start on an incompatible change.
///
/// The four things this does, in the order an operator experiences them:
/// records a first run, accepts an unchanged one, adopts or refuses keys the
/// store has never seen, and — on a genuine change — reports **every**
/// differing key with its grade and its own remedy before failing.
///
/// # Errors
///
/// [`HyDbError::Layout`] when the configured layout is invalid or disagrees
/// with the store; [`HyDbError::Pg`] if `init_params` cannot be read or
/// written.
pub async fn verify(
  pool: &PgPool,
  cfg: &HyDbLayoutCfg,
  created_by: &str,
) -> HyDbResult<HyDbLayout> {
  let stored = read_store(pool).await?;
  let first_run = stored.as_deref().is_none_or(<[HyDbInitParam]>::is_empty);
  let planned = plan(cfg, stored.as_deref());

  let mut refusals: Vec<String> = Vec::new();
  let mut layout = HyDbLayout::default();
  let mut to_write: Vec<(&'static str, String)> = Vec::new();
  let mut adopted: Vec<(&'static str, String)> = Vec::new();
  let mut advanced: Vec<(&'static str, String, String)> = Vec::new();
  let mut dropped: Vec<(&'static str, String)> = Vec::new();

  for p in &planned {
    match &p.decision {
      Decision::Record(v) => {
        layout.set(&p.key, v)?;
        to_write.push((p.key.name, v.clone()));
      }
      Decision::Keep(v) => layout.set(&p.key, v)?,
      Decision::Adopt(v) => {
        layout.set(&p.key, v)?;
        to_write.push((p.key.name, v.clone()));
        adopted.push((p.key.name, v.clone()));
      }
      Decision::Advance { stored, to } => {
        layout.set(&p.key, to)?;
        to_write.push((p.key.name, to.clone()));
        advanced.push((p.key.name, stored.clone(), to.clone()));
      }
      Decision::Dropped(v) => {
        layout.set(&p.key, v)?;
        dropped.push((p.key.name, v.clone()));
      }
      Decision::Refuse(msg) => refusals.push(msg.clone()),
    }
  }

  if !refusals.is_empty() {
    return Err(HyDbError::Layout(format!(
      "the configured storage layout is incompatible with this database.\n\
       {count} key(s) disagree; ALL of them are listed so one restart resolves \
       all of them:\n{list}",
      count = refusals.len(),
      list = refusals.join("\n"),
    )));
  }

  layout.validate()?;

  // Anything unknown in the store is a key a *newer* daemon recorded. It is
  // not an error — this binary simply has no opinion — but it is worth saying
  // out loud, because it means the database has been opened by a newer build.
  if let Some(rows) = stored.as_deref() {
    for row in rows {
      if !LAYOUT_KEYS.iter().any(|k| k.name == row.key) {
        tracing::warn!(
          key = %row.key,
          value = %row.value,
          created_by = %row.created_by,
          "init_params holds a layout key this build does not know; ignoring it"
        );
      }
    }
  }

  write_params(pool, &to_write, created_by).await?;

  if first_run {
    tracing::info!(
      network = %layout.network,
      partition_span = layout.partition_span,
      p2pkh_version = layout.p2pkh_version,
      p2sh_version = layout.p2sh_version,
      rich_list_floor = layout.rich_list_floor,
      undo_span = layout.undo_span,
      schema_version = layout.schema_version,
      created_by,
      "first run: recorded the storage layout in init_params — these values are \
       fixed for the life of this database"
    );
  } else {
    for (key, value) in &adopted {
      tracing::info!(
        key,
        value,
        created_by,
        "adopted a layout key this database had never recorded (defaultable: no \
         already-written data depends on it)"
      );
    }
    for (key, from, to) in &advanced {
      tracing::info!(key, from = %from, to = %to, "layout key advanced by the migrator");
    }
    for (key, value) in &dropped {
      tracing::warn!(
        key,
        value,
        "[db.layout] does not state this key; using the value stored in \
         init_params. Restore it to the config to make the intent explicit."
      );
    }
    tracing::debug!(?layout, "storage layout matches init_params");
  }

  Ok(layout)
}

/// Render stored versus configured for `--show-layout`, without connecting a
/// full facade or applying anything.
///
/// # Errors
///
/// [`HyDbError::Pg`] if `init_params` cannot be read.
pub async fn show(pool: &PgPool, cfg: &HyDbLayoutCfg) -> HyDbResult<String> {
  let stored = read_store(pool).await?;
  let defaults = HyDbLayout::default();
  let mut out = String::new();
  match stored.as_deref() {
    None => out.push_str(
      "init_params: ABSENT (this database predates the layout contract, or has \
       never been migrated)\n\n",
    ),
    Some([]) => out.push_str("init_params: EMPTY (first run — nothing recorded yet)\n\n"),
    Some(rows) => {
      let by = rows.first().map_or("?", |r| r.created_by.as_str());
      let at = rows.first().map_or("?", |r| r.created_at.as_str());
      out.push_str(&format!(
        "init_params: {n} row(s), first written by {by} at {at}\n\n",
        n = rows.len()
      ));
    }
  }
  out.push_str(&format!(
    "{:<16} {:<13} {:<24} {:<24} {}\n",
    "KEY", "GRADE", "STORED", "CONFIGURED", "STATE"
  ));
  for p in plan(cfg, stored.as_deref()) {
    let key = p.key;
    let stored_value = stored
      .as_deref()
      .and_then(|rows| rows.iter().find(|r| r.key == key.name))
      .map(|r| r.value.clone())
      .unwrap_or_else(|| "-".to_owned());
    let configured = cfg
      .configured(&key)
      .unwrap_or_else(|| format!("({})", defaults.value(&key)));
    let state = match &p.decision {
      Decision::Record(_) => "would record".to_owned(),
      Decision::Keep(_) => "ok".to_owned(),
      Decision::Adopt(_) => "would adopt".to_owned(),
      Decision::Advance { .. } => "would advance".to_owned(),
      Decision::Dropped(_) => "using stored (not configured)".to_owned(),
      Decision::Refuse(_) => format!("REFUSED — {}", key.remedy),
    };
    out.push_str(&format!(
      "{:<16} {:<13} {:<24} {:<24} {}\n",
      key.name,
      key.grade.label(),
      stored_value,
      configured,
      state
    ));
  }
  out.push_str(
    "\nA configured value shown in (parentheses) is the built-in default: the \
     key is absent from [db.layout].\n",
  );
  Ok(out)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn param(key: &str, value: &str) -> HyDbInitParam {
    HyDbInitParam {
      key: key.to_owned(),
      value: value.to_owned(),
      created_by: "hyphend/test".to_owned(),
      created_at: "2026-01-01T00:00:00+00".to_owned(),
    }
  }

  fn full_store() -> Vec<HyDbInitParam> {
    LAYOUT_KEYS
      .iter()
      .map(|k| param(k.name, &HyDbLayout::default().value(k)))
      .collect()
  }

  fn full_cfg() -> HyDbLayoutCfg {
    let d = HyDbLayout::default();
    HyDbLayoutCfg {
      network: Some(d.network),
      partition_span: Some(d.partition_span),
      p2pkh_version: Some(d.p2pkh_version),
      p2sh_version: Some(d.p2sh_version),
      rich_list_floor: Some(d.rich_list_floor),
      undo_span: Some(d.undo_span),
      schema_version: Some(d.schema_version),
    }
  }

  fn decision(planned: &[Planned], name: &str) -> Decision {
    planned
      .iter()
      .find(|p| p.key.name == name)
      .map(|p| p.decision.clone())
      .unwrap_or(Decision::Keep("<missing>".to_owned()))
  }

  #[test]
  fn every_key_name_fits_the_24_character_budget() {
    for key in &LAYOUT_KEYS {
      assert!(
        key.name.len() <= 24,
        "layout key `{}` is {} characters",
        key.name,
        key.name.len()
      );
    }
  }

  #[test]
  fn case1_absent_store_records_every_key() {
    let planned = plan(&full_cfg(), None);
    assert!(planned
      .iter()
      .all(|p| matches!(p.decision, Decision::Record(_))));
  }

  #[test]
  fn case1_empty_store_records_every_key() {
    let planned = plan(&HyDbLayoutCfg::default(), Some(&[]));
    assert!(planned
      .iter()
      .all(|p| matches!(p.decision, Decision::Record(_))));
  }

  #[test]
  fn case2_matching_store_keeps_every_key() {
    let planned = plan(&full_cfg(), Some(&full_store()));
    assert!(planned
      .iter()
      .all(|p| matches!(p.decision, Decision::Keep(_))));
  }

  #[test]
  fn case3_reports_every_differing_key_not_just_the_first() {
    let mut cfg = full_cfg();
    cfg.network = Some("testnet".to_owned());
    cfg.partition_span = Some(50_000);
    cfg.rich_list_floor = Some(5_000_000_000);
    let planned = plan(&cfg, Some(&full_store()));
    let refusals: Vec<_> = planned
      .iter()
      .filter_map(|p| match &p.decision {
        Decision::Refuse(m) => Some(m.clone()),
        _ => None,
      })
      .collect();
    assert_eq!(refusals.len(), 3, "{refusals:#?}");
    assert!(refusals.iter().any(|m| m.contains("network")
      && m.contains("stored=mainnet configured=testnet")
      && m.contains("[hard]")));
    assert!(refusals
      .iter()
      .any(|m| m.contains("partition_span") && m.contains("resync")));
    // The rebuildable grade must not tell the operator to resync.
    let floor = refusals
      .iter()
      .find(|m| m.contains("rich_list_floor"))
      .cloned()
      .unwrap_or_default();
    assert!(floor.contains("[rebuildable]"), "{floor}");
    assert!(floor.contains("NO RESYNC IS NEEDED"), "{floor}");
    assert!(floor.contains("rebuild"), "{floor}");
  }

  #[test]
  fn case4_defaultable_key_is_adopted_and_hard_key_is_refused() {
    let store: Vec<_> = full_store()
      .into_iter()
      .filter(|p| p.key != "schema_version" && p.key != "partition_span")
      .collect();
    let planned = plan(&full_cfg(), Some(&store));
    assert!(matches!(
      decision(&planned, "schema_version"),
      Decision::Adopt(_)
    ));
    let msg = match decision(&planned, "partition_span") {
      Decision::Refuse(msg) => msg,
      other => format!("NOT REFUSED (hard key missing from the store): {other:?}"),
    };
    assert!(msg.contains("stored=<absent>"), "{msg}");
    assert!(
      msg.contains("State the value this database was built with"),
      "{msg}"
    );
  }

  /// `rich_list_floor` is `Rebuildable`, but data was written against it, so an
  /// unrecorded one is refused rather than adopted.
  ///
  /// Adopting it leaves `address_rich` holding every address between the floor
  /// it was materialised at and the one now configured, served on the rich list
  /// indefinitely. That is the offline rebuild the key's own remedy describes,
  /// skipped without anything saying so.
  #[test]
  fn an_unrecorded_rich_list_floor_is_refused_rather_than_adopted() {
    let store: Vec<_> = full_store()
      .into_iter()
      .filter(|p| p.key != "rich_list_floor")
      .collect();
    let planned = plan(&full_cfg(), Some(&store));
    let msg = match decision(&planned, "rich_list_floor") {
      Decision::Refuse(msg) => msg,
      other => format!("NOT REFUSED (address_rich was built at some floor): {other:?}"),
    };
    assert!(msg.contains("stored=<absent>"), "{msg}");
    assert!(msg.contains("rich_list_floor"), "{msg}");
  }

  #[test]
  fn case5_key_dropped_from_config_warns_and_uses_stored() {
    let mut store = full_store();
    for row in &mut store {
      if row.key == "rich_list_floor" {
        row.value = "7000000000".to_owned();
      }
    }
    let mut cfg = full_cfg();
    cfg.rich_list_floor = None;
    let planned = plan(&cfg, Some(&store));
    assert_eq!(
      decision(&planned, "rich_list_floor"),
      Decision::Dropped("7000000000".to_owned())
    );
  }

  #[test]
  fn schema_version_advances_forward_and_refuses_a_downgrade() {
    let mut store = full_store();
    for row in &mut store {
      if row.key == "schema_version" {
        row.value = "3".to_owned();
      }
    }
    let mut cfg = full_cfg();
    cfg.schema_version = Some(4);
    assert!(matches!(
      decision(&plan(&cfg, Some(&store)), "schema_version"),
      Decision::Advance { .. }
    ));
    cfg.schema_version = Some(2);
    let msg = match decision(&plan(&cfg, Some(&store)), "schema_version") {
      Decision::Refuse(msg) => msg,
      other => format!("NOT REFUSED (schema downgrade): {other:?}"),
    };
    assert!(msg.contains("stored=3 configured=2"), "{msg}");
  }

  #[test]
  fn invalid_values_are_rejected_before_the_store_is_consulted() {
    assert!(HyDbLayout {
      partition_span: 0,
      ..HyDbLayout::default()
    }
    .validate()
    .is_err());
    assert!(HyDbLayout {
      undo_span: -1,
      ..HyDbLayout::default()
    }
    .validate()
    .is_err());
    assert!(HyDbLayout {
      network: "  ".to_owned(),
      ..HyDbLayout::default()
    }
    .validate()
    .is_err());
  }

  #[test]
  fn defaultable_is_never_set_on_a_hard_key() {
    for key in &LAYOUT_KEYS {
      if key.grade == HyDbLayoutGrade::Hard {
        assert!(!key.defaultable, "hard key `{}` is defaultable", key.name);
      }
    }
  }
}
