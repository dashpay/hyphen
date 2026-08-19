//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Consistency checks over *chain data*, as opposed to the schema and layout
//! checks in [`crate::migrate`] and [`crate::layout`].
//!
//! One implementation, two callers: [`HyphenDb::connect`](crate::HyphenDb::connect)
//! runs [`undo_coverage`] as a startup gate before handing out a facade, and
//! the same function backs the operator-facing read-only diagnostic
//! (`HyDbStatus::undo_coverage`). The gate refuses only on the defects that
//! make a rollback impossible; everything else is reported and survivable.
//!
//! # Why counting is the whole point
//!
//! `MIN`/`MAX` over `undo_blocks` bounds a range; it cannot see a hole inside
//! that range. A crash mid-reindex, a hand-dropped partition, or a prune that
//! raced a config change all leave the *bounds* looking healthy while a
//! disconnect walking down from the tip runs into a height with no pre-image —
//! discovered, today, only mid-rollback with the balance stage already half
//! reversed. So coverage is established by comparing a row count against the
//! count the height span implies, and the gaps are then named.
//!
//! # Pruned is not missing
//!
//! Undo is bounded on purpose: [`crate::ingest`] prunes it on every advance by
//! dropping whole height partitions at or below `tip - undo_keep_depth`. The
//! two states are distinguished by *direction*, which is what makes this
//! decidable at all:
//!
//! - **Pruning only ever removes a contiguous prefix from the bottom.** It
//!   drops a partition whose highest possible height is at or below the
//!   cutoff, so it can neither punch an interior hole nor touch the top.
//! - **Corruption shows up above the floor**: a hole between the oldest
//!   retained height and the tip, or coverage that stops short of the tip.
//!
//! The check therefore anchors on the *retained* floor (`MIN(height)`) rather
//! than on the configured keep depth, and asks whether `[floor, tip]` is
//! complete. Anything below the floor is, by construction, either pruned or
//! never written, and is deliberately invisible here — refusing startup over
//! correctly pruned undo would be a worse bug than the gap it was hunting.
//!
//! How much history the floor leaves reachable is a separate question from
//! whether the retained window has holes, and is reported as a warning
//! ([`HyDbUndoDefect::ShallowRetention`]) rather than a refusal: the retained
//! depth legitimately dips one block below `max_reorg_depth` when the config
//! sets it equal to `undo_keep_depth` and a partition boundary lands exactly
//! on the cutoff.

use sqlx::PgPool;

use crate::error::{HyDbError, HyDbResult};
use crate::layout::undo_span;
use crate::tuning::{max_reorg_depth, undo_keep_depth};
use crate::types::HyDbHeight;

/// Gap runs named in a report. A hole big enough to exceed this is already
/// established beyond doubt; the count is exact regardless.
const MAX_GAP_RUNS: i64 = 32;

/// Undo partitions named per report. A gap spanning more than this many
/// partitions needs no further enumeration to be actionable.
const MAX_NAMED_PARTITIONS: usize = 64;

/// A contiguous run of heights inside the retained undo window that has no
/// pre-image row. Inclusive on both ends.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HyDbUndoGap {
  pub lo: HyDbHeight,
  pub hi: HyDbHeight,
}

impl HyDbUndoGap {
  /// How many heights this run covers.
  #[must_use]
  pub fn heights(&self) -> u32 {
    self.hi.0.saturating_sub(self.lo.0).saturating_add(1)
  }
}

