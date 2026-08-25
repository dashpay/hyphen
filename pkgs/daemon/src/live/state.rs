//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! In-RAM live/ephemeral node state.
//!
//! This is the data that is **not** chain-derivable — mempool, governance,
//! InstantSend/ChainLock locks, network snapshot, live ProTx — and therefore
//! does not belong in `hyphen-db`. Historically it was mirrored into Postgres
//! (`sql/live/**`); it now lives here, in process. The refresh loops (JSON-RPC
//! poll and ZMQ) write these cells; the REST handlers read them.
//!
//! Concurrency: singleton domains hold a [`SingletonCell`] (an `Arc`-swapping
//! `RwLock`); keyed domains (added as each is ported) hold a `RwLock<HashMap>`.
//! Reads clone an `Arc` under a short read lock and never hold it across an
//! `.await`.
//!
//! Poisoning is handled by [`Shared`], not by each call site.
//! Release builds set `panic = "abort"`, so a lock cannot poison in production
//! at all; under test it can, and the previous handling — `if let Ok(guard)`,
//! `.ok()`, `.unwrap_or_default()` — turned that into silently empty reads and
//! silently dropped writes. `Shared` recovers the guard instead: the data
//! behind a poisoned lock is intact, poisoning only records that some thread
//! panicked while holding it.
//
// Wiring note: the loops/handlers repoint domain-by-domain; until a domain is
// switched over, its cell is written but not yet read (or vice versa).
#![allow(dead_code)]

use std::borrow::Borrow;
use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use serde_json::Value;

/// The daemon's in-RAM live state, shared (behind an `Arc`) by the refresh
/// workers that populate it and the REST handlers that read it.
#[derive(Debug, Default)]
pub struct LiveState {
  chainlock: SingletonCell<ChainLockSnapshot>,
  network: SingletonCell<NetworkSnapshot>,
  islock: KeyedCell<Vec<u8>, IsLock>,
  protx: SingletonCell<ProtxSnapshot>,
  mempool_summary: SingletonCell<MempoolSummary>,
  mempool: Shared<MempoolTxs>,
  governance: Shared<Governance>,
}

impl LiveState {
  pub fn new() -> Self {
    Self::default()
  }

  /// Latest ChainLock, or `None` before the first refresh.
  pub fn chainlock(&self) -> Option<Arc<ChainLockSnapshot>> {
    self.chainlock.get()
  }

  pub fn set_chainlock(&self, snapshot: ChainLockSnapshot) {
    self.chainlock.set(snapshot);
  }

  /// Latest network snapshot, or `None` before the first refresh.
  pub fn network(&self) -> Option<Arc<NetworkSnapshot>> {
    self.network.get()
  }

  pub fn set_network(&self, snapshot: NetworkSnapshot) {
    self.network.set(snapshot);
  }

  /// The current InstantSend lock for a txid (serialized-endian key), if any.
  pub fn islock(&self, txid: &[u8]) -> Option<Arc<IsLock>> {
    self.islock.get(txid)
  }

  pub fn upsert_islock(&self, islock: IsLock) {
    let txid = islock.txid.clone();
    self.islock.insert(txid, islock);
  }

  pub fn remove_islock(&self, txid: &[u8]) {
    self.islock.remove(txid);
  }

  /// The current live ProTx (validator) snapshot, or `None` before first poll.
  pub fn protx(&self) -> Option<Arc<ProtxSnapshot>> {
    self.protx.get()
  }

  pub fn set_protx(&self, snapshot: ProtxSnapshot) {
    self.protx.set(snapshot);
  }

  /// Latest mempool summary, or `None` before the first refresh.
  pub fn mempool_summary(&self) -> Option<Arc<MempoolSummary>> {
    self.mempool_summary.get()
  }

  pub fn set_mempool_summary(&self, summary: MempoolSummary) {
    self.mempool_summary.set(summary);
  }

  /// One mempool transaction by txid (serialized-endian), if currently held.
  pub fn mempool_tx(&self, txid: &[u8]) -> Option<Arc<MempoolTx>> {
    self.mempool.read().txs.get(txid).cloned()
  }

