# Mainnet sync log

Live supervision of a from-scratch mainnet sync on branch `sqlite-fixes`
(debug build), against a local fully-synced Dash Core node.

## Run parameters

| | |
|---|---|
| Branch | `sqlite-fixes` @ `922ec90` |
| Build | `cargo build` (debug — unwinding panics, overflow checks on) |
| Node | dash-qt mainnet, height 2,524,639, unpruned |
| Postgres | 18.3 Homebrew, `/Volumes/Samsung/pgdata`, unix socket `/tmp` |
| Database | `hyphen_mainnet` |
| Datadir | `/Volumes/Storage/Library/Hyphen/mainnet` |
| Host | 14 cores, 48 GiB RAM |
| Layers | all eleven — mempool, islock, chainlock, governance, network, stream, protx and chainlock overlays, address, validator and metrics indices |
| Run 1 | 20:59:26 UTC, stopped at ~15k to hotfix finding 1 |
| Run 2 | 21:08:04 UTC, current |

Postgres was purged first: all twelve pre-existing `hyphen_*` databases were
dropped and `hyphen_mainnet` recreated empty. It was dropped and recreated
again for run 2, so nothing below is inherited.

## Startup

Clean both runs. No warnings, no errors. Several of these exercise code the
review pass changed.

- The unix-socket path connected first attempt, and the endpoint printed as
  `unix:/tmp` rather than a URL — the redaction fix holds on a form with no
  password to redact.
- `init_params` was written once, at first run, with all seven keys and
  `schema_version=1`. The collapsed single-migration schema applies to an
  empty database without complaint.
- Server settings were read back and logged rather than assumed:
  `shared_buffers=8GB`, `effective_cache_size=24GB`, `work_mem=64MB`,
  `max_connections=200` against `pool_total=60`.
- All four pools opened with their intended session GUCs, each distinct.

One config refusal fired before startup and was correct. `stepped_sync_gap`
was initially set equal to `partition_span` (100,000), and the engine refused
with an explanation of why a gap spanning a whole partition has no close to
build at. Lowered to 50,000 and it started.

## Findings

### 1. Stepped sync could never engage during an initial sync — HIGH — FIXED

Found on run 1, fixed in `fb12d8d`, verified on run 2.

The branch's headline initial-sync optimization was unreachable in the one
scenario it was built for, and no configuration could reach it.

**What happened.** The state machine decided from a gap it derived from the
hash it was asked to connect toward:

```
pkgs/db/src/ingest/pipeline.rs   node_height = chain::ser_height(pool, &node_tip)
pkgs/db/src/stepped.rs:487       gap = node_tip - tip
```

But a cold sync feeds the chain in bounded chunks, and the daemon handed the
engine that *chunk's* tip:

```
pkgs/daemon/src/ingest/mod.rs:41   const ABSORB_CHUNK_BLOCKS: u32 = 512;
```

So the observed gap was bounded by the chunk and structurally incapable of
reaching `stepped_sync_gap`. Measured live, 2.5M blocks behind:

```
seen_tip | connected_tip | gap
   14335 |         13842 | 493
```

493, against an entry gap of 50,000. The daemon reported `syncMode: realtime`
while 2.5M blocks behind, and at startup the engine *built* partition 0's
seven deferred indexes instead of deferring them.

**Why no configuration fixed it.** Any entry gap below 512 would also engage
during ordinary tip-following, which is exactly what the hysteresis and the
`stepped_sync_gap` < `partition_span` refusal exist to prevent. There was no
value both above the tip-following noise and below the absorb chunk.

**The fix.** `on_tip` now takes a `HyDbTipTarget` rather than a bare hash,
which makes the caller state whether that hash is the node's own tip
(`AtNode`) or the end of a chunk with a backlog behind it (`Chunk { tip,
network_height }`). There is no default correct for both, so the distinction
is carried in the type rather than inferred. The daemon passes the node
height it already polls. `AtNode` still derives the height from the hash,
which is right there because the two genuinely coincide.

An `Option<HyDbHeight>` with a fallback was considered and rejected: a silent
default is what hid this in the first place.

