//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Stepped sync: defer the open partition's query-only indexes while the tip
//! is far behind the node, and build them in one pass when that partition
//! closes.
//!
//! # Why partition boundaries and not an interval
//!
//! Building an index costs O(table), not O(delta). Rebuilding every N blocks
//! therefore rebuilds a structure that is itself growing, hundreds of times
//! over a chain. Partition-aligned means one build per partition — about 25
//! over mainnet — and each one is bounded by a partition, not by the chain.
//!
//! # What is deferrable
//!
//! Exactly the query-only indexes of the five height-partitioned tables, and
//! only on the partition currently being filled. Measured at
//! `address_txs_p7`-equivalent scale these are ~3 GB of random-write index
//! maintenance per partition:
//!
//! | index | size |
//! |---|---|
//! | `idx_address_txs_addr_height_desc` | 1,682 MB |
//! | `idx_tx_outputs_outpoint`          |   595 MB |
//! | `idx_tx_outputs_txid_vout`         |   594 MB |
//! | `idx_tx_outputs_address`           |   322 MB |
//! | `idx_transactions_display_txid`    |   178 MB |
//! | `idx_transactions_list`            |   174 MB |
//! | `idx_transactions_txid`            |   111 MB |
//!
//! The first of those is the only unique member, and the only one whose
//! absence relaxes a constraint rather than only a plan — see the section
//! below on why it is built `UNIQUE` at close rather than declared so.
//!
//! An index the ingest path *reads* is never in that set. Dropping one turns
//! the write path into sequential scans, which converts a slow sync into a
//! stopped one, so `address_stats_pkey`, the `utxo_set` keys, the primary keys
//! the partitioned tables still have, and `idx_validator_collateral` all stay.
//! (`address_txs` has none; the schema withholds it as a redundant key prefix
//! of the index above.) The unpartitioned tables cannot participate at all: any
//! rebuild there is full-size by construction.
//!
//! `idx_tx_outputs_outpoint` is the one member of the deferred set that ingest
//! reads at all, and only on *disconnect* — it serves the UTXO-restore join.
//! Stepped mode is only ever engaged when the tip is `stepped_sync_gap` blocks
//! behind the node, which is far deeper than `max_reorg_depth`, so a reversal
//! cannot reach the open partition while the mode holds. Should a reorg be
//! planned anyway, [`SteppedSync::force_realtime`] leaves stepped mode and
//! rebuilds before the disconnect runs, rather than letting it full-scan.
//!
//! # `address_txs` has one btree, and the build is what makes it unique
//!
//! `address_txs` used to carry two btrees whose keys are a prefix
//! relationship: `address_txs_pkey (address, block_height, txid)` and this
//! set's `idx_address_txs_addr_height_desc (address, block_height DESC,
//! block_pos DESC, txid DESC)`. The first is a strict key-prefix subset of the
//! second, and 8.8 GB of mainnet's 17.6 GB of `address_txs` index. Nothing read
//! it: the paginator needs all four columns in that order, and A5's narrowing
//! of the writer's dedup key to exactly the constraint's columns removed the
//! `ON CONFLICT` arbiter it existed to back.
//!
//! The schema does not declare it. That leaves the constraint with
//! nowhere to live, and the obvious repair — declaring the survivor `UNIQUE` —
//! is the wrong one for IBD: a unique index cannot be deferred, because
//! deferring it *is* not enforcing it, so the wide index would have to be
//! maintained on every insert and stepped mode would lose the whole
//! narrow-pkey-plus-bulk-build scheme.
//!
//! So the survivor stays deferred and is built with `CREATE UNIQUE INDEX` when
//! the partition closes. The build is then two things at once: the index, and a
//! retroactive proof that no duplicate key entered that partition. Enforcement
//! is per-partition during IBD and per-row near the tip, and verified either
//! way — if the writer's dedup breaks, or a new path bypasses it, the close
//! fails with the partition named rather than corrupting silently. What is
//! accepted in exchange: reads against the still-open partition can see a
//! duplicate before it closes, and a failed close needs an operator. Both are
//! the right failure mode, because a duplicate here is a bug worth stopping on.
//!
//! During stepped sync `address_txs` is consequently left maintaining exactly
//! one index — `idx_address_txs_block_height`, 325 MB over nine partitions and
//! append-only, which the schema keeps for the reversal delete. It was two, one
//! of them the 6.5 GB random-write primary key.
//!
//! The unique key is one column wider than the constraint it replaces, and that
//! is exact rather than approximate: `block_pos` is functionally determined by
//! `(block_height, txid)`, which is `transactions`' own primary key, so a
//! genuine duplicate `(address, block_height, txid)` carries the same
//! `block_pos` and collides. The one shape that escapes is a duplicate whose
//! rows disagree about `block_pos` — a writer that has already corrupted the
//! position it copies — and no single index over the paginator's ordering can
//! catch that, because that ordering needs all four columns in that order.
//!
//! # Why no partitioned parent index
//!
//! Postgres refuses to drop a partition of a partitioned index, and re-creates
//! the child unconditionally when a partition is added — via `CREATE TABLE …
//! PARTITION OF` or `ALTER TABLE … ATTACH PARTITION`, and whether the parent
//! is valid or was made invalid with `ON ONLY`. All four combinations were
//! measured on PostgreSQL 18.3. So an attached partition can never lack a
//! child of an existing parent index, and "deferred" is only reachable if the
//! index has no parent at all. `sql/0001_schema.sql` is what withholds those
//! parents — its DEFERRED INDEX SET block names the seven and creates none
//! of them; this module owns the per-partition indexes that stand in.
//!
//! Nothing is lost by it. Postgres has no global indexes — a partitioned index
//! is a metadata umbrella, and plans are per-partition regardless. A
//! parent-level query over a table whose newest partition lacks the index
//! produces an `Append` that index-scans every partition that has one and
//! falls back only on the one that does not.
//!
//! How graceful the fallback is depends on the table, and for `address_txs` it
//! stopped being graceful once the primary key went. Measured, not
//! assumed, on an 8.46M-row `p0`/`p1` pair with `p0` closed and `p1` open:
//!
//! With the index built — realtime, or after the partition closes — the plan is
//! an `Index Only Scan` over it, unsorted. Deferred, with a primary key still
//! present, it degrades to an `Index Scan Backward` on that key under an
//! `Incremental Sort`.
//!
//! Deferred with no primary key, which is the shape now, it degrades all the
//! way to a `Parallel Seq Scan` under a `Sort`.
//!
//! The parent-level query is a `Parallel Append`, so only the open partition
//! degrades: the closed one still contributes a `Parallel Index Only Scan`. But
//! it degrades further than it used to. On that open partition alone the scan
//! took 112,544 physical buffer reads and discarded 2,819,996 rows to return
//! 11, where the old pkey fallback was an index scan.
//!
//! That is a real cost and it is accepted, because of *when* it applies:
//! stepped mode only engages when the tip is `stepped_sync_gap` (25,000 blocks
//! by default) behind the node, i.e. during a sync whose API is serving a
//! partial chain anyway, and the degradation ends the moment the partition
//! closes. It is also bounded by one partition rather than by the table. If
//! serving address history during IBD ever matters more than the write cost it
//! buys, the lever is to leave `address_txs` out of the deferred set — not to
//! bring back a second btree that no query plans on.
//!
//! # What it is worth
//!
//! Measured by replaying the real mainnet heights 860,000-869,999 (10,000
//! blocks, 179,870 transactions) into a `p8` already holding 800,000-859,999,
//! one transaction per block, `synchronous_commit` off, against the same
//! template database both times:
//!
//! | | wall | blocks/s |
//! |---|---|---|
//! | realtime | 179.30 s | 55.8 |
//! | stepped  | 113.78 s | 87.9 |
//!
//! 1.58x, or 37% less wall time, for the seven indexes alone — the primary
//! keys, `idx_tx_inputs_txid` and `idx_spends_outpoint` were maintained in
//! both runs. The build that pays for it, on a partition holding 70,000
//! blocks with `maintenance_work_mem` at 2 GB:
//!
//! | index | build | size |
//! |---|---|---|
//! | `idx_address_txs_addr_height_desc` | 49.01 s | 983 MB |
//! | `idx_tx_outputs_txid_vout`         | 27.17 s | 340 MB |
//! | `idx_tx_outputs_outpoint`          |  6.26 s | 340 MB |
//! | `idx_tx_outputs_address`           |  5.61 s | 187 MB |
//! | `idx_transactions_display_txid`    |  5.14 s | 107 MB |
//! | `idx_transactions_txid`            |  1.89 s |  66 MB |
//! | `idx_transactions_list`            |  1.37 s |  75 MB |
//! | **total**                          | **96.44 s** | **2,098 MB** |
//!
//! Building `idx_address_txs_addr_height_desc` `UNIQUE` rather than plain is
//! expected to cost little, because the uniqueness check rides along on the
//! sort the build already does — adjacent equal keys are adjacent in it. This
//! has NOT been measured at seed scale: the run that produced a figure was
//! against a synthetic stand-in on an unrelated cluster, so no number is
//! quoted here. Measure it against a real partition before relying on the
//! duplicate proof being free.
//!
//! So the saving runs at ~6.6 s per 1,000 blocks against a one-off build of
//! ~96 s per partition — the trade only works because it is charged once per
//! partition and not once per interval. Over a full 100,000-block partition
//! that is roughly 11 minutes saved against something under 2.5 minutes spent.
//! Read the per-block figure as an upper bound rather than an average: the
//! window measured sits 60,000 blocks into the partition, where the indexes
//! being maintained are already near their final size.
//!
//! A second, unbudgeted-for effect: an index built in one pass is denser than
//! the same index grown by random insertion. Total relation size after the
//! same 10,000 blocks came out 3,444 MB against 3,790 MB for `address_txs_p8`
//! and 2,602 MB against 2,921 MB for `tx_outputs_p8` — 9-11% smaller.
//!
//! # Builds are plain, not `CONCURRENTLY`
//!
//! Measured on 3M real rows: 6.47 s plain against 32.76 s concurrent, a 5.1x
//! penalty for a non-blocking property that buys nothing here. The only writer
//! is the ingest that is paused for the build, and during an initial sync
//! there are no readers to protect. `CONCURRENTLY` also cannot run inside a
//! transaction block and leaves an `INVALID` index behind on failure, where a
//! plain build leaves nothing and simply runs again.
//!
//! # Hysteresis
//!
//! A single threshold makes a tip oscillating around the gap flap between
//! modes, and every flap drops and rebuilds those ~3 GB. So entering and
//! leaving use different marks and both require the condition to hold for
//! [`STEPPED_HOLD_ADVANCES`] consecutive advances.