  /// Recent mempool txs, newest-first by `(last_seen_at, txid)`, after an
  /// optional keyset cursor, capped at `limit`.
  pub fn mempool_recent(&self, limit: usize, after: Option<(i64, &[u8])>) -> Vec<Arc<MempoolTx>> {
    let guard = self.mempool.read();
    let mut txs: Vec<Arc<MempoolTx>> = guard.txs.values().cloned().collect();
    // Newest-first, txid-tiebroken descending — matches the SQL keyset order.
    txs.sort_by(|a, b| {
      b.last_seen_at
        .cmp(&a.last_seen_at)
        .then_with(|| b.txid.cmp(&a.txid))
    });
    txs
      .into_iter()
      .filter(|tx| match after {
        Some((last_seen_at, txid)) => (tx.last_seen_at, tx.txid.as_slice()) < (last_seen_at, txid),
        None => true,
      })
      .take(limit)
      .collect()
  }

  /// Txids of recent mempool txs touching `address`.
  pub fn mempool_by_address(&self, address: &str) -> Vec<Vec<u8>> {
    self
      .mempool
      .read()
      .by_address
      .get(address)
      .map(|set| set.iter().cloned().collect())
      .unwrap_or_default()
  }

  pub fn mempool_len(&self) -> usize {
    self.mempool.read().txs.len()
  }

  pub fn insert_mempool_tx(&self, tx: MempoolTx) {
    let mut guard = self.mempool.write();
    guard.insert(tx);
  }

  pub fn remove_mempool_tx(&self, txid: &[u8]) {
    let mut guard = self.mempool.write();
    guard.remove(txid);
  }

  /// Replace the whole recent set (the RPC snapshot rebuild).
  pub fn replace_mempool(&self, txs: Vec<MempoolTx>) {
    let mut guard = self.mempool.write();
    *guard = MempoolTxs::default();
    for tx in txs {
      guard.insert(tx);
    }
  }

  /// Governance objects (proposals), newest-first by `(updated_at,
  /// object_hash)`, after an optional keyset cursor, capped at `limit`.
  pub fn governance_objects(
    &self,
    limit: usize,
    after: Option<(i64, &[u8])>,
  ) -> Vec<Arc<GovObject>> {
    let guard = self.governance.read();
    let mut objects: Vec<Arc<GovObject>> = guard.objects.values().cloned().collect();
    sort_governance_objects(&mut objects);
    objects
      .into_iter()
      .filter(|object| governance_object_after(object, after))
      .take(limit)
      .collect()
  }

  /// One governance object by hash, if currently held.
  pub fn governance_object(&self, object_hash: &[u8]) -> Option<Arc<GovObject>> {
    self.governance.read().objects.get(object_hash).cloned()
  }

  /// All governance objects, newest-first by `(updated_at, object_hash)`.
  pub fn governance_all_objects(&self) -> Vec<Arc<GovObject>> {
    let guard = self.governance.read();
    let mut objects: Vec<Arc<GovObject>> = guard.objects.values().cloned().collect();
    sort_governance_objects(&mut objects);
    objects
  }

  /// The current governance object count.
  pub fn governance_object_total(&self) -> i64 {
    self.governance.read().objects.len() as i64
  }

  /// Current votes for one object, newest-first by `(updated_at, vote_hash)`,
  /// after an optional keyset cursor, capped at `limit`.
  pub fn governance_votes(
    &self,
    object_hash: &[u8],
    limit: usize,
    after: Option<(i64, &[u8])>,
  ) -> Vec<Arc<GovVote>> {
    let guard = self.governance.read();
    let Some(votes) = guard.votes.get(object_hash) else {
      return Vec::new();
    };
    let mut votes: Vec<Arc<GovVote>> = votes.clone();
    sort_governance_votes(&mut votes);
    votes
      .into_iter()
      .filter(|vote| governance_vote_after(vote, after))
      .take(limit)
      .collect()
  }