**Verified on run 2:**

```
sync mode=stepped: deferring the open partition's query-only indexes
(gap 2523616 blocks to the node tip)
tip=1023 node_tip=2524639 gap=2523616 open_partition=0
```

All seven deferred indexes dropped for the open partition; `pg_class` shows
zero `idx_%_p0` present; `/api/v1/status` reports `syncMode: stepped`.

### 2. Every restart rebuilds the open partition's indexes — MEDIUM

No correctness impact. Raised from LOW once it was measured on a populated
partition rather than an empty one.

The engine reconciles the open partition's indexes *before* the first advance
has told it which mode it is in, so it builds all seven, and the first
stepped transition drops them again. On a fresh database that is free,
because the partition is empty:

```
21:08:10  stepped sync built deferred index idx_transactions_txid_p0 ...
21:08:14  sync mode=stepped: deferring the open partition's ...
21:08:14  stepped sync deferred index idx_transactions_txid_p0: dropped
```

It is not free on a restart part-way through. Run 3 resumed at height
642,047 with partition 6 open and populated, and the same sequence cost
**14.0 seconds** of index building that was discarded 37 seconds later:

```
22:08:29  built idx_transactions_txid_p6              0.28s
22:08:29  built idx_transactions_list_p6              0.41s
22:08:31  built idx_transactions_display_txid_p6      1.43s
22:08:32  built idx_tx_outputs_txid_vout_p6           1.58s
22:08:34  built idx_tx_outputs_address_p6             1.75s
22:08:35  built idx_tx_outputs_outpoint_p6            1.56s
22:08:42  built idx_address_txs_addr_height_desc_p6   6.99s
                                                     ------
                                                     14.00s
22:09:19  sync mode=stepped ... open_partition=6
22:09:19  all seven dropped for the open partition
```

A third restart, after the storage dropout in finding 5, confirmed what the
cost tracks. Partition 9 had opened only 5,740 blocks earlier, and the same
seven builds cost **10.2 s** — less than partition 6's 14.0 s despite the
chain being denser there, because the table held a fraction of the rows:

```
partition 6, 42,047 blocks in (42% full)   14.0 s
partition 9,  5,740 blocks in ( 6% full)   10.2 s
```

So the bill is proportional to rows already in the open partition, not to
height. Two consequences. A restart just after a partition boundary is
cheap, which is why this one was; and a restart just *before* one, in the
dense back half of the chain, is the worst case rather than the 14 s
measured here. It is paid on every restart, which is exactly when an
operator is already waiting. Folding the first reconcile into the first
observation, rather than running it ahead of one, removes it entirely.

### 3. Partition close builds the deferred indexes as designed — WORKING

Recorded because it is the half of stepped sync that had never run. Partition
5 closed during run 2 and built all seven, `closed=true`:

```
idx_transactions_txid_p5             0.30s
idx_transactions_list_p5             0.43s
idx_transactions_display_txid_p5     1.85s
idx_tx_outputs_txid_vout_p5          1.99s
idx_tx_outputs_address_p5            1.75s
idx_tx_outputs_outpoint_p5           2.19s
idx_address_txs_addr_height_desc_p5  8.65s
```

The mode transition also drove the coupled tuning, which is the `Auto`
profile doing what it claims:

```
applied [db.ingest] storage tuning table="utxo_set"
  ALTER TABLE utxo_set SET (autovacuum_vacuum_scale_factor = 0.2,
                            autovacuum_analyze_scale_factor = 0.05)
```

### 2b. The reconcile cost ten minutes, not fourteen seconds — FIXED

Finding 2 was rated MEDIUM on a 14.0 s measurement. That estimate was wrong,
and the mechanism was worse than "churn".

A restart at height 983,432, with partition 9 some 83,000 blocks full, spent
**over ten minutes** rebuilding the open partition's deferred indexes with
ingest stalled behind them, and had still not finished the seventh:

```
idx_transactions_txid_p9           9.90s
idx_transactions_list_p9           8.51s
idx_transactions_display_txid_p9  26.83s
... three more, then address_txs still running at the ten-minute mark
```

