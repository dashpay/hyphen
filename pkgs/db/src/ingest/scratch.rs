//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The per-block carrier the [`IngestDerive`](super::pipeline::IngestDerive) fills and the
//! [`IngestStage`s](super::pipeline::IngestStage) read, plus the two typed error domains that the
//! explicit data flow makes possible. A stage that needs a field the deriver
//! never produced gets a `MissingScratch` naming exactly which field and stage —
//! an ordering/wiring bug surfaced structurally rather than as a stray string.

use std::collections::{HashMap, HashSet};

use crate::error::HyDbError;
use crate::records::{HyDbBlock, SpentOutput};
use crate::types::HyDbOutpoint;

/// Per-address accumulation within one block, produced by the deriver.
#[derive(Default)]
pub(super) struct AddrDelta {
  pub received: i64,
  pub sent: i64,
  pub created_utxos: i64,
  pub spent_utxos: i64,
  pub txs: HashSet<Vec<u8>>,
}

/// The block-level inputs to the cumulative `block_metrics` row.
pub(super) struct MetricInputs {
  pub spendable_created: i64,
  pub unspendable_created: i64,
  pub inputs_spent: i64,
  pub coinbase_value: i64,
  /// Sum of all non-coinbase transaction fees in the block. The true supply
  /// delta is `coinbase_value - fee_total` (the subsidy).
  pub fee_total: i64,
  /// Value newly parked in addressless outputs minus value released from
  /// them, so `supply_summary.unspendable_value` stays exact.
  pub unspendable_delta: i64,
  pub raw_len: i64,
}

/// The block-scoped scratchpad. The spine seeds `height`, the block's
/// serialized length and the decoded block itself; the deriver fills the rest
/// read-only; stages consume it through the `require`-style accessors below.
///
/// The raw bytes are deliberately *not* here. Decoding moved off the write
/// transaction in A4, so by the time a scratch exists the bytes have already
/// served their only purpose and the length is all that survives them — which
/// is also all the projection ever wanted (`blocks.size`, and the metric
/// inputs).
pub(super) struct Scratch {
  pub height: i32,
  pub raw_len: i64,
  block: Option<HyDbBlock>,
  /// Resolved prevout per input, indexed `[tx][vin]`; `None` for a coinbase
  /// input.
  resolved: Option<Vec<Vec<Option<SpentOutput>>>>,
  /// Fee per transaction, indexed `[tx]`; `None` for the coinbase.
  fees: Option<Vec<Option<i64>>>,
  address_deltas: Option<HashMap<String, AddrDelta>>,
  spent_outpoints: Option<Vec<HyDbOutpoint>>,
  metrics: Option<MetricInputs>,
  /// Cumulative chainwork through this block (64-char hex), accumulated by
  /// the deriver from the parent's stored chainwork plus this block's proof.
  chainwork: Option<String>,
}

impl Scratch {
  pub(super) fn new(height: i32, raw_len: i64) -> Self {
    Self {
      height,
      raw_len,
      block: None,
      resolved: None,
      fees: None,
      address_deltas: None,
      spent_outpoints: None,
      metrics: None,
      chainwork: None,
    }
  }

  /// Record everything the deriver computed for the block, in one shot.
  #[allow(clippy::too_many_arguments)]
  pub(super) fn fill(
    &mut self,
    block: HyDbBlock,
    resolved: Vec<Vec<Option<SpentOutput>>>,
    fees: Vec<Option<i64>>,
    address_deltas: HashMap<String, AddrDelta>,
    spent_outpoints: Vec<HyDbOutpoint>,
    metrics: MetricInputs,
    chainwork: Option<String>,
  ) {
    self.block = Some(block);
    self.resolved = Some(resolved);
    self.fees = Some(fees);
    self.address_deltas = Some(address_deltas);
    self.spent_outpoints = Some(spent_outpoints);
    self.metrics = Some(metrics);
    self.chainwork = chainwork;
  }

  pub(super) fn block(&self, stage: &'static str) -> Result<&HyDbBlock, HyDbStageError> {
    self.block.as_ref().ok_or(HyDbStageError::MissingScratch {
      stage,
      field: "block",
    })
  }

  pub(super) fn resolved(
    &self,
    stage: &'static str,
  ) -> Result<&[Vec<Option<SpentOutput>>], HyDbStageError> {
    self
      .resolved
      .as_deref()
      .ok_or(HyDbStageError::MissingScratch {
        stage,
        field: "resolved",
      })
  }

  pub(super) fn fees(&self, stage: &'static str) -> Result<&[Option<i64>], HyDbStageError> {
    self.fees.as_deref().ok_or(HyDbStageError::MissingScratch {
      stage,
      field: "fees",
    })
  }