  /// Governance objects matching a hash-prefix or free-text `query`,
  /// newest-first, capped at `limit`.
  pub fn governance_search(&self, query: &str, limit: usize) -> Vec<Arc<GovObject>> {
    let guard = self.governance.read();
    let hash_prefix = query.to_ascii_lowercase();
    let text_needle = query.to_ascii_lowercase();
    let mut matches: Vec<Arc<GovObject>> = guard
      .objects
      .values()
      .filter(|object| governance_object_matches(object, &hash_prefix, &text_needle))
      .cloned()
      .collect();
    sort_governance_objects(&mut matches);
    matches.truncate(limit);
    matches
  }

  /// Distinct regular/evonode voter counts per object, resolved against the
  /// live ProTx snapshot (no join to indexed chain data).
  pub fn governance_vote_type_counts(
    &self,
    object_hashes: &[Vec<u8>],
  ) -> HashMap<Vec<u8>, GovVoteTypeCounts> {
    if object_hashes.is_empty() {
      return HashMap::new();
    }
    let protx = self.protx();
    let guard = self.governance.read();
    let mut counts = HashMap::new();
    for object_hash in object_hashes {
      let Some(votes) = guard.votes.get(object_hash) else {
        continue;
      };
      let mut voters: HashSet<String> = HashSet::new();
      for vote in votes {
        if let Some(pro_tx_hash_hex) = vote_pro_tx_hash_hex(&vote.raw_vote) {
          voters.insert(pro_tx_hash_hex);
        }
      }
      let mut tally = GovVoteTypeCounts::default();
      for pro_tx_hash_hex in &voters {
        let class = protx
          .as_ref()
          .and_then(|snapshot| {
            hex::decode(pro_tx_hash_hex).ok().and_then(|bytes| {
              snapshot
                .get(&bytes)
                .map(|entry| entry.validator_type.clone())
            })
          })
          .map(|validator_type| public_validator_type(validator_type.as_deref()))
          .unwrap_or("unknown");
        match class {
          "regular" => tally.voted_regular_count += 1,
          "evonode" => tally.voted_evo_count += 1,
          _ => {}
        }
      }
      counts.insert(object_hash.clone(), tally);
    }
    counts
  }

  /// Replace the whole governance set (the RPC snapshot rebuild): objects and
  /// their votes together.
  pub fn replace_governance(&self, objects: Vec<GovObject>, votes: Vec<GovVote>) {
    let mut guard = self.governance.write();
    *guard = Governance::default();
    for object in objects {
      guard.insert_object(object);
    }
    guard.replace_all_votes(votes);
  }

  /// Replace every object's votes wholesale, leaving the objects untouched.
  pub fn replace_governance_votes(&self, votes: Vec<GovVote>) {
    let mut guard = self.governance.write();
    guard.replace_all_votes(votes);
  }

  /// Upsert one governance object without disturbing other objects or votes.
  pub fn upsert_governance_object(&self, object: GovObject) {
    let mut guard = self.governance.write();
    guard.insert_object(object);
  }

  /// Replace the votes for one object, leaving other objects' votes untouched.
  pub fn replace_governance_votes_for_object(&self, object_hash: &[u8], votes: Vec<GovVote>) {
    let mut guard = self.governance.write();
    if votes.is_empty() {
      guard.votes.remove(object_hash);
    } else {
      guard.votes.insert(
        object_hash.to_vec(),
        votes.into_iter().map(Arc::new).collect(),
      );
    }
  }
}

/// A single-value cell holding the latest snapshot of one live domain. Writes
/// A `RwLock` whose guards cannot be a `Result`.
///
/// Poisoning means a thread panicked while holding the lock, not that the data
/// is unusable, so the guard is recovered with `into_inner()` and the fact is
/// logged once per process. Release builds abort on panic and can never reach
/// this; what it removes is the dev/test failure mode where a poisoned lock
/// made every live-state read look like "no data".
#[derive(Debug)]
struct Shared<T>(RwLock<T>);

/// Logged at most once: a poisoned lock is one event with many consequences,
/// and a warning per read would bury it.
static POISON_REPORTED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