Every second of it was waste: the first real observation dropped all seven.

**Root cause.** `ensure_settled` read the mode straight out of the state
machine, but before the first observation that value is only its *default* —
and the default is `Realtime`, the one mode that wants those indexes present.

**Fixed in `39fa046`** by tracking whether the mode reflects a measurement and
leaving the open partition alone until it does. Closed partitions still
reconcile, because their set does not depend on the mode and building what
they are missing is how a daemon killed mid-build heals itself.

There is a second trap in the same code, guarded by the same commit: skipping
the open partition while still recording `settled = (partition, mode)`
short-circuits every later call, so the open partition is never reconciled and
the deferred set stays in place for good. That would turn a slow restart into
a sync that never defers again.

**Verified live.** The next restart logged zero deferred-index builds, went
straight from `compaction complete compacted=0 deferred=5` into ingesting, and
on the following mode transition dropped all seven and left zero behind.

### 3b. Autovacuum on untuned tables was the throughput collapse — FIXED

Throughput fell from 92 blocks/s to 1.1 blocks/s by height 969,000. It was
not block density, not the USB link's ceiling, and not a cache cliff — table
cache-hit ratios were 98.8% and 99.5%.

The sync profile's autovacuum relaxation reached exactly one of the five
hottest tables. `TUNED_TABLES` has two entries and only `utxo_set` had
`vacuum_tuned: true`:

| table | reloptions in force |
|---|---|
| `utxo_set` | fillfactor, scale 0.2, analyze 0.05, cost limit, cost delay |
| `address_stats` | `fillfactor=80` only |
| `address_txs_p9` | **(none)** |
| `transactions_p9` | **(none)** |
| `tx_outputs_p9` | **(none)** |

Every per-height partition — the tables that churn hardest during exactly the
cold sync the profile exists for — ran on cluster defaults. Caught in the act:
`autovacuum: ANALYZE public.address_txs_p9` waiting on `IO/AioIoCompletion`
while the ingest `INSERT` waited on `IO/DataFileWrite` behind it. Each
partition had taken 9 vacuums and 16 analyzes.

The partitions are **append-only — measured 0.0% dead tuples** — so every one
of those vacuums reclaimed nothing.

Relaxing analyze took the ingest from **2.66 to 11.87 blocks/s, 4.46x**,
sustained over the following twenty minutes.

**Fixed in `2a4ac54`**, as a deliberate trade of disk for throughput:

- Height partitions are created with autovacuum off, and the debt is settled
  by `tuning::compact` once a partition stops growing.
- The debt is *derived from `pg_class.reloptions`* rather than a ledger. A
  table with autovacuum disabled is by construction one the engine switched
  off and has not compacted, so a daemon killed mid-sync — or one whose
  storage device leaves the bus, which happened here — comes back knowing
  what it owes with nothing to reconcile.
- The pass runs at startup before anything is served, with
  `--skip-compacting` for an operator who needs the node up now. Skipping
  costs bloat and stale statistics, never correctness, so a failed pass warns
  and continues rather than holding the node down.
- `SYNC_ANALYZE_SCALE_FACTOR` went 0.05 to 0.5. It had been *more* aggressive
  than the server default, on the reasoning that suppressed statistics cause a
  post-sync plan cliff. The reasoning was right and the number was wrong:
  analyze fires on growth, so it ran hardest while a partition filled. The
  cliff is now answered by the compaction pass, which takes statistics once,
  off the write path, instead of twenty times on it.

One change was made and then reverted. `address_stats` was switched to
`vacuum_tuned: true` and three tests failed; rechecking the data showed the
tests were right. It had taken 1 autovacuum and 5 analyzes all run, so it was
never the cost — its slowness is upsert and bloat, not autovacuum passes. The
original note about HOT updates needing vacuum to keep running stands.

### 3c. The compaction step, measured — WORKING

The trade in finding 3b, priced on real data. First restart with genuine debt
settled partition 9's five tables and correctly held back partition 10's five
as the open one:

```
settling the vacuum a sync deferred before serving  tables=10
  address_txs_p9    31.6s
  tx_inputs_p9      26.1s
  spends_p9         23.8s
  tx_outputs_p9     16.7s
  transactions_p9    9.2s
                   ------
                   107.0s
compaction complete compacted=5 deferred=5
```

So the bill is roughly **1.8 minutes of one-time startup cost per 100,000
blocks**, against the 4.46x throughput it buys during the sync. Paid once with
nothing competing for the device, rather than continuously against the ingest
that created it. Autovacuum is handed back per table as each completes, so a
compacted table is under normal management again immediately instead of
staying deferred until the whole pass ends.

### 7. An index build was indistinguishable from a stall — FIXED

A deferred build runs on the write path deliberately: the writer a
`CONCURRENTLY` build would avoid blocking is the one waiting for it. So while
one runs the projection stops, and because derivation is atomic per block,
*every* layer's rate reads zero at once. The console had no surface for it, and
the watchdog called it `Stalled` — which sends an operator hunting for a fault
that is a scheduled index build.

Worth stating plainly, because it looks like a display bug and is not: the six
layers are **not** independent and their identical rates are truthful. The
daemon says so where it fills them in — "every stage is lock-step at the
canonical tip: derivation is atomic per block in the storage engine". There are
no per-layer heights to decouple. What is asynchronous is the index build.

Fixed in `922ec90`: `SteppedSync` publishes the build in flight on a watch
channel, the heartbeat carries it beside the state it explains, and the console
renders it in the chain-data row and the indices panel title, outranking
`Stalled`. The run is collected before the first build starts, so a long wait
reads as `3/7` rather than an unbounded "still building".

### 4. Slow-statement warnings on the write path — OBSERVING

Not yet a finding. The 500 ms `slow_query_ms` threshold fires on ingest
writes as block weight grows, which is the threshold doing its job rather
than a fault:

```
INSERT INTO utxo_set ...       rows=101   643 ms
INSERT INTO utxo_set ...       rows=2143  605 ms
INSERT INTO tx_inputs ...      rows=26    595 ms
INSERT INTO address_stats ...  rows=229  1676 ms
```

Two things make this worth watching rather than dismissing. The 101-row and
26-row cases took as long as the 2143-row one, so the cost is not row count —
it is more likely lock or I/O wait. And `address_stats` is an upsert with
per-address contention, which is the one that would degrade worst as the
address set grows. Revisit once the sync is deep enough for a trend.

### 5. Storage device dropped out under load — HARDWARE — RECOVERED

Not a code defect, and recorded because the daemon's behaviour under it is
the only real test the failure path has had.

At 23:22 UTC, height 905,612, the external SSD holding `pgdata` dropped off
the bus mid-write. Postgres took an unreadable-block error and died:

```
ERROR hyphend::ingest: ingest cycle failed permanently; pausing ingestion
  until an operator intervenes
  error=storage error (advance to tip): postgres error: connect:
  upsert address_stats: could not read blocks 75760..75760 in file
  "base/2315177/2315346": Device not configured
```

**The daemon did exactly the right thing.** `failure_action` classified it as
`Outcome::Error`, which parks ingestion for an operator rather than retrying
into a dead disk or taking the process down, and the REST API stayed up
serving what was already indexed. That is the branch's three-way severity
policy working against a genuine I/O fault rather than a synthetic one.

**Recovery was clean.** WAL replay took 70 s and the end-of-recovery
checkpoint wrote 969,551 buffers (92.5%) in 164 s. The one alarming-looking
line is not corruption:

```
incorrect resource manager data checksum in record at 101/43170D08
```

That is the torn final record from the crash, which is how Postgres finds the
end of the log. Everything before it replayed, and replay actually recovered
128 blocks *past* the height the daemon last reported.

Integrity after recovery, against the node:

```
tip     905,740      blocks 905,741   (contiguous, zero gaps)
txs     7,135,016
size    59 GB
init_params  all seven keys intact, schema_version=1
```

A daemon restart then re-ran the layout and migration-history checks against
the recovered store and both passed, which is a stronger statement than the
row counts: the schema is the one the binary expects and the recorded layout
still matches.