/// Something the undo ledger should not be doing.
///
/// [`Self::refuses_startup`] is the policy split between the two callers: the
/// startup gate acts on the defects that make a rollback impossible, the
/// operator diagnostic shows all of them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum HyDbUndoDefect {
  /// A canonical chain exists and there is no undo ledger at all. Not a prune
  /// outcome: pruning cannot reach the tip's own partition.
  NoCoverage { tip: HyDbHeight },
  /// Undo coverage stops below the canonical tip. The signature of a crash
  /// between the block write and the undo capture, or of a projection advanced
  /// by something that does not maintain undo.
  TipUncovered {
    tip: HyDbHeight,
    undo_hi: HyDbHeight,
  },
  /// Heights inside the retained window `[floor, tip]` have no pre-image.
  Hole {
    /// Total heights missing from the window (exact, never truncated).
    missing_heights: i64,
    /// The first [`MAX_GAP_RUNS`] runs, oldest first.
    gaps: Vec<HyDbUndoGap>,
    /// Whether more runs exist than were named.
    truncated: bool,
    /// Undo partitions absent outright inside those runs — a whole dropped
    /// partition rather than scattered missing rows.
    dropped_partitions: Vec<String>,
  },
  /// Undo rows sit above the canonical tip. Inert (a disconnect walks down
  /// from the tip and never reads them) but it means a rollback left the
  /// ledger behind, so it is worth an operator's attention.
  StaleAboveTip { tip: Option<HyDbHeight>, rows: i64 },
  /// The retained window is shallower than the deepest reorg the pipeline
  /// would attempt. Reported, never fatal — the data on hand is intact, it
  /// just does not reach as far back as the config promises.
  ShallowRetention { retained: i64, required: i64 },
}

impl HyDbUndoDefect {
  /// Whether this defect makes the database unsafe to open for writing.
  ///
  /// True exactly for the states in which a reorg cannot roll back: no undo
  /// at all, coverage short of the tip, or a hole inside the retained window.
  #[must_use]
  pub fn refuses_startup(&self) -> bool {
    matches!(
      self,
      Self::NoCoverage { .. } | Self::TipUncovered { .. } | Self::Hole { .. }
    )
  }

  /// One line, phrased for an operator reading a startup refusal.
  #[must_use]
  pub fn describe(&self) -> String {
    match self {
      Self::NoCoverage { tip } => format!(
        "canonical tip {} has no undo coverage at all (undo_blocks is empty); \
         the first reorg could not roll back",
        tip.0
      ),
      Self::TipUncovered { tip, undo_hi } => format!(
        "undo coverage stops at height {} but the canonical tip is {} — \
         {} block(s) at the tip cannot be reversed",
        undo_hi.0,
        tip.0,
        tip.0.saturating_sub(undo_hi.0)
      ),
      Self::Hole {
        missing_heights,
        gaps,
        truncated,
        dropped_partitions,
      } => {
        let runs = gaps
          .iter()
          .map(|g| format!("{}..={}", g.lo.0, g.hi.0))
          .collect::<Vec<_>>()
          .join(", ");
        let more = if *truncated { ", ..." } else { "" };
        let dropped = if dropped_partitions.is_empty() {
          String::new()
        } else {
          format!(
            "; partitions absent outright: {}",
            dropped_partitions.join(", ")
          )
        };
        format!(
          "{missing_heights} height(s) inside the retained undo window have no \
           pre-image: [{runs}{more}]{dropped}"
        )
      }
      Self::StaleAboveTip { tip, rows } => format!(
        "{rows} undo row(s) sit above the canonical tip ({}); a rollback left \
         the undo ledger behind",
        tip.map_or_else(|| "none".to_owned(), |t| t.0.to_string())
      ),
      Self::ShallowRetention { retained, required } => format!(
        "undo retains {retained} block(s) below the tip but db.ingest.max_reorg_depth \
         is {required}; a reorg deeper than the retained window would need a re-derive"
      ),
    }
  }
}

/// The state of the undo ledger relative to the canonical tip.
///
/// Every field is reported whether or not a defect was found, so the operator
/// diagnostic can render the healthy case too.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct HyDbUndoReport {
  /// The canonical tip (`MAX(height) FROM blocks`), or `None` when the
  /// projection is empty.
  pub tip: Option<HyDbHeight>,
  /// Oldest retained undo height — the prune floor.
  pub undo_lo: Option<HyDbHeight>,
  /// Newest undo height at or below the tip.
  pub undo_hi: Option<HyDbHeight>,
  /// Heights the retained window `[undo_lo, tip]` spans.
  pub expected_rows: i64,
  /// Undo rows actually present in that window.
  pub covered_rows: i64,
  /// Undo rows above the canonical tip.
  pub rows_above_tip: i64,
  /// The prune cutoff in force (`tip - undo_keep_depth`); partitions entirely
  /// at or below it are droppable. Negative before the tip passes the keep
  /// depth, and `None` on an empty projection.
  pub prune_cutoff: Option<i64>,
  /// Blocks of undo below the tip, counted from the floor.
  pub retained_depth: i64,
  /// Everything wrong, in the order found.
  pub defects: Vec<HyDbUndoDefect>,
}