use std::sync::Mutex;
use std::time::Instant;

use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use tokio::sync::watch;

use crate::error::{HyDbError, HyDbResult};
use crate::layout::partition_span;
use crate::tuning;

/// Gap below which stepped mode is left again, when the configured entry gap
/// leaves room for it. Deliberately far below the 25,000-block default entry
/// gap: the two marks are what stop a tip hovering at the threshold from
/// rebuilding the deferred set over and over.
pub const STEPPED_EXIT_GAP: i64 = 5_000;

/// Consecutive advances a mode condition must hold before it is acted on.
/// The third guard against flapping, and the one that covers a node tip that
/// jitters within a single advance's worth of blocks.
pub const STEPPED_HOLD_ADVANCES: u32 = 3;

/// Which index-maintenance regime the write path is running.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum HyDbSyncMode {
  /// Every index is maintained per block, as the schema declares them.
  #[default]
  Realtime,
  /// The open partition's query-only indexes are absent and will be built
  /// when it closes.
  Stepped,
}

impl HyDbSyncMode {
  /// The stable, machine-greppable spelling used in logs and on the wire.
  pub const fn as_str(self) -> &'static str {
    match self {
      Self::Realtime => "realtime",
      Self::Stepped => "stepped",
    }
  }
}

impl core::fmt::Display for HyDbSyncMode {
  fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
    f.write_str(self.as_str())
  }
}