fn report_poisoned(access: &'static str, location: &'static std::panic::Location<'static>) {
  if !POISON_REPORTED.swap(true, std::sync::atomic::Ordering::Relaxed) {
    tracing::warn!(
      access,
      location = %location,
      "live-state lock was poisoned by a panicking thread; recovering the data \
       behind it. Release builds abort on panic and cannot reach this."
    );
  }
}

impl<T> Shared<T> {
  fn new(value: T) -> Self {
    Self(RwLock::new(value))
  }

  #[track_caller]
  fn read(&self) -> RwLockReadGuard<'_, T> {
    self.0.read().unwrap_or_else(|poisoned| {
      report_poisoned("read", std::panic::Location::caller());
      poisoned.into_inner()
    })
  }

  #[track_caller]
  fn write(&self) -> RwLockWriteGuard<'_, T> {
    self.0.write().unwrap_or_else(|poisoned| {
      report_poisoned("write", std::panic::Location::caller());
      poisoned.into_inner()
    })
  }
}

impl<T: Default> Default for Shared<T> {
  fn default() -> Self {
    Self::new(T::default())
  }
}

/// replace the whole value; reads hand back a cheap `Arc` clone.
#[derive(Debug)]
struct SingletonCell<T>(Shared<Option<Arc<T>>>);

impl<T> SingletonCell<T> {
  fn get(&self) -> Option<Arc<T>> {
    self.0.read().clone()
  }

  fn set(&self, value: T) {
    let mut guard = self.0.write();
    *guard = Some(Arc::new(value));
  }
}

impl<T> Default for SingletonCell<T> {
  fn default() -> Self {
    Self(Shared::new(None))
  }
}

/// A keyed set of live values (e.g. mempool txs, InstantSend locks) behind one
/// `RwLock`. Reads clone an `Arc`; writes insert/remove/replace whole entries.
#[derive(Debug)]
struct KeyedCell<K, V>(Shared<HashMap<K, Arc<V>>>);

impl<K: Eq + Hash, V> KeyedCell<K, V> {
  fn get<Q>(&self, key: &Q) -> Option<Arc<V>>
  where
    K: Borrow<Q>,
    Q: Hash + Eq + ?Sized,
  {
    self.0.read().get(key).cloned()
  }

  fn insert(&self, key: K, value: V) {
    self.0.write().insert(key, Arc::new(value));
  }

  fn remove<Q>(&self, key: &Q)
  where
    K: Borrow<Q>,
    Q: Hash + Eq + ?Sized,
  {
    self.0.write().remove(key);
  }

  /// Snapshot of all current values (a cheap `Arc` clone each).
  fn values(&self) -> Vec<Arc<V>> {
    self.0.read().values().cloned().collect()
  }

  fn len(&self) -> usize {
    self.0.read().len()
  }
}

impl<K, V> Default for KeyedCell<K, V> {
  fn default() -> Self {
    Self(Shared::new(HashMap::new()))
  }
}

/// One currently known InstantSend tx-lock — the locked tx bytes and when seen.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct IsLock {
  /// Serialized-endian txid (the key).
  pub txid: Vec<u8>,
  /// Raw `pubrawtxlock` payload bytes (the locked transaction).
  pub raw_islock: Vec<u8>,
  pub seen_at: i64,
}

/// Latest ChainLock as reported by the node (`getbestchainlock`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChainLockSnapshot {
  pub block_hash: Vec<u8>,
  pub block_height: Option<i32>,
  pub core_height: Option<i32>,
  pub raw_chainlock: Vec<u8>,
  pub seen_at: i64,
}

/// Latest network snapshot: the merged `networkInfo` + `peerInfo` blob.
#[derive(Debug, Clone)]
pub struct NetworkSnapshot {
  pub payload: Value,
  pub updated_at: i64,
}