  pub(super) fn address_deltas(
    &self,
    stage: &'static str,
  ) -> Result<&HashMap<String, AddrDelta>, HyDbStageError> {
    self
      .address_deltas
      .as_ref()
      .ok_or(HyDbStageError::MissingScratch {
        stage,
        field: "address_deltas",
      })
  }

  pub(super) fn spent_outpoints(
    &self,
    stage: &'static str,
  ) -> Result<&[HyDbOutpoint], HyDbStageError> {
    self
      .spent_outpoints
      .as_deref()
      .ok_or(HyDbStageError::MissingScratch {
        stage,
        field: "spent_outpoints",
      })
  }

  pub(super) fn metrics(&self, stage: &'static str) -> Result<&MetricInputs, HyDbStageError> {
    self.metrics.as_ref().ok_or(HyDbStageError::MissingScratch {
      stage,
      field: "metrics",
    })
  }

  /// Cumulative chainwork hex for this block, when the deriver could compute
  /// it (`None` only if the parent row carried unparseable chainwork).
  pub(super) fn chainwork(&self) -> Option<&str> {
    self.chainwork.as_deref()
  }
}

// ===========================================================================
// Typed error domains
// ===========================================================================

/// Everything the read-only fact-derivation layer can fail on. Distinct from
/// [`HyDbStageError`] so the orchestrator can route each cause to its own
/// recovery (a `MissingPrevout` re-derives; a transient `Pg` retries).
#[derive(Debug)]
pub(super) enum HyDbDeriveError {
  /// The caller-supplied data source could not decode the block bytes.
  Decode(String),
  /// A non-coinbase input consumes an outpoint absent from both this block's
  /// own outputs and the persisted UTXO set — the block references an
  /// unknown/already-spent output, or the UTXO projection is inconsistent.
  MissingPrevout { outpoint: HyDbOutpoint, height: i32 },
  /// A base fact violated an invariant the derivation assumes.
  Malformed { what: String },
  /// A read against Postgres failed. Wraps the `HyDbError` so the SQLSTATE — and
  /// thus the transient/permanent classification — is preserved.
  Pg(HyDbError),
}

impl HyDbDeriveError {
  pub(super) fn pg(context: &str, error: sqlx::Error) -> Self {
    Self::Pg(HyDbError::pg(context, error))
  }

  /// Whether retrying the block might succeed (a transient datastore fault).
  pub(super) fn transient(&self) -> bool {
    matches!(self, Self::Pg(e) if e.is_transient())
  }

  /// Whether a higher-level recovery (re-derive the affected range) applies.
  pub(super) fn recoverable(&self) -> bool {
    matches!(self, Self::MissingPrevout { .. })
  }
}

impl From<HyDbDeriveError> for HyDbError {
  fn from(e: HyDbDeriveError) -> Self {
    match e {
      HyDbDeriveError::Decode(s) => HyDbError::Decode(s),
      HyDbDeriveError::MissingPrevout { outpoint, height } => HyDbError::Derive(format!(
        "input at height {height} spends an unknown outpoint {}",
        hex::encode(outpoint.packed())
      )),
      HyDbDeriveError::Malformed { what } => HyDbError::Derive(what),
      HyDbDeriveError::Pg(e) => e,
    }
  }
}

/// Everything an accumulation stage can fail on. `MissingScratch`/`Inconsistent`
/// are wiring/invariant bugs (unreachable in a correctly-ordered pipeline, but
/// typed so they name the culprit); `Pg` is a datastore fault.
#[derive(Debug)]
pub(super) enum HyDbStageError {
  /// A prior producer did not populate scratch this stage requires.
  MissingScratch {
    stage: &'static str,
    field: &'static str,
  },
  /// Scratch was present but violates this stage's expectation.
  Inconsistent { stage: &'static str, what: String },
  /// A write against Postgres failed. Wraps the `HyDbError`, preserving code.
  /// `stage` names the culprit for diagnostics; it rides the `Debug` derive.
  Pg {
    #[allow(dead_code)]
    stage: &'static str,
    source: HyDbError,
  },
}

impl HyDbStageError {
  /// Map an `sqlx::Error` from a stage's raw query into a typed stage error.
  pub(super) fn sql(stage: &'static str, context: &'static str) -> impl Fn(sqlx::Error) -> Self {
    move |error| HyDbStageError::Pg {
      stage,
      source: HyDbError::pg(context, error),
    }
  }

  /// Map a `HyDbError` returned by a facts insert into a typed stage error.
  pub(super) fn wrap(stage: &'static str) -> impl Fn(HyDbError) -> Self {
    move |source| HyDbStageError::Pg { stage, source }
  }