/// One query-only index that stepped mode may defer, described the way it has
/// to be re-created: a name, the partitioned table it belongs to, and the
/// definition tail that follows `CREATE INDEX <name> ON <partition>`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DeferredIndex {
  /// Name the index carried while it was a partitioned parent, and the stem
  /// of every per-partition name derived from it.
  pub stem: &'static str,
  /// The partitioned table it indexes.
  pub table: &'static str,
  /// Everything after the table name in `CREATE INDEX … ON …`.
  pub definition: &'static str,
  /// Build it `UNIQUE`. Set only where the key is a real uniqueness
  /// constraint that has nowhere else to live: `address_txs` traded its
  /// redundant primary key for this index (see `sql/0001_schema.sql`),
  /// so the build at partition close is also the proof that the partition
  /// holds no duplicate. Deferring a unique index means not enforcing it
  /// until the build, which is the trade that section of the module doc
  /// describes.
  pub unique: bool,
}

/// The deferrable set: query-only indexes on height-partitioned tables, and
/// nothing else. Must stay in step with the DEFERRED INDEX SET block of
/// `sql/0001_schema.sql`, which is asserted by
/// `deferred_set_matches_the_migration`.
pub const DEFERRED_INDEXES: [DeferredIndex; 7] = [
  DeferredIndex {
    stem: "idx_transactions_txid",
    table: "transactions",
    definition: "(txid)",
    unique: false,
  },
  DeferredIndex {
    stem: "idx_transactions_list",
    table: "transactions",
    definition: "(block_height DESC, block_pos DESC, txid DESC)",
    unique: false,
  },
  DeferredIndex {
    stem: "idx_transactions_display_txid",
    table: "transactions",
    definition: "(display_hex(txid) text_pattern_ops)",
    unique: false,
  },
  DeferredIndex {
    stem: "idx_tx_outputs_txid_vout",
    table: "tx_outputs",
    definition: "(txid, vout_index)",
    unique: false,
  },
  DeferredIndex {
    stem: "idx_tx_outputs_address",
    table: "tx_outputs",
    definition: "(address) WHERE address IS NOT NULL",
    unique: false,
  },
  DeferredIndex {
    stem: "idx_tx_outputs_outpoint",
    table: "tx_outputs",
    definition: "(outpoint)",
    unique: false,
  },
  DeferredIndex {
    stem: "idx_address_txs_addr_height_desc",
    table: "address_txs",
    definition: "(address, block_height DESC, block_pos DESC, txid DESC)",
    // The one unique member: this index *is* `address_txs`'s uniqueness
    // constraint now that the redundant prefix primary key is gone.
    unique: true,
  },
];

/// The indexes the *ingest* path reads and that therefore may never be
/// deferred, whatever else changes. Asserted against [`DEFERRED_INDEXES`] by
/// `no_ingest_path_index_is_deferrable`, so adding a member to the deferred
/// set that ingest depends on is a failing test rather than a stalled sync.
///
/// Every one of these was traced to a statement on the connect path:
/// prevout resolution reads `utxo_set` by primary key (`ingest/derive.rs`),
/// the balance stage reads and upserts `address_stats` by primary key
/// (`ingest/balances.rs`), the validator stage joins on
/// `idx_validator_collateral` (`ingest/validators.rs`), and the remaining
/// primary keys back the `ON CONFLICT` arbiters and prevout joins their own
/// tables' statements name — `transactions` by `(block_height, txid)`,
/// `tx_inputs` and `tx_outputs` by that pair plus an index.
///
/// `address_txs_pkey` used to be on this list and no longer exists. It was
/// here as the arbiter of `INSERT INTO address_txs … ON CONFLICT DO NOTHING`;
/// A5 narrowed the writer's in-memory dedup key to exactly the constraint's
/// columns, which made the key set unique by construction and let the clause
/// go, and the schema then stopped declaring the constraint
/// itself, as a redundant key prefix of
/// `idx_address_txs_addr_height_desc`. `address_txs` is consequently the one
/// base table with *no* entry here: the ingest path reads none of its indexes
/// on connect, and the reversal delete reads `idx_address_txs_block_height`,
/// which keeps its partitioned parent and is not in the deferred set.
/// Nothing named here is an `address_txs` index, so this list no longer needs
/// one — but the property the list encodes is unchanged.
pub const INGEST_PATH_INDEXES: [&str; 8] = [
  "address_stats_pkey",
  "utxo_set_pkey",
  "idx_utxo_set_address_key",
  "idx_utxo_set_block_height",
  "transactions_pkey",
  "tx_inputs_pkey",
  "tx_outputs_pkey",
  "idx_validator_collateral",
];

/// Per-partition name for a deferred index: the stem plus the partition's own
/// `_pN` suffix. The same rule `sql/0001_schema.sql` documents, so a
/// database that has been migrated and one that was built by the daemon carry
/// identical names.
#[must_use]
pub fn child_index_name(stem: &str, partition: i32) -> String {
  format!("{stem}_p{partition}")
}

/// Partition table name for a partitioned table at a partition index.
#[must_use]
fn partition_name(table: &str, partition: i32) -> String {
  format!("{table}_p{partition}")
}

/// The partition index a height falls in.
#[must_use]
pub fn partition_of(height: i32) -> i32 {
  height.max(0) / partition_span()
}

/// A mode change the state machine decided on.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SteppedTransition {
  pub to: HyDbSyncMode,
  /// Projection tip at the moment of the decision.
  pub tip: Option<i32>,
  /// Node tip the decision was measured against.
  pub node_tip: i32,
  /// `node_tip - tip`.
  pub gap: i64,
}

#[derive(Debug)]
struct MachineState {
  mode: HyDbSyncMode,
  toward_stepped: u32,
  toward_realtime: u32,
  /// `(partition, mode)` the on-disk index set was last reconciled for. Lets
  /// the reconcile be skipped entirely — no catalog round trip — while
  /// neither has changed, which is every advance but a boundary crossing or a
  /// transition.
  settled: Option<(i32, HyDbSyncMode)>,
}