/// One validator's live ProTx status, as reported by `protx list`.
#[derive(Debug, Clone)]
pub struct ProTxEntry {
  pub pro_tx_hash: Vec<u8>,
  pub validator_type: Option<String>,
  pub current_status: Option<String>,
  pub last_paid_height: Option<i32>,
  pub consecutive_payments: Option<i32>,
  pub pose_penalty: Option<i32>,
  pub pose_revived_height: Option<i32>,
  pub pose_ban_height: Option<i32>,
  pub revocation_reason: Option<i32>,
  pub updated_at: i64,
}

/// The whole live ProTx snapshot, replaced wholesale each poll. Self-contained:
/// the summary/aggregate views are computed from the entries' own
/// `validator_type`/`current_status`, with no join to indexed chain data.
#[derive(Debug, Default)]
pub struct ProtxSnapshot {
  entries: HashMap<Vec<u8>, ProTxEntry>,
}

impl ProtxSnapshot {
  pub fn from_entries(entries: Vec<ProTxEntry>) -> Self {
    Self {
      entries: entries
        .into_iter()
        .map(|entry| (entry.pro_tx_hash.clone(), entry))
        .collect(),
    }
  }

  /// The live status for one validator by pro_tx_hash (serialized-endian).
  pub fn get(&self, pro_tx_hash: &[u8]) -> Option<&ProTxEntry> {
    self.entries.get(pro_tx_hash)
  }

  /// All live ProTx entries, in arbitrary order.
  pub fn values(&self) -> impl Iterator<Item = &ProTxEntry> {
    self.entries.values()
  }

  pub fn summary(&self) -> ProtxSummary {
    ProtxSummary {
      total_nodes: self.entries.len() as i64,
      enabled_nodes: self
        .entries
        .values()
        .filter(|entry| entry.current_status.as_deref() == Some("valid"))
        .count() as i64,
    }
  }

  pub fn aggregate(&self) -> ProtxAggregate {
    let mut agg = ProtxAggregate::default();
    for entry in self.entries.values() {
      let (total, buckets) = match public_validator_type(entry.validator_type.as_deref()) {
        "regular" => (&mut agg.regular_total, &mut agg.regular_status_buckets),
        "evonode" => (&mut agg.evonode_total, &mut agg.evonode_status_buckets),
        _ => continue,
      };
      *total += 1;
      if let Some(status) = entry.current_status.as_deref() {
        *buckets
          .entry(public_validator_status(status).to_owned())
          .or_insert(0) += 1;
      }
    }
    agg
  }
}

/// Total/enabled node counts for the live ProTx overlay.
#[derive(Debug, Default, Clone)]
pub struct ProtxSummary {
  pub total_nodes: i64,
  pub enabled_nodes: i64,
}

/// Per-class (regular/evonode) status bucket counts for the live ProTx overlay.
#[derive(Debug, Default, Clone)]
pub struct ProtxAggregate {
  pub regular_total: i64,
  pub regular_status_buckets: HashMap<String, i64>,
  pub evonode_total: i64,
  pub evonode_status_buckets: HashMap<String, i64>,
}

/// Normalize a raw validator type onto the public `regular`/`evonode` classes.
fn public_validator_type(value: Option<&str>) -> &'static str {
  let compact = value
    .unwrap_or_default()
    .chars()
    .filter(|ch| ch.is_ascii_alphanumeric())
    .flat_map(char::to_lowercase)
    .collect::<String>();
  match compact.as_str() {
    "regular" => "regular",
    "highperformance" | "evo" | "evonode" => "evonode",
    _ => "unknown",
  }
}

/// Normalize a raw ProTx status onto the public status labels.
fn public_validator_status(status: &str) -> &'static str {
  match status {
    "valid" => "valid",
    "registered" => "registered",
    "revoked" => "revoked",
    "pose_banned" => "banned",
    _ => "unknown",
  }
}

/// The mempool summary (`getmempoolinfo`).
#[derive(Debug, Clone)]
pub struct MempoolSummary {
  pub tx_count: i64,
  pub total_bytes: i64,
  pub usage_bytes: Option<i64>,
  pub max_bytes: Option<i64>,
  pub updated_at: i64,
}