  /// Whether retrying the block might succeed.
  pub(super) fn transient(&self) -> bool {
    matches!(self, Self::Pg { source, .. } if source.is_transient())
  }
}

impl From<HyDbStageError> for HyDbError {
  fn from(e: HyDbStageError) -> Self {
    match e {
      HyDbStageError::MissingScratch { stage, field } => {
        HyDbError::Derive(format!("stage `{stage}` missing scratch field `{field}`"))
      }
      HyDbStageError::Inconsistent { stage, what } => {
        HyDbError::Derive(format!("stage `{stage}` inconsistent: {what}"))
      }
      HyDbStageError::Pg { source, .. } => source,
    }
  }
}

/// A failure anywhere in one block's connect, tagged with its layer so
/// [`advance`](super::pipeline::advance) can branch on the recovery strategy before
/// flattening to a public [`HyDbError`].
#[derive(Debug)]
pub(super) enum PipelineError {
  Derive(HyDbDeriveError),
  Stage(HyDbStageError),
  /// A spine-level datastore fault (partition setup, transaction begin/commit).
  Spine(HyDbError),
}

impl PipelineError {
  pub(super) fn transient(&self) -> bool {
    match self {
      Self::Derive(e) => e.transient(),
      Self::Stage(e) => e.transient(),
      Self::Spine(e) => e.is_transient(),
    }
  }

  /// A `MissingPrevout` is the one cause a higher-level re-derive can fix.
  pub(super) fn recoverable(&self) -> bool {
    matches!(self, Self::Derive(e) if e.recoverable())
  }
}

impl From<HyDbDeriveError> for PipelineError {
  fn from(e: HyDbDeriveError) -> Self {
    Self::Derive(e)
  }
}

impl From<HyDbStageError> for PipelineError {
  fn from(e: HyDbStageError) -> Self {
    Self::Stage(e)
  }
}

impl From<HyDbError> for PipelineError {
  fn from(e: HyDbError) -> Self {
    Self::Spine(e)
  }
}

impl From<PipelineError> for HyDbError {
  fn from(e: PipelineError) -> Self {
    match e {
      PipelineError::Derive(e) => e.into(),
      PipelineError::Stage(e) => e.into(),
      PipelineError::Spine(e) => e,
    }
  }
}

#[cfg(test)]
mod tests {
  use super::*;
  use crate::types::HyDbHash;
  use rstest::rstest;

  fn transient_pg() -> HyDbError {
    HyDbError::Pg {
      message: "pool timed out".into(),
      code: None,
    }
  }
  fn permanent_pg() -> HyDbError {
    HyDbError::Pg {
      message: "dup".into(),
      code: Some("23505".into()),
    }
  }
  fn outpoint() -> HyDbOutpoint {
    HyDbOutpoint {
      txid: HyDbHash([9u8; 32]),
      vout: 3,
    }
  }

  // ---- Scratch accessors: the missing-field error arm --------------------

  #[rstest]
  #[case("block")]
  #[case("resolved")]
  #[case("fees")]
  #[case("address_deltas")]
  #[case("spent_outpoints")]
  #[case("metrics")]
  fn empty_scratch_reports_missing_field(#[case] field: &'static str) {
    let s = Scratch::new(0, 0);
    let err = match field {
      "block" => s.block("stg").err(),
      "resolved" => s.resolved("stg").err(),
      "fees" => s.fees("stg").err(),
      "address_deltas" => s.address_deltas("stg").err(),
      "spent_outpoints" => s.spent_outpoints("stg").err(),
      _ => s.metrics("stg").err(),
    };
    assert!(
      matches!(err, Some(HyDbStageError::MissingScratch { stage: "stg", field: f }) if f == field),
      "expected MissingScratch for `{field}`, got {err:?}"
    );
  }

  // ---- HyDbDeriveError ----------------------------------------------------

  #[test]
  fn derive_error_transient_only_for_transient_pg() {
    assert!(HyDbDeriveError::Pg(transient_pg()).transient());
    assert!(!HyDbDeriveError::Pg(permanent_pg()).transient());
    assert!(!HyDbDeriveError::Decode("x".into()).transient());
    assert!(!HyDbDeriveError::Malformed { what: "x".into() }.transient());
    assert!(!HyDbDeriveError::MissingPrevout {
      outpoint: outpoint(),
      height: 1
    }
    .transient());
  }

  #[test]
  fn derive_error_recoverable_only_for_missing_prevout() {
    assert!(HyDbDeriveError::MissingPrevout {
      outpoint: outpoint(),
      height: 1
    }
    .recoverable());
    assert!(!HyDbDeriveError::Decode("x".into()).recoverable());
    assert!(!HyDbDeriveError::Malformed { what: "x".into() }.recoverable());
    assert!(!HyDbDeriveError::Pg(transient_pg()).recoverable());
  }