/// The mode decision, with no database attached.
///
/// Split out from [`SteppedSync`] because deciding and building are different
/// jobs with different failure modes: this half is pure, synchronous and
/// exhaustively testable without a server, and the half that issues DDL has
/// nothing to decide.
#[derive(Debug)]
struct ModeMachine {
  enabled: bool,
  enter_gap: i64,
  exit_gap: i64,
  hold: u32,
  state: Mutex<MachineState>,
  mode_tx: watch::Sender<HyDbSyncMode>,
}

impl ModeMachine {
  /// The exit mark is derived rather than configured: it is
  /// [`STEPPED_EXIT_GAP`], or a fifth of the entry gap when that is smaller,
  /// so an operator who lowers `stepped_sync_gap` below 5,000 still gets two
  /// distinct marks instead of one threshold wearing two names.
  fn new(enabled: bool, enter_gap: i64) -> Self {
    let enter_gap = enter_gap.max(1);
    let exit_gap = STEPPED_EXIT_GAP.min(enter_gap / 5).max(1);
    let (mode_tx, _) = watch::channel(HyDbSyncMode::Realtime);
    Self {
      enabled,
      enter_gap,
      exit_gap,
      hold: STEPPED_HOLD_ADVANCES,
      state: Mutex::new(MachineState {
        mode: HyDbSyncMode::Realtime,
        toward_stepped: 0,
        toward_realtime: 0,
        settled: None,
      }),
      mode_tx,
    }
  }

  /// The mode in force.
  #[must_use]
  pub fn mode(&self) -> HyDbSyncMode {
    self.lock().mode
  }

  /// Watch the mode, for the status endpoint and the TUI.
  #[must_use]
  pub fn subscribe(&self) -> watch::Receiver<HyDbSyncMode> {
    self.mode_tx.subscribe()
  }

  /// The gap at which stepped mode engages.
  #[must_use]
  pub const fn enter_gap(&self) -> i64 {
    self.enter_gap
  }

  /// The gap below which it disengages.
  #[must_use]
  pub const fn exit_gap(&self) -> i64 {
    self.exit_gap
  }

  fn lock(&self) -> std::sync::MutexGuard<'_, MachineState> {
    self.state.lock().unwrap_or_else(|e| e.into_inner())
  }

  /// Feed one advance's observation into the state machine.
  ///
  /// Pure apart from the log line: it decides, it does not build. Returns the
  /// transition to act on, or `None` when the mode stands — including when the
  /// condition is met but has not held long enough, which is logged at DEBUG
  /// so a mode that refuses to move is diagnosable rather than mysterious.
  pub fn observe(&self, tip: Option<i32>, node_tip: i32) -> Option<SteppedTransition> {
    let gap = i64::from(node_tip) - tip.map_or(-1, i64::from);
    let mut st = self.lock();

    if !self.enabled {
      // The flag is off: never enter, and if a previous configuration left us
      // stepped, leave at the first observation.
      if st.mode == HyDbSyncMode::Stepped {
        st.mode = HyDbSyncMode::Realtime;
        st.toward_stepped = 0;
        st.toward_realtime = 0;
        let _ = self.mode_tx.send(HyDbSyncMode::Realtime);
        return Some(SteppedTransition {
          to: HyDbSyncMode::Realtime,
          tip,
          node_tip,
          gap,
        });
      }
      return None;
    }

    let want = match st.mode {
      HyDbSyncMode::Realtime if gap >= self.enter_gap => Some(HyDbSyncMode::Stepped),
      HyDbSyncMode::Stepped if gap < self.exit_gap => Some(HyDbSyncMode::Realtime),
      _ => None,
    };

    let Some(target) = want else {
      st.toward_stepped = 0;
      st.toward_realtime = 0;
      // Only worth a line when the gap is actually *between* the two marks —
      // that is the case where the mode looks wrong and is not. A gap of
      // nothing in realtime mode, which is every advance at the tip, is not a
      // refusal and does not need reporting.
      if (self.exit_gap..self.enter_gap).contains(&gap) {
        tracing::debug!(
          mode = st.mode.as_str(),
          gap,
          enter_gap = self.enter_gap,
          exit_gap = self.exit_gap,
          "stepped sync refuses to flap: mode={} holds because the gap sits in \
           the hysteresis band between {} and {}",
          st.mode.as_str(),
          self.exit_gap,
          self.enter_gap
        );
      }
      return None;
    };

    let held = match target {
      HyDbSyncMode::Stepped => {
        st.toward_realtime = 0;
        st.toward_stepped = st.toward_stepped.saturating_add(1);
        st.toward_stepped
      }
      HyDbSyncMode::Realtime => {
        st.toward_stepped = 0;
        st.toward_realtime = st.toward_realtime.saturating_add(1);
        st.toward_realtime
      }
    };
    if held < self.hold {
      tracing::debug!(
        mode = st.mode.as_str(),
        wanted = target.as_str(),
        gap,
        held,
        hold = self.hold,
        "stepped sync refuses to flap: mode={} holds until the condition has \
         held for {} consecutive advances",
        st.mode.as_str(),
        self.hold
      );
      return None;
    }

    st.mode = target;
    st.toward_stepped = 0;
    st.toward_realtime = 0;
    let _ = self.mode_tx.send(target);
    Some(SteppedTransition {
      to: target,
      tip,
      node_tip,
      gap,
    })
  }

  /// Leave stepped mode immediately, whatever the gap says, and reset the
  /// hysteresis so re-entry has to earn its hold again.
  ///
  /// The reorg escape hatch. `idx_tx_outputs_outpoint` is the join the UTXO
  /// restore runs on every disconnected block, and it is in the deferred set
  /// only because a reorg cannot reach the open partition while the tip is
  /// `stepped_sync_gap` blocks behind. If one is planned regardless, this is
  /// what puts the index back before the disconnect rather than letting it
  /// scan every output the partition holds.
  pub fn force_realtime(&self, reason: &str) -> Option<HyDbSyncMode> {
    let mut st = self.lock();
    st.toward_stepped = 0;
    st.toward_realtime = 0;
    if st.mode == HyDbSyncMode::Realtime {
      return None;
    }
    st.mode = HyDbSyncMode::Realtime;
    let _ = self.mode_tx.send(HyDbSyncMode::Realtime);
    tracing::info!(
      mode = "realtime",
      reason,
      "sync mode=realtime: leaving stepped sync early"
    );
    Some(HyDbSyncMode::Realtime)
  }
}

