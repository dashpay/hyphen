//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Bounded memory of txids with an observed InstantSend lock.

use std::num::NonZeroUsize;

use lru::LruCache;

const MAX_KNOWN_LOCKS: usize = 100_000;

/// Serialized-order txids whose lock has been seen, least recently seen evicted first.
///
/// A lock is permanent for its txid, so entries outlive mining, removal,
/// reorgs and re-admission. Only presence is kept: absence means "unknown",
/// never "not locked".
#[derive(Debug)]
pub(crate) struct KnownLocks(LruCache<[u8; 32], ()>);

impl Default for KnownLocks {
  fn default() -> Self {
    Self::with_capacity(MAX_KNOWN_LOCKS)
  }
}

impl KnownLocks {
  fn with_capacity(cap: usize) -> Self {
    Self(LruCache::new(
      NonZeroUsize::new(cap).unwrap_or(NonZeroUsize::MIN),
    ))
  }

  pub fn contains(&self, txid: &[u8]) -> bool {
    <&[u8; 32]>::try_from(txid).is_ok_and(|txid| self.0.contains(txid))
  }

  /// Record a lock; returns whether it was new.
  pub fn insert(&mut self, txid: [u8; 32]) -> bool {
    self.0.put(txid, ()).is_none()
  }
}

#[cfg(test)]
mod tests {
  use super::KnownLocks;

  #[test]
  fn locks_are_presence_only_and_evict_the_least_recently_seen() {
    let mut locks = KnownLocks::with_capacity(2);
    assert!(!locks.contains(&[1; 32]));
    assert!(locks.insert([1; 32]));
    assert!(locks.insert([2; 32]));
    assert!(!locks.insert([1; 32]), "a repeated lock is not new");
    assert!(locks.insert([3; 32]));
    assert!(locks.contains(&[1; 32]));
    assert!(!locks.contains(&[2; 32]), "oldest lock should be evicted");
    assert!(locks.contains(&[3; 32]));
    assert!(
      !locks.contains(&[1; 31]),
      "malformed txids are never locked"
    );
  }

  #[test]
  fn default_capacity_holds_one_hundred_thousand_locks() {
    let mut locks = KnownLocks::default();
    for n in 0..=100_000u32 {
      let mut txid = [0; 32];
      txid[..4].copy_from_slice(&n.to_le_bytes());
      locks.insert(txid);
    }
    let mut first = [0; 32];
    first[..4].copy_from_slice(&0u32.to_le_bytes());
    let mut second = [0; 32];
    second[..4].copy_from_slice(&1u32.to_le_bytes());
    assert!(!locks.contains(&first));
    assert!(locks.contains(&second));
  }
}