impl HyDbUndoReport {
  /// No defects at all, including the survivable ones.
  #[must_use]
  pub fn is_clean(&self) -> bool {
    self.defects.is_empty()
  }

  /// The defects that do not block startup.
  pub fn warnings(&self) -> impl Iterator<Item = &HyDbUndoDefect> {
    self.defects.iter().filter(|d| !d.refuses_startup())
  }

  /// The refusal message for the startup gate, or `None` when the database is
  /// safe to open. Names the remedy, because a refusal an operator cannot act
  /// on is just an outage.
  #[must_use]
  pub fn startup_refusal(&self) -> Option<String> {
    let fatal: Vec<String> = self
      .defects
      .iter()
      .filter(|d| d.refuses_startup())
      .map(HyDbUndoDefect::describe)
      .collect();
    if fatal.is_empty() {
      return None;
    }
    Some(format!(
      "undo coverage is not contiguous up to the canonical tip: {}. \
       Rewind below the affected range or re-derive the projection from \
       ser_blocks before starting against this database",
      fatal.join("; ")
    ))
  }
}

/// Check that the undo ledger covers every height from its retained floor up
/// to the canonical tip, and grade what it does not.
///
/// Read-only: no DDL, no writes, no advisory locks — safe to run against a
/// live database from a diagnostic path while ingest is running (a concurrent
/// advance can only make the answer more complete, and a concurrent prune only
/// raises the floor).
///
/// # Errors
///
/// [`HyDbError::Pg`] if a read fails, [`HyDbError::Decode`] if a stored height
/// is negative.
pub async fn undo_coverage(pool: &PgPool) -> HyDbResult<HyDbUndoReport> {
  let tip = sqlx::query_scalar!(r#"SELECT MAX(height) AS "tip" FROM blocks"#)
    .fetch_one(pool)
    .await
    .map_err(|e| HyDbError::pg("verify: canonical tip", e))?;

  // `-1` for an empty projection keeps one code path: the window is empty and
  // every undo row, if any, counts as sitting above the tip.
  let tip_or_below_genesis = tip.unwrap_or(-1);

  let bounds = sqlx::query!(
    r#"SELECT MIN(height) FILTER (WHERE height <= $1) AS lo,
              MAX(height) FILTER (WHERE height <= $1) AS hi,
              COUNT(*) FILTER (WHERE height <= $1) AS "in_window!",
              COUNT(*) FILTER (WHERE height > $1) AS "above_tip!"
         FROM undo_blocks"#,
    tip_or_below_genesis
  )
  .fetch_one(pool)
  .await
  .map_err(|e| HyDbError::pg("verify: undo bounds", e))?;

  let mut report = HyDbUndoReport {
    tip: tip.map(HyDbHeight::try_from_i32).transpose()?,
    undo_lo: bounds.lo.map(HyDbHeight::try_from_i32).transpose()?,
    undo_hi: bounds.hi.map(HyDbHeight::try_from_i32).transpose()?,
    covered_rows: bounds.in_window,
    rows_above_tip: bounds.above_tip,
    prune_cutoff: tip.map(|t| i64::from(t) - i64::from(undo_keep_depth())),
    ..HyDbUndoReport::default()
  };

  if report.rows_above_tip > 0 {
    report.defects.push(HyDbUndoDefect::StaleAboveTip {
      tip: report.tip,
      rows: report.rows_above_tip,
    });
  }

  let (Some(tip), Some(tip_h)) = (tip, report.tip) else {
    // No canonical chain: nothing to cover. This is the fresh-database case
    // the gate must let through.
    return Ok(report);
  };

  // Both bounds are window-scoped (`FILTER (WHERE height <= tip)`): undo rows
  // above the tip are reported separately and must not move the floor.
  let (Some(lo), Some(hi)) = (bounds.lo, bounds.hi) else {
    report
      .defects
      .push(HyDbUndoDefect::NoCoverage { tip: tip_h });
    return Ok(report);
  };

  report.expected_rows = i64::from(tip) - i64::from(lo) + 1;
  report.retained_depth = i64::from(hi) - i64::from(lo) + 1;

  // Direction matters: pruning raises the floor, so a shortfall at the *top*
  // is never a prune outcome.
  if hi < tip {
    report.defects.push(HyDbUndoDefect::TipUncovered {
      tip: tip_h,
      undo_hi: HyDbHeight::try_from_i32(hi)?,
    });
  }

  // The count against the span the retained rows imply — the check `MIN`/`MAX`
  // cannot perform. Measured over `[lo, hi]`, not up to the tip: a shortfall
  // at the top is already reported as `TipUncovered`, and counting it twice
  // would put a gap in the report that no pair of rows brackets.
  let interior_span = i64::from(hi) - i64::from(lo) + 1;
  if report.covered_rows < interior_span {
    let missing_heights = interior_span - report.covered_rows;
    let (gaps, truncated) = interior_gaps(pool, tip).await?;
    let dropped_partitions = absent_partitions(pool, &gaps).await?;
    report.defects.push(HyDbUndoDefect::Hole {
      missing_heights,
      gaps,
      truncated,
      dropped_partitions,
    });
  }

  // Retention depth, only once pruning has actually moved the floor: with the
  // floor still at genesis nothing was removed and the window is simply as
  // deep as the chain is long.
  let required = max_reorg_depth();
  if lo > 0 && report.retained_depth < required {
    report.defects.push(HyDbUndoDefect::ShallowRetention {
      retained: report.retained_depth,
      required,
    });
  }

  Ok(report)
}

/// Locate the missing runs strictly inside the covered window, as
/// `(gaps, truncated)`.
///
/// One ordered pass over the undo heights with `LEAD`, rather than an
/// anti-join against `generate_series`: the ledger is bounded by the keep
/// depth, the height span is not (a database that has never pruned spans the
/// whole chain), and only the rows that exist can bound a gap.
async fn interior_gaps(pool: &PgPool, tip: i32) -> HyDbResult<(Vec<HyDbUndoGap>, bool)> {
  let rows = sqlx::query!(
    r#"SELECT lo AS "lo!", hi AS "hi!"
         FROM (SELECT height + 1 AS lo,
                      LEAD(height) OVER (ORDER BY height) - 1 AS hi
                 FROM undo_blocks
                WHERE height <= $1) runs
        WHERE hi IS NOT NULL AND hi >= lo
        ORDER BY lo
        LIMIT $2"#,
    tip,
    MAX_GAP_RUNS + 1
  )
  .fetch_all(pool)
  .await
  .map_err(|e| HyDbError::pg("verify: undo gaps", e))?;

  let truncated = rows.len() as i64 > MAX_GAP_RUNS;
  let gaps = rows
    .into_iter()
    .take(MAX_GAP_RUNS as usize)
    .map(|r| {
      Ok(HyDbUndoGap {
        lo: HyDbHeight::try_from_i32(r.lo)?,
        hi: HyDbHeight::try_from_i32(r.hi)?,
      })
    })
    .collect::<HyDbResult<Vec<_>>>()?;
  Ok((gaps, truncated))
}

/// Which `undo_blocks` partitions covering those gaps do not exist.
///
/// This is the difference between a whole partition dropped — by a prune with
/// a stale horizon, or by hand — and rows individually absent inside a
/// partition that is still there, which is what an interrupted write leaves.
/// Both are fatal for coverage; they call for different operator responses.
async fn absent_partitions(pool: &PgPool, gaps: &[HyDbUndoGap]) -> HyDbResult<Vec<String>> {
  let span = undo_span();
  if span <= 0 || gaps.is_empty() {
    return Ok(Vec::new());
  }
  let span = u32::try_from(span).unwrap_or(1);

  let mut names: Vec<String> = Vec::new();
  for gap in gaps {
    for idx in (gap.lo.0 / span)..=(gap.hi.0 / span) {
      let name = format!("undo_blocks_p{idx}");
      if !names.contains(&name) {
        names.push(name);
      }
      if names.len() >= MAX_NAMED_PARTITIONS {
        break;
      }
    }
  }

  let present: Vec<String> = sqlx::query_scalar!(
    r#"SELECT n AS "n!" FROM UNNEST($1::text[]) AS n WHERE to_regclass(n) IS NOT NULL"#,
    &names
  )
  .fetch_all(pool)
  .await
  .map_err(|e| HyDbError::pg("verify: undo partition presence", e))?;

  names.retain(|n| !present.contains(n));
  Ok(names)
}

#[cfg(test)]
mod tests {
  use super::*;

  fn h(v: u32) -> HyDbHeight {
    HyDbHeight(v)
  }

  fn gap(lo: u32, hi: u32) -> HyDbUndoGap {
    HyDbUndoGap {
      lo: h(lo),
      hi: h(hi),
    }
  }

  /// The policy split, stated as such: only the three states in which a
  /// rollback is impossible may stop a daemon from starting. A pruned floor
  /// and a stale row above the tip are reported and survived — refusing over
  /// correctly pruned undo would be worse than the gap.
  #[test]
  fn only_unrollbackable_states_refuse_startup() {
    assert!(HyDbUndoDefect::NoCoverage { tip: h(10) }.refuses_startup());
    assert!(HyDbUndoDefect::TipUncovered {
      tip: h(10),
      undo_hi: h(8)
    }
    .refuses_startup());
    assert!(HyDbUndoDefect::Hole {
      missing_heights: 1,
      gaps: vec![gap(5, 5)],
      truncated: false,
      dropped_partitions: Vec::new(),
    }
    .refuses_startup());

    assert!(!HyDbUndoDefect::StaleAboveTip {
      tip: Some(h(10)),
      rows: 3
    }
    .refuses_startup());
    assert!(!HyDbUndoDefect::ShallowRetention {
      retained: 900,
      required: 1_000
    }
    .refuses_startup());
  }

  #[test]
  fn a_clean_report_has_no_refusal() {
    let report = HyDbUndoReport {
      tip: Some(h(1_000)),
      undo_lo: Some(h(500)),
      undo_hi: Some(h(1_000)),
      expected_rows: 501,
      covered_rows: 501,
      ..HyDbUndoReport::default()
    };
    assert!(report.is_clean());
    assert!(report.startup_refusal().is_none());
  }

  /// A warning-only report still opens the database, and the warning is still
  /// visible to the operator path.
  #[test]
  fn warnings_do_not_refuse_but_are_reported() {
    let report = HyDbUndoReport {
      tip: Some(h(10_000)),
      undo_lo: Some(h(9_500)),
      undo_hi: Some(h(10_000)),
      expected_rows: 501,
      covered_rows: 501,
      retained_depth: 501,
      defects: vec![HyDbUndoDefect::ShallowRetention {
        retained: 501,
        required: 1_000,
      }],
      ..HyDbUndoReport::default()
    };
    assert!(!report.is_clean());
    assert!(report.startup_refusal().is_none());
    assert_eq!(report.warnings().count(), 1);
  }

  /// The refusal must name every fatal defect and the remedy; an operator who
  /// cannot act on it has only an outage.
  #[test]
  fn refusal_names_the_defects_and_the_remedy() {
    let report = HyDbUndoReport {
      tip: Some(h(1_000)),
      undo_hi: Some(h(998)),
      defects: vec![
        HyDbUndoDefect::TipUncovered {
          tip: h(1_000),
          undo_hi: h(998),
        },
        HyDbUndoDefect::Hole {
          missing_heights: 500,
          gaps: vec![gap(500, 999)],
          truncated: false,
          dropped_partitions: vec!["undo_blocks_p1".to_owned()],
        },
        HyDbUndoDefect::StaleAboveTip {
          tip: Some(h(1_000)),
          rows: 2,
        },
      ],
      ..HyDbUndoReport::default()
    };
    let refusal = report.startup_refusal().unwrap_or_default();
    assert!(!refusal.is_empty(), "fatal defects must produce a refusal");
    assert!(refusal.contains("998"), "{refusal}");
    assert!(refusal.contains("500..=999"), "{refusal}");
    assert!(refusal.contains("undo_blocks_p1"), "{refusal}");
    assert!(refusal.contains("re-derive"), "{refusal}");
    // The survivable defect is not part of the refusal.
    assert!(
      !refusal.contains("left the undo ledger behind"),
      "{refusal}"
    );
  }

  #[test]
  fn gap_runs_count_inclusively() {
    assert_eq!(gap(7, 7).heights(), 1);
    assert_eq!(gap(500, 999).heights(), 500);
  }
}