**Cost.** Roughly eight hours idle, because restarting Postgres needs Full
Disk Access on the removable volume and this shell did not have it. Once
granted, the same `pg_ctl start` worked first time — its only failure had
been the PID-file confirmation step, not the server launch.

**Mitigation.** The supervisor now repairs this class of failure itself:
it restarts Postgres if the postmaster is gone, restarts the daemon if that
is gone, and clears a parked ingestion by restarting the daemon, since a
pause deliberately outlives the condition that caused it. A repeat costs
minutes rather than a night. The durable fix is still to move `pgdata` off a
volume that disconnects under sustained write load.

### 6. Prefix search reported every coinbase as non-coinbase — FIXED

Found by probing the search API mid-sync. The same txid answered differently
depending on which path resolved it:

```
full   e0028eb9…662c7  ->  "coinbase": true
prefix q=e002          ->  "coinbase": false
```

`block_height=0` confirms the genesis coinbase, so the prefix path was wrong.
It was a deliberate shortcut that asserted a value it admitted it did not
have:

```rust
// Prefix hits carry no positional detail; coinbase can't be inferred.
coinbase: false,
```

Every block has exactly one coinbase, so the prefix path misreported a real
fraction of its hits. Fixed in `83e000b`: `search_prefix` now carries
`block_pos` — already present in the table it was scanning — and the REST
layer derives `coinbase: block_pos == 0` exactly as the exact-match path does.

Three reported search failures were investigated and are **not** defects:
hex prefixes of 6 to 63 characters, ambiguous base58/hex terms, and
out-of-range heights all returned 503. One shared cause — the query pool's 5 s
`statement_timeout` firing on a prefix scan whose index stepped sync had
deliberately dropped. Out-of-range heights are the same cause, not a separate
numeric bug: every decimal string is also valid hex, so the term takes the hex
arm too. The one fair criticism is that `503 … retry shortly` is misleading
when the index will not exist again for hours.

An empty `validators` table was also reported as an untested gap. It is
correct: DIP3 masternodes do not activate on mainnet until height 1,028,160.

## Sync progress

| Time (UTC) | Height | Notes |
|---|---|---|
| 21:08:04 | 0 | run 2 started |
| 21:08:14 | 1,023 | stepped mode engaged, gap 2,523,616 |
| 21:09:30 | 4,114 | |
| 21:10:20 | 9,810 | 25,939 txs, database 90 MB |
| 22:07 | 642,047 | ~92 blk/s; killed by the harness, resumed clean |
| 23:22 | 905,612 | storage device dropped out (finding 5) |
| 12:43 | 905,740 | recovered; WAL replay found 128 blocks more than reported |
| 07:58 | 969,000 | throughput collapsed to 1.1 blk/s (finding 3b) |
| 08:00 | 969,298 | autovacuum relaxed: 2.66 -> 11.87 blk/s |
| 08:21 | 983,652 | reconcile fix in; zero wasted index builds on restart |
| 08:25 | 986,329 | 12.4M txs, database 80.3 GB, stepped |
| 08:45 | 1,000,328 | partition 10 created with vacuum deferred |
| 08:59 | 1,009,793 | first real compaction pass: 5 settled, 5 deferred, 107s |
| 09:05 | 1,010,938 | 12.9M txs, database 89.1 GB |

Rates before roughly 100k are not meaningful — those blocks are nearly empty.

Throughput history, with causes rather than a single average:

| phase | rate | limited by |
|---|---|---|
| 0 to 642k | ~92 blk/s | mostly small blocks |
| to 969k | fell to 1.1 blk/s | autovacuum on untuned partitions |
| after the relax | 11.9 blk/s | device-bound random writes over USB |

The remaining ceiling is the store's transport. The volume is USB-attached and
measured ~84 MB/s at ~3,200 tps, which is small-random-write bound; that is
the figure the ingest now runs against, and it does not move without moving
`pgdata`. Disk headroom is not a concern — 855 GiB free against 80 GB used at
39% of the chain.
