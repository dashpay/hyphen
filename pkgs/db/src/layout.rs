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