/// The stepped-sync state machine and the index maintenance it drives.
///
/// One per ingest handle, which is one per database: the ingest advisory lock
/// already guarantees a single writer, so this needs no cross-process
/// coordination. DDL runs on the admin pool, which is where
/// `[db.client.admin] maint_work_mem` is set — the parameter that decides how
/// fast a deferred build runs.
#[derive(Debug)]
pub struct SteppedSync {
  admin: PgPool,
  machine: ModeMachine,
}

impl SteppedSync {
  /// Build from the process-wide `[db.ingest]` tunables.
  #[must_use]
  pub fn from_config(admin: PgPool) -> Self {
    Self::with_params(admin, tuning::stepped_sync(), tuning::stepped_sync_gap())
  }

  /// Build with explicit parameters.
  #[must_use]
  pub fn with_params(admin: PgPool, enabled: bool, enter_gap: i64) -> Self {
    Self {
      admin,
      machine: ModeMachine::new(enabled, enter_gap),
    }
  }

  /// The admin pool DDL runs on, for callers that must re-apply storage
  /// tuning alongside a mode change.
  #[must_use]
  pub const fn admin(&self) -> &PgPool {
    &self.admin
  }

  /// The mode in force.
  #[must_use]
  pub fn mode(&self) -> HyDbSyncMode {
    self.machine.mode()
  }

  /// Watch the mode, for the status endpoint and the TUI.
  #[must_use]
  pub fn subscribe(&self) -> watch::Receiver<HyDbSyncMode> {
    self.machine.subscribe()
  }

  /// The gap at which stepped mode engages.
  #[must_use]
  pub const fn enter_gap(&self) -> i64 {
    self.machine.enter_gap()
  }

  /// The gap below which it disengages.
  #[must_use]
  pub const fn exit_gap(&self) -> i64 {
    self.machine.exit_gap()
  }

  /// Feed one advance's observation into the state machine. See
  /// [`ModeMachine::observe`].
  pub fn observe(&self, tip: Option<i32>, node_tip: i32) -> Option<SteppedTransition> {
    self.machine.observe(tip, node_tip)
  }

  /// Leave stepped mode immediately. See [`ModeMachine::force_realtime`].
  pub fn force_realtime(&self, reason: &str) -> Option<HyDbSyncMode> {
    self.machine.force_realtime(reason)
  }

  /// Act on a transition: log it, then reconcile the index set.
  ///
  /// # Errors
  ///
  /// [`HyDbError::Pg`] if a build or drop fails.
  pub async fn apply(&self, transition: SteppedTransition) -> HyDbResult<()> {
    let open = partition_of(transition.tip.unwrap_or(0));
    let affected: Vec<&str> = DEFERRED_INDEXES.iter().map(|d| d.stem).collect();
    tracing::info!(
      mode = transition.to.as_str(),
      tip = transition.tip,
      node_tip = transition.node_tip,
      gap = transition.gap,
      open_partition = open,
      indexes = affected.join(",").as_str(),
      "sync mode={}: {} the open partition's query-only indexes (gap {} blocks \
       to the node tip)",
      transition.to.as_str(),
      match transition.to {
        HyDbSyncMode::Stepped => "deferring",
        HyDbSyncMode::Realtime => "restoring",
      },
      transition.gap
    );
    self.reconcile(open, transition.to).await
  }

  /// Bring the on-disk index set in line with `mode` for the partition that
  /// `height` falls in, building anything a *closed* partition is missing.
  ///
  /// This is where "build at partition close" happens: a partition stops being
  /// the open one the moment the tip moves past its last height, and every
  /// partition below the open one is expected to carry the full set. It is
  /// also self-healing — a daemon killed mid-build finds the missing index on
  /// its next boundary and builds it then.
  ///
  /// Skipped entirely, with no catalog round trip, while the open partition
  /// and the mode are both unchanged since the last call.
  ///
  /// # Errors
  ///
  /// [`HyDbError::Pg`] if a build or drop fails.
  pub async fn ensure_settled(&self, height: i32) -> HyDbResult<()> {
    let open = partition_of(height);
    let mode = {
      let st = self.machine.lock();
      if st.settled == Some((open, st.mode)) {
        return Ok(());
      }
      st.mode
    };
    self.reconcile(open, mode).await
  }

  async fn reconcile(&self, open: i32, mode: HyDbSyncMode) -> HyDbResult<()> {
    // One round trip for the partitions that exist, one for the indexes that
    // exist. Everything after is decided in memory.
    let mut wanted_tables: Vec<String> = Vec::new();
    for partition in 0..=open {
      for spec in &DEFERRED_INDEXES {
        wanted_tables.push(partition_name(spec.table, partition));
      }
    }
    wanted_tables.sort_unstable();
    wanted_tables.dedup();
    let live_tables = existing(&self.admin, &wanted_tables).await?;

    let mut wanted_indexes: Vec<String> = Vec::new();
    for partition in 0..=open {
      for spec in &DEFERRED_INDEXES {
        if live_tables.contains(&partition_name(spec.table, partition)) {
          wanted_indexes.push(child_index_name(spec.stem, partition));
        }
      }
    }
    let live_indexes = existing_indexes(&self.admin, &wanted_indexes).await?;

    for partition in 0..=open {
      // Closed partitions carry the full set; the open one carries it only in
      // realtime mode.
      let want = partition < open || mode == HyDbSyncMode::Realtime;
      for spec in &DEFERRED_INDEXES {
        let table = partition_name(spec.table, partition);
        if !live_tables.contains(&table) {
          continue;
        }
        let name = child_index_name(spec.stem, partition);
        match (want, live_indexes.get(&name)) {
          (true, None) => self.build(&name, &table, spec, partition, open).await?,
          // Present under the right name but with the wrong uniqueness. For
          // the unique member this is the whole constraint on `address_txs`
          // missing, so it is rebuilt rather than accepted: a name match is
          // not evidence that the index enforces anything.
          (true, Some(&is_unique)) if is_unique != spec.unique => {
            tracing::warn!(
              index = name,
              partition,
              found_unique = is_unique,
              want_unique = spec.unique,
              "stepped sync deferred index {name}: uniqueness does not match \
               the spec; dropping and rebuilding"
            );
            self.drop_index(&name, partition).await?;
            self.build(&name, &table, spec, partition, open).await?;
          }
          (false, Some(_)) => self.drop_index(&name, partition).await?,
          _ => {}
        }
      }
    }

    let mut st = self.machine.lock();
    st.settled = Some((open, mode));
    Ok(())
  }