  #[test]
  fn derive_error_maps_to_public_error() {
    assert!(matches!(
      HyDbError::from(HyDbDeriveError::Decode("boom".into())),
      HyDbError::Decode(m) if m == "boom"
    ));
    assert!(matches!(
      HyDbError::from(HyDbDeriveError::Malformed { what: "bad".into() }),
      HyDbError::Derive(m) if m == "bad"
    ));
    assert!(matches!(
      HyDbError::from(HyDbDeriveError::MissingPrevout { outpoint: outpoint(), height: 7 }),
      HyDbError::Derive(m) if m.contains("height 7") && m.contains("unknown outpoint")
    ));
    // A wrapped Pg passes through unchanged (SQLSTATE preserved).
    assert!(matches!(
      HyDbError::from(HyDbDeriveError::Pg(permanent_pg())),
      HyDbError::Pg { code: Some(c), .. } if c == "23505"
    ));
  }

  // ---- HyDbStageError -----------------------------------------------------

  #[test]
  fn stage_error_transient_only_for_transient_pg() {
    assert!(HyDbStageError::Pg {
      stage: "s",
      source: transient_pg()
    }
    .transient());
    assert!(!HyDbStageError::Pg {
      stage: "s",
      source: permanent_pg()
    }
    .transient());
    assert!(!HyDbStageError::MissingScratch {
      stage: "s",
      field: "f"
    }
    .transient());
    assert!(!HyDbStageError::Inconsistent {
      stage: "s",
      what: "w".into()
    }
    .transient());
  }

  #[test]
  fn stage_error_wrap_builds_pg_preserving_source() {
    let err = HyDbStageError::wrap("mystage")(permanent_pg());
    assert!(!err.transient());
    assert!(matches!(
      HyDbError::from(err),
      HyDbError::Pg { code: Some(c), .. } if c == "23505"
    ));
  }

  #[test]
  fn stage_error_maps_to_public_error() {
    assert!(matches!(
      HyDbError::from(HyDbStageError::MissingScratch { stage: "base", field: "block" }),
      HyDbError::Derive(m) if m.contains("base") && m.contains("block")
    ));
    assert!(matches!(
      HyDbError::from(HyDbStageError::Inconsistent { stage: "stats", what: "no undo".into() }),
      HyDbError::Derive(m) if m.contains("stats") && m.contains("no undo")
    ));
    assert!(matches!(
      HyDbError::from(HyDbStageError::Pg {
        stage: "s",
        source: permanent_pg()
      }),
      HyDbError::Pg { .. }
    ));
  }

  // ---- PipelineError ------------------------------------------------------

  #[rstest]
  #[case(PipelineError::from(HyDbDeriveError::Pg(transient_pg())), true, false)]
  #[case(PipelineError::from(HyDbDeriveError::MissingPrevout { outpoint: outpoint(), height: 1 }), false, true)]
  #[case(PipelineError::from(HyDbDeriveError::Decode("x".into())), false, false)]
  #[case(PipelineError::from(HyDbStageError::Pg { stage: "s", source: transient_pg() }), true, false)]
  #[case(PipelineError::from(HyDbStageError::MissingScratch { stage: "s", field: "f" }), false, false)]
  #[case(PipelineError::from(transient_pg()), true, false)]
  #[case(PipelineError::from(permanent_pg()), false, false)]
  fn pipeline_error_classification(
    #[case] err: PipelineError,
    #[case] transient: bool,
    #[case] recoverable: bool,
  ) {
    assert_eq!(err.transient(), transient);
    assert_eq!(err.recoverable(), recoverable);
  }

  #[test]
  fn pipeline_error_from_conversions_tag_the_layer() {
    assert!(matches!(
      PipelineError::from(HyDbDeriveError::Decode("d".into())),
      PipelineError::Derive(_)
    ));
    assert!(matches!(
      PipelineError::from(HyDbStageError::MissingScratch {
        stage: "s",
        field: "f"
      }),
      PipelineError::Stage(_)
    ));
    assert!(matches!(
      PipelineError::from(transient_pg()),
      PipelineError::Spine(_)
    ));
  }

  #[test]
  fn pipeline_error_flattens_to_public_error() {
    // Spine passes the inner HyDbError through untouched.
    assert!(matches!(
      HyDbError::from(PipelineError::Spine(permanent_pg())),
      HyDbError::Pg { code: Some(c), .. } if c == "23505"
    ));
    // Derive/Stage delegate to their own mappings.
    assert!(matches!(
      HyDbError::from(PipelineError::from(HyDbDeriveError::Malformed { what: "m".into() })),
      HyDbError::Derive(m) if m == "m"
    ));
    assert!(matches!(
      HyDbError::from(PipelineError::from(HyDbStageError::MissingScratch {
        stage: "s",
        field: "f"
      })),
      HyDbError::Derive(_)
    ));
  }
}
