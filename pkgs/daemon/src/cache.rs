//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Small weighted LRU helper for bounded in-process read caches.

use std::collections::HashMap;
use std::future::Future;
use std::hash::Hash;
use std::sync::Arc;

use lru::LruCache;
use tokio::sync::{Mutex, Notify};

struct CacheEntry<V> {
  value: Arc<V>,
  weight: usize,
}

struct Inner<K, V> {
  entries: LruCache<K, CacheEntry<V>>,
  inflight: HashMap<K, Arc<Notify>>,
  current_weight: usize,
}

/// Weighted LRU with per-key single-flight fetch coordination.
pub struct WeightedLru<K, V> {
  enabled: bool,
  budget_bytes: usize,
  inner: Mutex<Inner<K, V>>,
}

impl<K, V> std::fmt::Debug for WeightedLru<K, V> {
  fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
    f.debug_struct("WeightedLru")
      .field("enabled", &self.enabled)
      .field("budget_bytes", &self.budget_bytes)
      .finish_non_exhaustive()
  }
}

impl<K, V> WeightedLru<K, V>
where
  K: Eq + Hash + Clone,
{
  /// Build a new weighted cache with the given byte budget.
  pub fn new(enabled: bool, budget_bytes: usize) -> Self {
    Self {
      enabled,
      budget_bytes,
      inner: Mutex::new(Inner {
        entries: LruCache::unbounded(),
        inflight: HashMap::new(),
        current_weight: 0,
      }),
    }
  }

  /// Return one cached value when present, updating LRU recency.
  pub async fn get(&self, key: &K) -> Option<Arc<V>> {
    if !self.enabled {
      return None;
    }
    let mut inner = self.inner.lock().await;
    inner.entries.get(key).map(|entry| Arc::clone(&entry.value))
  }

  /// Read one cached value or fetch and cache it on demand.
  ///
  /// # Errors
  ///
  /// Returns whatever error the supplied fetch closure produces.
  pub async fn get_or_fetch<E, F, Fut, W>(&self, key: K, fetch: F, weigh: W) -> Result<Arc<V>, E>
  where
    F: FnOnce() -> Fut,
    Fut: Future<Output = Result<V, E>>,
    W: Fn(&V) -> usize,
  {
    if !self.enabled {
      return fetch().await.map(Arc::new);
    }

    loop {
      let wait = {
        let mut inner = self.inner.lock().await;
        if let Some(entry) = inner.entries.get(&key) {
          return Ok(Arc::clone(&entry.value));
        }
        if let Some(wait) = inner.inflight.get(&key) {
          Some(Arc::clone(wait))
        } else {
          let wait = Arc::new(Notify::new());
          inner.inflight.insert(key.clone(), Arc::clone(&wait));
          None
        }
      };

      if let Some(wait) = wait {
        wait.notified().await;
        continue;
      }

      let result = fetch().await;
      let mut inner = self.inner.lock().await;
      let wait = inner.inflight.remove(&key);

      if let Ok(value) = result {
        let value = Arc::new(value);
        let weight = weigh(value.as_ref());
        if weight <= self.budget_bytes {
          if let Some(previous) = inner.entries.put(
            key,
            CacheEntry {
              value: Arc::clone(&value),
              weight,
            },
          ) {
            inner.current_weight = inner.current_weight.saturating_sub(previous.weight);
          }
          inner.current_weight = inner.current_weight.saturating_add(weight);
          while inner.current_weight > self.budget_bytes {
            let Some((_, evicted)) = inner.entries.pop_lru() else {
              break;
            };
            inner.current_weight = inner.current_weight.saturating_sub(evicted.weight);
          }
        }
        if let Some(wait) = wait {
          wait.notify_waiters();
        }
        return Ok(value);
      }

      if let Some(wait) = wait {
        wait.notify_waiters();
      }
      return result.map(Arc::new);
    }
  }
}

#[cfg(test)]
mod tests {
  use super::WeightedLru;
  use std::sync::atomic::{AtomicUsize, Ordering};
  use std::sync::Arc;

  #[tokio::test]
  async fn weighted_lru_evicts_least_recent_entry_to_fit_budget() {
    let cache = WeightedLru::<u32, String>::new(true, 8);
    let weigh = |value: &String| value.len();

    let _ = cache
      .get_or_fetch(1, || async { Ok::<_, ()>("abcd".to_owned()) }, weigh)
      .await
      .expect("cached");
    let _ = cache
      .get_or_fetch(2, || async { Ok::<_, ()>("efgh".to_owned()) }, weigh)
      .await
      .expect("cached");
    let _ = cache.get(&1).await.expect("entry 1 should be cached");
    let _ = cache
      .get_or_fetch(3, || async { Ok::<_, ()>("ijkl".to_owned()) }, weigh)
      .await
      .expect("cached");

    assert!(cache.get(&1).await.is_some());
    assert!(cache.get(&2).await.is_none());
    assert!(cache.get(&3).await.is_some());
  }

  #[tokio::test]
  async fn weighted_lru_single_flights_same_key() {
    let cache = Arc::new(WeightedLru::<u32, String>::new(true, 64));
    let calls = Arc::new(AtomicUsize::new(0));

    let mut tasks = Vec::new();
    for _ in 0..4 {
      let cache = Arc::clone(&cache);
      let calls = Arc::clone(&calls);
      tasks.push(tokio::spawn(async move {
        cache
          .get_or_fetch(
            7,
            || async move {
              calls.fetch_add(1, Ordering::Relaxed);
              tokio::time::sleep(std::time::Duration::from_millis(10)).await;
              Ok::<_, ()>("shared".to_owned())
            },
            |value| value.len(),
          )
          .await
          .expect("fetch should succeed")
      }));
    }

    for task in tasks {
      let value = task.await.expect("task should join");
      assert_eq!(value.as_ref(), "shared");
    }
    assert_eq!(calls.load(Ordering::Relaxed), 1);
  }
}