  async fn build(
    &self,
    name: &str,
    table: &str,
    spec: &DeferredIndex,
    partition: i32,
    open: i32,
  ) -> HyDbResult<()> {
    // Plain, not CONCURRENTLY: 6.47 s against 32.76 s on 3M rows, and the
    // writer this would avoid blocking is the one waiting for the build.
    //
    // `UNIQUE` where the spec says so, which for `address_txs` makes this
    // build the partition's duplicate check as well as its index: the key it
    // enforces is the constraint the dropped primary key used to hold, so a
    // failure here is a real duplicate and must stop the advance rather than
    // be retried into a silently non-unique index.
    //
    // No `IF NOT EXISTS`. `reconcile` has already established that the name is
    // free, so a collision here is a surprise worth hearing about, and the
    // clause is what let a same-named non-unique index satisfy this statement
    // silently — which for the unique member is the constraint quietly absent.
    let unique = if spec.unique { "UNIQUE " } else { "" };
    let sql = format!("CREATE {unique}INDEX {name} ON {table} {}", spec.definition);
    let started = Instant::now();
    sqlx::query(&sql).execute(&self.admin).await.map_err(|e| {
      // Name the partition in the context rather than leaving it to Postgres'
      // own message, and carry the DETAIL through: on a unique build the
      // DETAIL is the duplicated key itself, which is the whole diagnosis, and
      // `sqlx`'s `Display` drops it.
      let detail = e
        .as_database_error()
        .and_then(|db| db.try_downcast_ref::<sqlx::postgres::PgDatabaseError>())
        .and_then(sqlx::postgres::PgDatabaseError::detail)
        .map(|d| format!(" ({d})"))
        .unwrap_or_default();
      let ctx = if spec.unique {
        format!(
          "stepped sync: build UNIQUE {name} on partition p{partition} of \
           {table} — a failure here is a duplicate key in that partition, not \
           a build problem{detail}"
        )
      } else {
        format!("stepped sync: build deferred index {name} on p{partition}{detail}")
      };
      tracing::error!(
        index = name,
        table,
        partition,
        unique = spec.unique,
        "stepped sync could not build {name} on {table} (partition p{partition}): {e}{detail}"
      );
      HyDbError::pg(&ctx, e)
    })?;
    let elapsed = started.elapsed();
    tracing::info!(
      index = name,
      table,
      partition,
      unique = spec.unique,
      closed = partition < open,
      elapsed_ms = elapsed.as_millis() as u64,
      "stepped sync built deferred index {name} on {table} in {:.2}s",
      elapsed.as_secs_f64()
    );
    Ok(())
  }

  async fn drop_index(&self, name: &str, partition: i32) -> HyDbResult<()> {
    let sql = format!("DROP INDEX IF EXISTS {name}");
    sqlx::query(&sql)
      .execute(&self.admin)
      .await
      .map_err(|e| HyDbError::pg("stepped sync: drop deferred index", e))?;
    tracing::info!(
      index = name,
      partition,
      "stepped sync deferred index {name}: dropped for the open partition"
    );
    Ok(())
  }
}

/// Which of `names` exist as relations, in one round trip.
async fn existing(pool: &PgPool, names: &[String]) -> HyDbResult<Vec<String>> {
  if names.is_empty() {
    return Ok(Vec::new());
  }
  sqlx::query_scalar!(
    r#"SELECT n AS "n!" FROM UNNEST($1::text[]) AS n WHERE to_regclass(n) IS NOT NULL"#,
    names
  )
  .fetch_all(pool)
  .await
  .map_err(|e| HyDbError::pg("stepped sync: probe index catalog", e))
}

/// Which of `names` exist as indexes, and whether each enforces uniqueness.
///
/// Existence alone was what `reconcile` used to decide on, and `to_regclass`
/// matches any relation of that name. An index left plain where the spec calls
/// for `UNIQUE` therefore read as satisfied, which for `address_txs` is its
/// only constraint silently absent.
///
/// Written against the runtime API rather than `query!` so it needs no entry in
/// the offline query cache.
async fn existing_indexes(
  pool: &PgPool,
  names: &[String],
) -> HyDbResult<std::collections::HashMap<String, bool>> {
  if names.is_empty() {
    return Ok(std::collections::HashMap::new());
  }
  let rows: Vec<(String, bool)> = sqlx::query_as(
    "SELECT c.relname::text, i.indisunique \
     FROM UNNEST($1::text[]) AS n \
     JOIN pg_class c ON c.oid = to_regclass(n) \
     JOIN pg_index i ON i.indexrelid = c.oid",
  )
  .bind(names)
  .fetch_all(pool)
  .await
  .map_err(|e| HyDbError::pg("stepped sync: probe index uniqueness", e))?;
  Ok(rows.into_iter().collect())
}

#[cfg(test)]
mod tests {
  use super::*;

  /// The one property that makes the whole mode safe rather than merely fast:
  /// nothing the write path reads is ever dropped. A deferred index that
  /// ingest probes does not slow the sync down, it stops it — the probe
  /// becomes a sequential scan of a partition that is still growing.
  #[test]
  fn no_ingest_path_index_is_deferrable() {
    for spec in &DEFERRED_INDEXES {
      assert!(
        !INGEST_PATH_INDEXES.contains(&spec.stem),
        "{} is read by the ingest path and must never be deferred",
        spec.stem
      );
    }
  }