/// One recent mempool transaction, normalized for the live endpoints (raw bytes
/// plus the decoded detail + resolved input provenance + output addresses).
#[derive(Debug, Clone)]
pub struct MempoolTx {
  pub txid: Vec<u8>,
  pub raw_tx: Vec<u8>,
  pub version: i16,
  pub tx_type: String,
  /// Wide enough for the full u32 nLockTime range.
  pub lock_time: i64,
  pub size: i32,
  pub value_out: i64,
  pub input_count: i32,
  pub output_count: i32,
  pub fee: Option<i64>,
  pub payload: Option<Value>,
  pub inputs: Vec<MempoolTxInput>,
  pub outputs: Vec<MempoolTxOutput>,
  pub first_seen_at: i64,
  pub last_seen_at: i64,
}

impl MempoolTx {
  /// Distinct addresses this tx touches (input provenance + output scripts).
  fn addresses(&self) -> HashSet<String> {
    self
      .inputs
      .iter()
      .filter_map(|input| input.prev_address.clone())
      .chain(
        self
          .outputs
          .iter()
          .filter_map(|output| output.address.clone()),
      )
      .collect()
  }
}

/// One resolved mempool transaction input.
#[derive(Debug, Clone)]
pub struct MempoolTxInput {
  pub vin_index: i32,
  pub sequence: i64,
  pub prev_txid: Option<Vec<u8>>,
  pub prev_vout: Option<i32>,
  pub coinbase: Option<Vec<u8>>,
  pub script_sig: Vec<u8>,
  pub prev_address: Option<String>,
  pub prev_value: Option<i64>,
}

/// One mempool transaction output.
#[derive(Debug, Clone)]
pub struct MempoolTxOutput {
  pub vout_index: i32,
  pub value: i64,
  pub script_pubkey: Vec<u8>,
  pub address: Option<String>,
}

/// The recent mempool set: txs by txid, with an address → txids secondary
/// index kept in lockstep so an address lookup avoids scanning every tx.
#[derive(Debug, Default)]
struct MempoolTxs {
  txs: HashMap<Vec<u8>, Arc<MempoolTx>>,
  by_address: HashMap<String, HashSet<Vec<u8>>>,
}

impl MempoolTxs {
  fn insert(&mut self, tx: MempoolTx) {
    let txid = tx.txid.clone();
    // Drop any prior index entries for this txid before reinserting.
    self.remove(&txid);
    for address in tx.addresses() {
      self
        .by_address
        .entry(address)
        .or_default()
        .insert(txid.clone());
    }
    self.txs.insert(txid, Arc::new(tx));
  }

  fn remove(&mut self, txid: &[u8]) {
    let Some(tx) = self.txs.remove(txid) else {
      return;
    };
    for address in tx.addresses() {
      if let Some(set) = self.by_address.get_mut(&address) {
        set.remove(txid);
        if set.is_empty() {
          self.by_address.remove(&address);
        }
      }
    }
  }
}

/// One live governance object (proposal), normalized as reported by the node.
#[derive(Debug, Clone)]
pub struct GovObject {
  pub object_hash: Vec<u8>,
  pub parent_hash: Option<Vec<u8>>,
  pub revision: Option<i32>,
  pub object_time: Option<i64>,
  pub object_type: i32,
  pub data_json: Option<Value>,
  pub raw_object: Vec<u8>,
  pub updated_at: i64,
}

/// One live governance vote for an object.
#[derive(Debug, Clone)]
pub struct GovVote {
  pub vote_hash: Vec<u8>,
  pub object_hash: Vec<u8>,
  pub signal: Option<i32>,
  pub outcome: Option<i32>,
  pub vote_time: Option<i64>,
  pub raw_vote: Vec<u8>,
  pub updated_at: i64,
}

/// Distinct regular/evonode voter counts for one governance object.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GovVoteTypeCounts {
  pub voted_regular_count: i64,
  pub voted_evo_count: i64,
}

/// The live governance set: objects by hash, with each object's votes kept in a
/// side map keyed by object hash so a targeted refresh replaces one list.
#[derive(Debug, Default)]
struct Governance {
  objects: HashMap<Vec<u8>, Arc<GovObject>>,
  votes: HashMap<Vec<u8>, Vec<Arc<GovVote>>>,
}

