# Mainnet sync log

Live supervision of a from-scratch mainnet sync on branch `sqlite-fixes`
(debug build), against a local fully-synced Dash Core node.

## Run parameters

| | |
|---|---|
| Branch | `sqlite-fixes` @ `fb12d8d` |
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

### 2. Every restart builds the open partition's indexes, then drops them — MEDIUM

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

That cost scales with how full the open partition is, so a restart late in a
sync pays more than this, not less — and it is paid on every restart, which
is exactly when an operator is already waiting. Folding the first reconcile
into the first observation, rather than running it ahead of one, removes it.

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

## Sync progress

| Time (UTC) | Height | Notes |
|---|---|---|
| 21:08:04 | 0 | run 2 started |
| 21:08:14 | 1,023 | stepped mode engaged, gap 2,523,616 |
| 21:09:30 | 4,114 | |
| 21:10:20 | 9,810 | 25,939 txs, database 90 MB |

Rates this early are not meaningful — blocks before roughly 100k are nearly
empty. Real figures start once block weight picks up.