  /// Deferral is only possible on a partitioned table — on an unpartitioned
  /// one the rebuild is full-size, so it can never be cheaper than the
  /// maintenance it avoids.
  #[test]
  fn every_deferred_index_is_on_a_partitioned_table() {
    const PARTITIONED: [&str; 5] = [
      "transactions",
      "tx_inputs",
      "tx_outputs",
      "address_txs",
      "spends",
    ];
    for spec in &DEFERRED_INDEXES {
      assert!(
        PARTITIONED.contains(&spec.table),
        "{} is on {}, which is not height-partitioned",
        spec.stem,
        spec.table
      );
    }
  }

  /// The committed schema, split into the DDL it executes and the DEFERRED
  /// INDEX SET block that documents what it deliberately does not create.
  ///
  /// The schema is one file and states its end state, so a deferred index
  /// appears in it only as commentary. Both halves are needed: the DDL half
  /// proves the index has no partitioned parent, the comment half is the
  /// list this module mirrors.
  fn schema_ddl_and_deferred_block() -> (String, String) {
    let sql = std::fs::read_to_string(
      std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("sql/0001_schema.sql"),
    )
    .unwrap_or_default();
    assert!(!sql.is_empty(), "0001_schema.sql should be readable");
    let block = sql
      .split("DEFERRED INDEX SET")
      .nth(1)
      .unwrap_or("")
      .split("END DEFERRED INDEX SET")
      .next()
      .unwrap_or("")
      .to_owned();
    assert!(
      !block.is_empty(),
      "the schema should carry a DEFERRED INDEX SET block"
    );
    // Everything the file actually executes: comments stripped, so a stem
    // named only in prose does not read as a CREATE INDEX.
    let ddl: String = sql
      .lines()
      .map(|line| line.split("--").next().unwrap_or(""))
      .collect::<Vec<_>>()
      .join("\n");
    (ddl, block)
  }

  /// The schema and this table are two spellings of one list. If they
  /// disagree, a freshly migrated database and a daemon-built one carry
  /// different indexes under the same names.
  ///
  /// The pin is to `sql/0001_schema.sql`, which is where the
  /// seven used to be taken off their parents before the migrations were
  /// collapsed into one file. The intent is unchanged and the grip is
  /// tighter: as well as checking that the schema names every member, it now
  /// checks that the schema never *creates* one, which is the property the
  /// existed to establish and the one a careless edit would undo.
  #[test]
  fn deferred_set_matches_the_migration() {
    let (ddl, block) = schema_ddl_and_deferred_block();
    for spec in &DEFERRED_INDEXES {
      assert!(
        block.contains(&format!("'{}'", spec.stem)),
        "{} is deferrable here but the schema's deferred set does not list it",
        spec.stem
      );
      assert!(
        block.contains(&format!("'{}'", spec.definition)),
        "{}'s definition here does not appear in the schema's deferred set",
        spec.stem
      );
      // The absence is the contract: a partitioned parent for any of these
      // makes the per-partition index undroppable and silently disables
      // stepped sync.
      assert!(
        !ddl.contains(spec.stem),
        "{} is deferrable, so the schema must not create it — a partitioned          parent cannot coexist with deferring it on the open partition",
        spec.stem
      );
    }
    // And nothing the schema defers is missing from this table: count the
    // listed entries by their table column.
    let rows = block.matches(" on '").count();
    assert_eq!(
      rows,
      DEFERRED_INDEXES.len(),
      "the schema lists {rows} deferred indexes and this table has {}",
      DEFERRED_INDEXES.len()
    );
  }

  /// The unique members are a second contract with the schema, and a sharper
  /// one: a `unique` flag that drifts away from it means a database where
  /// `address_txs` has neither a primary key nor a unique index — no
  /// constraint at all on the widest table in the schema, and nothing to
  /// notice it. So the flag is pinned to the schema, in both directions.
  ///
  /// The pin is to `sql/0001_schema.sql`, the schema that
  /// dropped `address_txs_pkey` and rebuilt each partition's index UNIQUE.
  /// The collapsed schema states that end directly: no primary key on
  /// `address_txs`, and the index marked UNIQUE in the deferred set. Both
  /// halves are asserted, because either one alone is satisfiable by an
  /// `address_txs` with no constraint whatsoever.
  #[test]
  fn the_unique_members_match_the_migration_that_made_them_unique() {
    let (ddl, block) = schema_ddl_and_deferred_block();
    assert!(
      block.contains("[UNIQUE]"),
      "the schema's deferred set should mark the surviving address_txs index UNIQUE"
    );
    // The redundant prefix primary key must not come back: it is the 8.8 GB
    // this table gave up, and re-declaring it would also re-parent nothing
    // but re-create the redundancy.
    let address_txs = ddl
      .split("CREATE TABLE address_txs (")
      .nth(1)
      .unwrap_or_default()
      .split(");")
      .next()
      .unwrap_or_default()
      .to_owned();
    assert!(
      !address_txs.is_empty(),
      "the schema should define the address_txs table"
    );
    assert!(
      !address_txs.contains("PRIMARY KEY"),
      "address_txs must carry no primary key: its uniqueness lives in the        deferred UNIQUE index, which a declared key would make redundant again"
    );
    assert!(
      !ddl.contains("address_txs_pkey"),
      "address_txs_pkey was dropped and must not be re-created"
    );

    let unique: Vec<&str> = DEFERRED_INDEXES
      .iter()
      .filter(|spec| spec.unique)
      .map(|spec| spec.stem)
      .collect();
    assert_eq!(
      unique,
      vec!["idx_address_txs_addr_height_desc"],
      "the unique deferred set is exactly the index that replaced        address_txs_pkey; adding another means another dropped constraint,        which is a schema decision and not a flag flip"
    );
  }