impl Governance {
  fn insert_object(&mut self, object: GovObject) {
    self
      .objects
      .insert(object.object_hash.clone(), Arc::new(object));
  }

  fn replace_all_votes(&mut self, votes: Vec<GovVote>) {
    self.votes.clear();
    for vote in votes {
      self
        .votes
        .entry(vote.object_hash.clone())
        .or_default()
        .push(Arc::new(vote));
    }
  }
}

/// Sort governance objects newest-first by `(updated_at DESC, object_hash
/// ASC)`.
fn sort_governance_objects(objects: &mut [Arc<GovObject>]) {
  objects.sort_by(|a, b| {
    b.updated_at
      .cmp(&a.updated_at)
      .then_with(|| a.object_hash.cmp(&b.object_hash))
  });
}

/// Sort governance votes newest-first by `(updated_at DESC, vote_hash ASC)`.
fn sort_governance_votes(votes: &mut [Arc<GovVote>]) {
  votes.sort_by(|a, b| {
    b.updated_at
      .cmp(&a.updated_at)
      .then_with(|| a.vote_hash.cmp(&b.vote_hash))
  });
}

/// Keyset predicate matching the objects SQL: strictly older, or same
/// `updated_at` with a lexically larger hash.
fn governance_object_after(object: &GovObject, after: Option<(i64, &[u8])>) -> bool {
  match after {
    Some((updated_at, object_hash)) => {
      object.updated_at < updated_at
        || (object.updated_at == updated_at && object.object_hash.as_slice() > object_hash)
    }
    None => true,
  }
}

/// Keyset predicate matching the votes SQL: strictly older, or same
/// `updated_at` with a lexically larger vote hash.
fn governance_vote_after(vote: &GovVote, after: Option<(i64, &[u8])>) -> bool {
  match after {
    Some((updated_at, vote_hash)) => {
      vote.updated_at < updated_at
        || (vote.updated_at == updated_at && vote.vote_hash.as_slice() > vote_hash)
    }
    None => true,
  }
}

/// Match an object by hash-prefix (hex) or free-text needle over its stored
/// JSON/raw payload — the RAM analogue of the search SQL's `LIKE`/`ILIKE`.
fn governance_object_matches(object: &GovObject, hash_prefix: &str, text_needle: &str) -> bool {
  if hex::encode(&object.object_hash).starts_with(hash_prefix) {
    return true;
  }
  let haystack = match &object.data_json {
    Some(value) => value.to_string(),
    None => String::from_utf8_lossy(&object.raw_object).into_owned(),
  };
  haystack.to_ascii_lowercase().contains(text_needle)
}

/// Extract the voter's pro_tx hash hex from a raw vote string: the first
/// `':'`-field, before any `'-'`, lowercased. Mirrors the vote_type SQL.
fn vote_pro_tx_hash_hex(raw_vote: &[u8]) -> Option<String> {
  let text = std::str::from_utf8(raw_vote).ok()?;
  let first_field = text.split(':').next()?;
  let hash = first_field.split('-').next()?;
  if hash.is_empty() {
    return None;
  }
  Some(hash.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
  use super::*;

  #[test]
  fn singleton_cell_starts_empty_then_holds_latest() {
    let live = LiveState::new();
    assert!(live.chainlock().is_none());
    live.set_chainlock(ChainLockSnapshot {
      block_hash: vec![1, 2, 3],
      block_height: Some(42),
      core_height: Some(42),
      raw_chainlock: vec![9],
      seen_at: 1000,
    });
    assert_eq!(live.chainlock().map(|c| c.block_height), Some(Some(42)));
    // A second write replaces the value.
    live.set_chainlock(ChainLockSnapshot {
      block_hash: vec![4],
      block_height: Some(43),
      core_height: Some(43),
      raw_chainlock: vec![8],
      seen_at: 1001,
    });
    assert_eq!(live.chainlock().map(|c| c.block_height), Some(Some(43)));
  }
}
