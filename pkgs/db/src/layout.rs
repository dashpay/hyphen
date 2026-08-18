//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Storage-layout parameters: values that shape what the database *contains*,
//! as opposed to how it is connected to. They are fixed for the life of a
//! database — changing one invalidates data already written — so they live as
//! constants here rather than as runtime knobs.

/// Minimum balance (in duffs) for an address to be carried in `address_rich`,
/// the maintained rich-list side table.
///
/// The rich list is served by ranking *within* this set, so the floor must sit
/// far enough below any depth a caller can reach that truncation is not
/// observable in practice; it must never be "the Nth balance", because that
/// value moves with every block. At 10 DASH the mainnet set is ~33,200
/// addresses (measured at height 967,013) against ~785,600 with any positive
/// balance — 66 full pages at the rich-list API's 500-row page cap, and 4% of
/// the rows a balance-indexed `address_stats` would have had to carry.
///
/// Under task T1 this becomes `[db.layout] rich_list_floor`, graded
/// *rebuildable*: changing it does not require a resync, because
/// `address_rich` is derivable from `address_stats` in a single scan —
/// it requires an offline rebuild of this one table.
pub const RICH_LIST_FLOOR: i64 = 1_000_000_000;

/// Height span of one *undo* partition.
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
/// relation count on a table the ingest inserts into on every block. Neither
/// end of that trade pays, so the span sits in the middle.
///
/// It is deliberately *not* derived from the keep depth: the drop rule is
/// keyed on the prune cutoff, so a larger configured depth lowers the cutoff
/// and drops strictly fewer partitions. Span and depth cannot conspire to
/// drop a partition early.
pub const UNDO_PARTITION_SPAN: i32 = 500;