  /// The dropped constraint must be gone from the ingest-path list too, or the
  /// integration test that checks every one of them still exists starts
  /// looking for an index no database has.
  #[test]
  fn the_dropped_address_txs_primary_key_is_not_claimed_by_the_ingest_path() {
    assert!(
      !INGEST_PATH_INDEXES.contains(&"address_txs_pkey"),
      "address_txs has no primary key and so cannot be an ingest-path index"
    );
    assert!(
      !INGEST_PATH_INDEXES
        .iter()
        .any(|name| name.starts_with("address_txs")),
      "no address_txs index is read by the connect path: the insert has no        arbiter and the reversal delete uses idx_address_txs_block_height"
    );
  }

  /// The decision half, with no database attached — which is the whole point
  /// of it being a separate type.
  fn machine_or_fail(enter_gap: i64) -> ModeMachine {
    ModeMachine::new(true, enter_gap)
  }

  #[tokio::test]
  async fn enters_stepped_only_after_the_gap_holds() {
    let sync = machine_or_fail(25_000);
    // Two advances past the entry gap are not enough — the third is.
    for advance in 1..STEPPED_HOLD_ADVANCES {
      assert_eq!(
        sync.observe(Some(900_000), 999_999),
        None,
        "advance {advance} should not have moved the mode yet"
      );
      assert_eq!(sync.mode(), HyDbSyncMode::Realtime);
    }
    let transition = sync.observe(Some(900_000), 999_999);
    assert_eq!(
      transition.map(|t| (t.to, t.gap)),
      Some((HyDbSyncMode::Stepped, 99_999)),
      "the condition has now held for the full hold"
    );
    assert_eq!(sync.mode(), HyDbSyncMode::Stepped);
  }

  #[tokio::test]
  async fn leaves_stepped_only_after_the_exit_gap_holds() {
    let sync = machine_or_fail(25_000);
    for _ in 0..STEPPED_HOLD_ADVANCES {
      sync.observe(Some(900_000), 999_999);
    }
    assert_eq!(sync.mode(), HyDbSyncMode::Stepped);

    // Below the *entry* gap but above the exit gap: still stepped, because
    // the band between the two marks is what stops the flapping.
    for _ in 0..(STEPPED_HOLD_ADVANCES * 2) {
      assert_eq!(sync.observe(Some(990_000), 1_000_000), None);
      assert_eq!(sync.mode(), HyDbSyncMode::Stepped);
    }

    for _ in 1..STEPPED_HOLD_ADVANCES {
      assert_eq!(sync.observe(Some(999_000), 1_000_000), None);
    }
    assert_eq!(
      sync.observe(Some(999_000), 1_000_000).map(|t| t.to),
      Some(HyDbSyncMode::Realtime),
      "under the exit gap for the full hold"
    );
    assert_eq!(sync.mode(), HyDbSyncMode::Realtime);
  }

  /// The property the whole hysteresis exists for: a tip that crosses the
  /// mark and comes back must not transition, because every transition drops
  /// and rebuilds ~3 GB of index.
  #[tokio::test]
  async fn a_tip_oscillating_across_the_mark_never_flaps() {
    let sync = machine_or_fail(25_000);
    for _ in 0..40 {
      // Alternate either side of the 25,000-block entry gap.
      sync.observe(Some(975_001), 1_000_000); // gap 24,999 — under
      sync.observe(Some(974_999), 1_000_000); // gap 25,001 — over
      assert_eq!(
        sync.mode(),
        HyDbSyncMode::Realtime,
        "an oscillating tip must not enter stepped mode"
      );
    }
    // ...and a gap that stays over the mark still gets in.
    for _ in 0..STEPPED_HOLD_ADVANCES {
      sync.observe(Some(974_999), 1_000_000);
    }
    assert_eq!(sync.mode(), HyDbSyncMode::Stepped);
  }

  #[tokio::test]
  async fn a_reorg_leaves_stepped_mode_and_resets_the_hold() {
    let sync = machine_or_fail(25_000);
    for _ in 0..STEPPED_HOLD_ADVANCES {
      sync.observe(Some(900_000), 999_999);
    }
    assert_eq!(sync.mode(), HyDbSyncMode::Stepped);

    assert_eq!(
      sync.force_realtime("a reorg was planned"),
      Some(HyDbSyncMode::Realtime)
    );
    assert_eq!(sync.mode(), HyDbSyncMode::Realtime);
    // Already realtime: nothing to report the second time.
    assert_eq!(sync.force_realtime("again"), None);

    // Re-entry has to earn the full hold again rather than resuming a count
    // the reorg interrupted.
    for _ in 1..STEPPED_HOLD_ADVANCES {
      assert_eq!(sync.observe(Some(900_000), 999_999), None);
    }
    assert!(sync.observe(Some(900_000), 999_999).is_some());
  }

  #[tokio::test]
  async fn the_flag_off_means_the_mode_never_engages() {
    let sync = ModeMachine::new(false, 25_000);
    for _ in 0..50 {
      assert_eq!(sync.observe(Some(0), 1_000_000), None);
    }
    assert_eq!(sync.mode(), HyDbSyncMode::Realtime);
  }

  /// The two marks must stay distinct even when an operator configures an
  /// entry gap below the built-in exit mark, or the hysteresis collapses back
  /// into the single threshold it exists to avoid.
  #[tokio::test]
  async fn the_exit_mark_stays_below_the_entry_mark() {
    for enter in [1_i64, 10, 5_000, 25_000, 99_999] {
      let sync = machine_or_fail(enter);
      assert!(
        sync.exit_gap() <= sync.enter_gap(),
        "entry {enter} produced exit {} above it",
        sync.exit_gap()
      );
      assert!(sync.exit_gap() >= 1);
    }
    assert_eq!(machine_or_fail(25_000).exit_gap(), STEPPED_EXIT_GAP);
  }

  #[test]
  fn child_names_follow_the_partition_suffix_rule() {
    assert_eq!(
      child_index_name("idx_tx_outputs_outpoint", 9),
      "idx_tx_outputs_outpoint_p9"
    );
    assert_eq!(partition_name("address_txs", 7), "address_txs_p7");
  }
}
