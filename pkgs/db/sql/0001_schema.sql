--
-- Copyright (c) 2026-present, The Dash Core developers
-- SPDX-License-Identifier: MIT
-- See the accompanying file LICENSE or https://opensource.org/license/MIT
--

-- hyphen-db schema — PostgreSQL 18.
--
-- Chain-derivable data only. Every row here is rebuildable from `ser_blocks`
-- plus the caller-supplied deriver; nothing that is application state or live
-- node state lives in this database.
--
-- This is the whole schema, in one file, and it is the only migration. sqlx
-- applies it once and records it in `_sqlx_migrations`. Do not add
-- IF NOT EXISTS guards — it runs exactly once, against a database that has
-- never held Hyphen state.
--
-- It states the END STATE, deliberately. Every deployment is a fresh sync, so
-- there is no database for an incremental step to migrate and no reader served
-- by replaying a history of additions and removals.
--
-- Where an object is deliberately absent it simply does not appear below, and a
-- comment says why. Two of those absences are load-bearing
-- and must not be "fixed" by adding the obvious index back:
--   * the seven deferred indexes have no partitioned parent (see the deferred
--     index set below);
--   * `address_txs` has no primary key (see that table).
--
-- Layout follows the five-layer model, narrowed to layers 1-3 plus an
-- internal undo family:
--   L1 base facts   — append-only, written once in block order
--   L2 accumulations — precomputed answers, no query-time aggregation
--   L3 indexes      — base facts reorganized for relationship queries
--   internal        — bounded shallow-fork undo, never exposed through a trait

-- ---------------------------------------------------------------------------
-- Enum types
-- ---------------------------------------------------------------------------
--
-- Hashes (block hash, txid, protx_hash) and packed outpoints are stored as
-- native `bytea` — 32/36 bytes on disk, not the 64/72 a hex text column would
-- cost. Their fixed lengths are guaranteed by the crate's newtypes
-- (`HyDbHash([u8; 32])`, `HyDbOutpoint`), so no DB-level DOMAIN check is needed
-- and every write binds `bytea` directly with no cast.

-- ---------------------------------------------------------------------------
-- Display-order hex
-- ---------------------------------------------------------------------------
--
-- THE SEARCH CONTRACT: user-facing hash identifiers (block hashes, txids,
-- protx hashes) are DISPLAY-order hex — the byte-reverse of the internal
-- little-endian bytes stored in `bytea` columns, i.e. exactly what explorer
-- UIs render and what users copy. Every prefix-search predicate matches
-- `display_hex(col)`, every prefix index is an expression index over it, and
-- the REST layer passes the user's prefix through lowercased. Internal-order
-- hex never crosses the API boundary.
CREATE FUNCTION display_hex(h bytea) RETURNS text
LANGUAGE sql IMMUTABLE STRICT PARALLEL SAFE AS $$
  SELECT COALESCE(string_agg(lpad(to_hex(get_byte(h, octet_length(h) - 1 - i)), 2, '0'),
                             '' ORDER BY i), '')
    FROM generate_series(0, octet_length(h) - 1) AS i
$$;

CREATE TYPE tx_type AS ENUM (
  'normal',
  'provider_register',
  'provider_update_service',
  'provider_update_registrar',
  'provider_update_revoke',
  'coinbase',
  'quorum_commitment',
  'asset_lock',
  'asset_unlock',
  'mnhf'
);

CREATE TYPE address_type AS ENUM (
  'p2pkh',
  'p2sh',
  'p2pk',
  'unknown'
);

-- Validator kind, stored already-normalized so filters are a plain equality
-- rather than a per-row regex. Ingest maps the node's raw type onto these.
CREATE TYPE validator_type AS ENUM (
  'regular',
  'evonode',
  'unknown'
);

-- ---------------------------------------------------------------------------
-- The storage-layout contract
-- ---------------------------------------------------------------------------
--
-- `[db.layout]` as it stood when this database was created.
--
-- The rows are the record of what the bytes in this database *mean*. They are
-- written once, on the first startup against a fresh database, and after that
-- a disagreement between them and the configuration refuses startup rather
-- than reinterpreting data already on disk. See `src/layout.rs` for the
-- grades (hard / rebuildable / managed) and their remedies.
--
-- The table has to exist before any block is written, which it does by
-- construction: `hyphen-db` resolves the layout immediately after
-- `MIGRATOR.run` returns, on a table this file has just created.
--
-- Key/value rather than one column per parameter, deliberately: a newer daemon
-- must be able to record a key an older one has never heard of, and an older
-- daemon must be able to read the table and ignore that key, without a
-- migration on either side.

CREATE TABLE init_params (
  -- Layout key, matching the `[db.layout]` config key exactly.
  key        text        PRIMARY KEY,
  -- Its value, rendered as text. The Rust side owns parsing and validation;
  -- storing text keeps one column type across integers, bytes and names.
  value      text        NOT NULL,
  -- The daemon version that first recorded this key. An operator diagnosing a
  -- refusal needs to know which build's idea of the layout they are looking
  -- at, especially for keys adopted later than the database was created.
  created_by text        NOT NULL,
  -- When it was recorded. Not updated when a managed key advances; the
  -- creation time is the useful fact.
  created_at timestamptz NOT NULL DEFAULT now()
);

COMMENT ON TABLE init_params IS
  'Immutable storage-layout parameters recorded at database creation; compared against [db.layout] on every startup.';

-- ---------------------------------------------------------------------------
-- The deferred index set
-- ---------------------------------------------------------------------------
--
-- Seven query-only indexes on the height-partitioned tables are NOT created
-- on their partitioned parents anywhere in this file. That absence is the
-- feature. Stepped sync defers these indexes on the partition that
-- is currently being filled and builds them in one pass when that partition
-- closes. Building an index costs O(table), not O(delta), so paying it once
-- per partition (~25 times over the chain) beats paying random-write index
-- maintenance on every one of ~3 GB of index per partition.
--
-- Why there can be no parent index
-- --------------------------------
-- Postgres will not let a partition of a partitioned index be dropped:
--
--   ERROR:  cannot drop index tx_outputs_p9_outpoint_idx
--           because index idx_tx_outputs_outpoint requires it
--
-- and it re-creates the child unconditionally whenever a partition is added,
-- whether by CREATE TABLE ... PARTITION OF or by ALTER TABLE ... ATTACH
-- PARTITION, and whether the parent index is valid or (having been made with
-- ON ONLY) invalid. All four combinations were measured on PostgreSQL 18.3;
-- there is no state in which an attached partition can lack a child of an
-- existing parent index. So "the open partition has no such index" is only
-- reachable if the index has no partitioned parent at all. Declaring any of
-- the seven below on the parent would silently disable stepped sync.
--
-- That costs nothing at query time. Postgres has no global indexes: a
-- partitioned index is a metadata umbrella that guarantees presence and
-- backs partition-spanning uniqueness, and plans are per-partition either
-- way. Measured on the same server, a parent-level query with p0/p1 indexed
-- and p2 not produces an Append of `Index Scan using w_p0_v_idx`,
-- `Index Scan using w_p1_v_idx`, and a fallback scan on p2 — identical
-- whether the umbrella index is absent, present-and-invalid, or valid. Only
-- the partition that actually lacks the index degrades; the rest of the table
-- is untouched.
--
-- None of the seven is read by the block-connect path, which resolves
-- prevouts through utxo_set's primary key and otherwise only inserts into
-- these tables. idx_tx_outputs_outpoint is read on *disconnect* (the UTXO
-- restore join), which is why stepped mode drops back to realtime before
-- servicing a reorg. The indexes the ingest path does read —
-- address_stats_pkey, the utxo_set keys, every partitioned table's own
-- primary key, and idx_address_txs_block_height — are deliberately not in
-- this list and are created on their parents in the normal way below.
--
-- The daemon creates each of these per partition at close, naming it with the
-- stem plus the partition's own `_pN` suffix
-- (`idx_tx_outputs_outpoint_p9`, and so on).
--
-- DEFERRED INDEX SET — mirrored by `stepped.rs::DEFERRED_INDEXES`, and pinned
-- to this block by `deferred_set_matches_the_migration`. The two are one list
-- written twice; if they drift, a daemon-built database and this schema carry
-- different indexes under the same names. Definitions are quoted exactly as
-- the Rust constant spells them.
--
--   'idx_transactions_txid'            on 'transactions'
--     '(txid)'
--     Lookup a transaction by txid alone (the PK leads with block_height).
--
--   'idx_transactions_list'            on 'transactions'
--     '(block_height DESC, block_pos DESC, txid DESC)'
--     Newest-first keyset pagination of the global transaction list.
--
--   'idx_transactions_display_txid'    on 'transactions'
--     '(display_hex(txid) text_pattern_ops)'
--     Prefix search on the display-order hex txid (see the search contract
--     above).
--
--   'idx_tx_outputs_txid_vout'         on 'tx_outputs'
--     '(txid, vout_index)'
--     A transaction's outputs, and input resolution by consumed (txid, vout).
--
--   'idx_tx_outputs_address'           on 'tx_outputs'
--     '(address) WHERE address IS NOT NULL'
--     An address's outputs.
--
--   'idx_tx_outputs_outpoint'          on 'tx_outputs'
--     '(outpoint)'
--     The generated packed key exists to be joined on (spent-status lookups
--     and the UTXO-restore step of every disconnect); without this index
--     those joins full-scan every partition.
--
--   'idx_address_txs_addr_height_desc' on 'address_txs'   [UNIQUE]
--     '(address, block_height DESC, block_pos DESC, txid DESC)'
--     The address paginator's ordering, and the only uniqueness constraint
--     `address_txs` has. Built with CREATE UNIQUE INDEX at partition close —
--     see the `address_txs` commentary for what that does and does not prove.
--
-- END DEFERRED INDEX SET

-- ===========================================================================
-- L1 — Base facts (append-only)
-- ===========================================================================

-- Every block ever seen, canonical or orphan, stored as its exact bytes.
-- `raw` is large and cold: force it out-of-line so header/topology scans over
-- this table never drag block bodies through the buffer cache.
CREATE TABLE ser_blocks (
  hash       bytea   PRIMARY KEY,
  prev_hash  bytea   NOT NULL,
  height     INTEGER NOT NULL,
  raw        bytea   NOT NULL
);
ALTER TABLE ser_blocks ALTER COLUMN raw SET STORAGE EXTERNAL;

-- Parent link walk (canonicalization) and orphan-by-height lookups.
CREATE INDEX idx_ser_blocks_prev_hash ON ser_blocks (prev_hash);
CREATE INDEX idx_ser_blocks_height    ON ser_blocks (height);

-- Canonical headers: the winning chain selected out of ser_blocks. Decoded,
-- queryable header facts; the body still lives in ser_blocks.
CREATE TABLE blocks (
  height            INTEGER PRIMARY KEY,
  hash              bytea   NOT NULL,
  prev_hash         bytea   NOT NULL,
  merkle_root       bytea   NOT NULL,
  -- Wide enough for any u32 timestamp (a post-2038 nTime must not wrap).
  time              BIGINT  NOT NULL,
  bits              INTEGER NOT NULL,
  nonce             BIGINT  NOT NULL,
  version           INTEGER NOT NULL,
  size              INTEGER NOT NULL,
  tx_count          INTEGER NOT NULL,
  chainwork         TEXT,
  cb_height         INTEGER,
  cb_merkle_root    bytea,
  cb_merkle_flags   INTEGER,
  cb_best_cl_height INTEGER,
  cb_best_cl_sig    bytea,
  cb_credit_pool    BIGINT
);
CREATE UNIQUE INDEX idx_blocks_hash ON blocks (hash);
CREATE INDEX idx_blocks_time ON blocks (time);
-- Prefix search on the display-order hex hash (see the search contract above).
-- text_pattern_ops makes LIKE 'prefix%' indexable under any collation.
CREATE INDEX idx_blocks_display_hash ON blocks (display_hex(hash) text_pattern_ops);

-- Canonical transactions, partitioned by 100k-block ranges. Partitions are
-- created on demand by the ingest pipeline (COPY-loaded, indexed once filled).
CREATE TABLE transactions (
  txid          bytea    NOT NULL,
  block_height  INTEGER  NOT NULL,
  block_pos     INTEGER  NOT NULL,
  version       INTEGER  NOT NULL,
  tx_type       tx_type  NOT NULL DEFAULT 'normal',
  -- Wide enough for the full u32 nLockTime range (values >= 2^31 are legal).
  lock_time     BIGINT   NOT NULL,
  size          INTEGER  NOT NULL,
  value_out     BIGINT   NOT NULL,
  input_count   INTEGER  NOT NULL,
  output_count  INTEGER  NOT NULL,
  fee           BIGINT,
  PRIMARY KEY (block_height, txid)
) PARTITION BY RANGE (block_height);

-- No parent indexes here. `transactions`' three query-only indexes
-- (idx_transactions_txid, idx_transactions_list, idx_transactions_display_txid)
-- are in the deferred set above and exist only per partition, created by the
-- daemon when the partition closes. The primary key stays on the parent: the
-- ingest path reads it.

-- Transaction inputs, partitioned alongside transactions. `prev_address` and
-- `prev_value` are resolved at derive time and stored inline, so reading a
-- transaction's inputs needs no join back to the consumed outputs.
CREATE TABLE tx_inputs (
  block_height  INTEGER NOT NULL,
  txid          bytea   NOT NULL,
  vin_index     INTEGER NOT NULL,
  prev_txid     bytea,
  prev_vout     INTEGER,
  script_sig    bytea   NOT NULL,
  sequence      BIGINT  NOT NULL,
  prev_address  TEXT,
  prev_value    BIGINT,
  PRIMARY KEY (block_height, txid, vin_index)
) PARTITION BY RANGE (block_height);
CREATE INDEX idx_tx_inputs_txid ON tx_inputs (txid);

-- Transaction outputs. `outpoint` is a generated, indexable packed key so the
-- sorted spend-resolution merge and utxo joins can key on a single column.
CREATE TABLE tx_outputs (
  block_height  INTEGER      NOT NULL,
  txid          bytea        NOT NULL,
  vout_index    INTEGER      NOT NULL,
  value         BIGINT       NOT NULL,
  script_pubkey bytea        NOT NULL,
  address       TEXT,
  address_type  address_type,
  outpoint      bytea        GENERATED ALWAYS AS (txid || int4send(vout_index)) STORED,
  PRIMARY KEY (block_height, txid, vout_index)
) PARTITION BY RANGE (block_height);
-- No parent indexes here either: idx_tx_outputs_txid_vout,
-- idx_tx_outputs_address and idx_tx_outputs_outpoint are all in the deferred
-- set above and live per partition only. idx_tx_outputs_outpoint is the one
-- of the three that a *disconnect* reads (the UTXO-restore join), which is
-- why stepped mode returns to realtime before servicing a reorg.

-- Dash special-transaction (DIP2/DIP3) payloads: a base fact keyed by txid,
-- decoded to JSONB, indexed by protx_hash for validator provenance queries.
CREATE TABLE sptx_payloads (
  block_height  INTEGER NOT NULL,
  txid          bytea   NOT NULL,
  tx_type       tx_type NOT NULL,
  protx_hash    bytea,
  payload       JSONB   NOT NULL,
  PRIMARY KEY (block_height, txid)
);
CREATE INDEX idx_sptx_txid ON sptx_payloads (txid);
CREATE INDEX idx_sptx_protx_hash ON sptx_payloads (protx_hash)
  WHERE protx_hash IS NOT NULL;
CREATE INDEX idx_sptx_type ON sptx_payloads (tx_type);

-- ===========================================================================
-- L2 — Accumulations (precomputed answers)
-- ===========================================================================

-- One row per address: the whale-safe balance answer is a single-row read,
-- never a sum over outputs. Written per-block with a pre-aggregated delta.
CREATE TABLE address_stats (
  address            TEXT    PRIMARY KEY,
  balance            BIGINT  NOT NULL DEFAULT 0,
  total_received     BIGINT  NOT NULL DEFAULT 0,
  tx_count           INTEGER NOT NULL DEFAULT 0,
  utxo_count         INTEGER NOT NULL DEFAULT 0,
  first_seen_height  INTEGER,
  last_seen_height   INTEGER
) WITH (fillfactor = 80);
-- The fillfactor is the other half of the no-balance-index decision below:
-- dropping the index makes a HOT update legal, but HOT also needs free space
-- on the tuple's own page, and a page packed to fillfactor 100 has none until
-- a VACUUM has been through. Swept against a replay of real mainnet heights
-- 860,000-869,999 (1,320,908 address updates over 10,000 blocks) on the full
-- 18.9M-row table, measuring the cold pass over a freshly packed table and
-- the steady-state pass after it:
--
--   ff   heap    cold HOT / WAL     steady HOT / WAL
--   100  1826MB   0.41% / 2296MB    45.47% / 338MB
--    90  2026MB  29.24% / 1875MB    55.59% / 305MB
--    80  2276MB  48.82% /  330MB    61.48% / 275MB
--    70  2595MB  63.53% /  272MB    66.67% / 256MB
--
-- 80 is the knee. It takes the cold pass from 0.41% to 48.82% HOT and its WAL
-- from 2296 MB to 330 MB, which is what an initial sync sees since nothing has
-- been vacuumed yet; 70 buys a further 5.2 points of steady-state HOT for
-- another 14% of table size (+319 MB here, and this table is 3 GB on mainnet),
-- which is not worth it. The +25% over fillfactor 100 is the price paid.
-- NOTE: no balance-ordered index lives here, deliberately. A HOT update
-- requires that no indexed column change, and `balance` changes on almost
-- every update of this table, so any index over it makes HOT impossible by
-- construction: measured 12,498 HOT out of 25,687,483 updates (0.05%) on a
-- 967k-block mainnet sync, 1.96M dead tuples and 35 autovacuum passes. The
-- rich list is served from `address_rich` below instead.

-- Address prefix search: LIKE 'prefix%' needs text_pattern_ops to use an
-- index under a non-C collation (the PK's default btree opclass cannot).
CREATE INDEX idx_address_stats_prefix
  ON address_stats (address text_pattern_ops);

-- The rich list, maintained in the block transaction alongside the
-- `address_stats` upsert. Holds only addresses whose balance is at or above
-- the rich-list floor, so the dust majority (99%+ of addresses, and nearly all
-- of the per-block update volume) never touches it and `address_stats` can
-- stay free of a balance-ordered index.
--
-- Membership needs no history scan: an address is only ever written here by a
-- block that already computed its new balance, and the reverse path re-reads
-- it from `address_stats` once that table has been rewound. See `layout::RICH_LIST_FLOOR`; changing
-- the floor requires an offline rebuild of this table (one scan of
-- `address_stats`), not a resync.
CREATE TABLE address_rich (
  address  TEXT   PRIMARY KEY,
  balance  BIGINT NOT NULL
);
CREATE INDEX idx_address_rich_rank ON address_rich (balance DESC, address ASC);
-- No fillfactor here, unlike `address_stats`: the rank index covers `balance`
-- and every update of this table changes it, so no update can ever be HOT and
-- same-page free space would buy nothing. The volume is a rounding error
-- besides — only addresses at or above the rich-list floor are written.

-- The number of addresses with a positive balance, maintained in the block
-- transaction by the same stage that writes `address_stats`, from the same
-- prior/new balance pair the rich-list membership check already computes.
--
-- It is a counter and not a `COUNT(*)` because `address_stats` deliberately
-- carries no index over `balance` (see above): the count was an index-only
-- scan of `idx_address_stats_balance_rank` and became a parallel sequential
-- scan of the whole 2 GB table the moment that index went away, on a read the
-- balances endpoint issues once per request.
--
-- It lives in its own singleton rather than in `supply_summary` because the
-- owning stage differs: `supply_summary` is read-modify-written by the stats
-- stage, which snapshots it into `undo_blocks` for reversal, while this value
-- is maintained by the balance stage from the prior/new balance pair it holds
-- in both directions (see `undo_address_stats`).
-- Folding it into `supply_summary` would put two stages on the same row in the
-- same block transaction and would make the stats stage's undo capture depend
-- on the balance stage having already run.
-- The three singleton summary tables below take no fillfactor either. A
-- one-row table's page is empty regardless of fillfactor, which applies to
-- inserts and not to the free space a HOT update looks for: measured 200,000
-- updates of a singleton at 99.46% HOT, and fillfactor 90 returned the same
-- 99.46%. They are already HOT-updating as well as they can.
CREATE TABLE address_summary (
  singleton              BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  positive_balance_count BIGINT  NOT NULL DEFAULT 0 CHECK (positive_balance_count >= 0)
);
INSERT INTO address_summary (singleton) VALUES (TRUE);

CREATE TABLE transactions_summary (
  singleton  BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  total      BIGINT  NOT NULL DEFAULT 0 CHECK (total >= 0)
);
INSERT INTO transactions_summary (singleton) VALUES (TRUE);

-- Circulating / included supply — the accumulation with no per-address home.
-- The invariant is `total = included_supply + unspendable_value`:
--   total            — true issued supply (Σ subsidy = Σ coinbase − Σ fees)
--   included_supply  — value currently attributable to addressed outputs
--                      (Σ received − sent over address deltas)
--   unspendable_value — value parked in addressless outputs (OP_RETURN,
--                       non-standard scripts), tracked separately so the two
--                       supply views stay consistent instead of silently
--                       diverging on the first addressless output.
CREATE TABLE supply_summary (
  singleton         BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  total             BIGINT  NOT NULL DEFAULT 0 CHECK (total >= 0),
  included_supply   BIGINT  NOT NULL DEFAULT 0 CHECK (included_supply >= 0),
  unspendable_value BIGINT  NOT NULL DEFAULT 0
);
INSERT INTO supply_summary (singleton) VALUES (TRUE);

-- Cumulative per-height chain metrics for charts; served by range + rollup.
CREATE TABLE block_metrics (
  height              INTEGER          PRIMARY KEY,
  tx_count            BIGINT           NOT NULL DEFAULT 0,
  utxos_spendable     BIGINT           NOT NULL DEFAULT 0,
  utxos_unspendable   BIGINT           NOT NULL DEFAULT 0,
  blockchain_size     BIGINT           NOT NULL DEFAULT 0,
  credit_pool_balance BIGINT,
  circulating_supply  BIGINT           NOT NULL DEFAULT 0,
  difficulty          DOUBLE PRECISION NOT NULL DEFAULT 0
);
CREATE INDEX idx_block_metrics_height ON block_metrics (height DESC);

CREATE TABLE validator_counts (
  status          TEXT           NOT NULL DEFAULT '',
  validator_type  validator_type NOT NULL,
  total           BIGINT         NOT NULL DEFAULT 0 CHECK (total >= 0),
  PRIMARY KEY (status, validator_type)
);

-- ===========================================================================
-- L3 — Indexes (relationships for querying)
-- ===========================================================================

-- "Is this output spent, and by whom?" — append-only, keyed by the height the
-- spend happened, never a mutation of the output row. Partitioned by spend
-- height so reorg re-derivation touches a bounded range.
CREATE TABLE spends (
  spend_height   INTEGER NOT NULL,
  spending_txid  bytea   NOT NULL,
  spending_vin   INTEGER NOT NULL,
  outpoint       bytea   NOT NULL,
  PRIMARY KEY (spend_height, spending_txid, spending_vin)
) PARTITION BY RANGE (spend_height);
-- "Was this outpoint spent, and by whom?" — the join for output spent-status.
CREATE INDEX idx_spends_outpoint ON spends (outpoint);

-- The currently-unspent set: gains a row on create, loses one on spend. The
-- one table that churns, so it runs a low fillfactor and aggressive autovacuum.
-- `script_pubkey` is carried here so an address's UTXO listing is a single
-- index scan with no join back to the (partitioned) tx_outputs table.
CREATE TABLE utxo_set (
  outpoint      bytea   PRIMARY KEY,
  address       TEXT,
  value         BIGINT  NOT NULL,
  block_height  INTEGER NOT NULL,
  script_pubkey bytea   NOT NULL
) WITH (
  fillfactor = 70,
  autovacuum_vacuum_scale_factor = 0.02,
  -- Not a throttle to be gentle with, a throttle to be got through. The
  -- global default (cost_limit 200, cost_delay 2ms) paces a vacuum of this
  -- table so slowly that it is never not running: on the 967k-block mainnet
  -- sync `utxo_set` took 23.1M inserts and 90 autovacuum passes, and a worker
  -- was on it in 40 of 40 consecutive pg_stat_activity samples, competing with
  -- ingest continuously. Raising the limit 10x keeps the yield point but lets
  -- each pass finish and get out of the way.
  --
  -- Replayed real mainnet heights 860,000-869,999 against the real UTXO set as
  -- of 859,999 (3,324,916 rows; 826,410 inserts and 702,375 deletes, one
  -- transaction per block, paced at ~33 blocks/s, autovacuum live):
  --
  --   setting                          worker present   passes  heap   catch-up
  --   default cost                     20.5% / 22.9%    10      693MB  1.77s
  --   cost_limit 2000, delay 2ms        11.1% /  9.2%   11      689MB  0.36s
  --   cost_limit 2000, delay 0          16.9%           11      693MB  0.52s
  --   scale_factor 0.2, default cost    15.6%            7      707MB  3.15s
  --   scale_factor 0.1 + limit 2000/0   14.4%            7      705MB  3.27s
  --   autovacuum off                     0.0%            0      811MB  5.04s
  --
  -- Halving the worker's presence costs nothing: ingest wall time was flat
  -- across every variant (the run is paced), and the table came out smaller,
  -- not larger. The other direction loses — vacuuming less often trades a
  -- couple of points of presence for real bloat and a catch-up bill that keeps
  -- growing, and turning it off entirely produced both the largest heap and
  -- the slowest run. So the frequency stays at 0.02; only the pacing changes.
  -- Delay stays at 2ms rather than 0: unthrottled measured worse than this on
  -- every axis, and 0 removes the yield point a shared server wants.
  --
  -- These are static defaults. B1 will override them per sync mode.
  autovacuum_vacuum_cost_limit = 2000,
  autovacuum_vacuum_cost_delay = 2
);
-- Keyset pagination of an address's UTXOs, newest-first. `outpoint` packs
-- txid||vout big-endian, so DESC on it orders by txid then vout, matching the
-- historical (block_height, txid, vout) ordering.
CREATE INDEX idx_utxo_set_address_key
  ON utxo_set (address, block_height DESC, outpoint DESC)
  WHERE address IS NOT NULL;
CREATE INDEX idx_utxo_set_block_height ON utxo_set (block_height);

-- An address's transactions, newest-first, keyset-paginated. Partitioned by
-- height so each page is a bounded index range independent of address size.
--
-- NO PRIMARY KEY, deliberately. This table carried two btrees whose keys are
-- a prefix relationship:
--
--   address_txs_pkey                  (address, block_height, txid)
--   idx_address_txs_addr_height_desc  (address, block_height DESC,
--                                      block_pos DESC, txid DESC)
--
-- The first is a strict key-prefix subset of the second. On the retained
-- mainnet database that redundancy was 8.8 GB of the table's 17.6 GB of
-- index; on the 860,000-block seed, 6,527 MB of 13 GB. Nothing needs it:
--
--   * The read path never used it. The address paginator orders by
--     (block_height DESC, block_pos DESC, txid DESC) within one address, which
--     the pkey cannot serve, and it plans as an Index Only Scan using
--     address_txs_pN_address_block_height_block_pos_txid_idx on every partition
--     of `hyphen_mainnet`.
--   * The write path stopped needing it in A5. The `address_txs` insert used to
--     carry `ON CONFLICT DO NOTHING` with the pkey as its arbiter; the writer's
--     in-memory dedup key was then narrowed to exactly (address, txid) at a
--     fixed height — the constraint's own columns — so the key set handed to
--     Postgres is unique by construction and the clause was removed. With no
--     arbiter there is no arbiter index to back.
--   * The reversal `DELETE ... WHERE block_height = $1` stands on
--     idx_address_txs_block_height below, not on the pkey — neither btree
--     leading with `address` could ever serve it.
--
-- Measured on a clone of the seed (`address_txs_p8`, 8.47M rows, 883 MB heap /
-- 2,321 MB index), 100 blocks of 141 rows each, `pg_statio` deltas:
--
--   layout                     pages/block   physical reads/block cold   ms/block
--   two btrees (before)            1281.5                       285.8      100.1
--   one btree                       713.2                        73.4       56.1
--   one UNIQUE btree                708.8                           —        47.1
--
-- `heap_blks_read` was zero in every phase: this is all index leaf traffic,
-- which is the `IO:DataFileRead` that stood at 73% of active samples.
--
-- Why the survivor is deferred and built UNIQUE, not declared UNIQUE
-- -----------------------------------------------------------------
-- A unique index cannot be deferred — deferring it means not enforcing it — so
-- declaring the wide index UNIQUE here would force it to be maintained on
-- every insert during IBD and lose stepped sync's cheap scheme entirely. It
-- would also need a partitioned parent, which is exactly what the deferred set
-- cannot have. Instead the index is a member of the deferred set above
-- (`stepped.rs::DEFERRED_INDEXES`, `unique: true`) and is built with
-- CREATE UNIQUE INDEX when the partition closes. That build both creates the
-- index and retroactively proves the partition holds no duplicate key: if A5's
-- dedup ever breaks, or a new path bypasses it, the close fails loudly with
-- the offending partition in the index name, instead of corrupting silently.
-- Near the tip, in realtime mode, the index exists and enforcement is per-row
-- again.
--
-- So enforcement granularity is per-row near the tip and per-partition during
-- IBD, and verified either way. The accepted exposure: reads against the still
-- open partition can see a duplicate before that partition closes, and a failed
-- close needs an operator. Both are the right failure mode, because a duplicate
-- here means a real bug in the writer.
--
-- What the unique key does and does not prove
-- -------------------------------------------
-- The key is the four columns the paginator orders by, which is one column
-- wider than the old constraint. `block_pos` is functionally determined by
-- (block_height, txid) — that pair is `transactions`' own primary key, and
-- `address_txs.block_pos` is a copy of the value stored there — so a genuine
-- duplicate (address, block_height, txid) carries the same block_pos and
-- collides. The one shape that would slip through is a duplicate whose two
-- rows disagree about `block_pos`, i.e. a writer that has already corrupted the
-- position it copies; no single index over the paginator's ordering can catch
-- that, because the ordering needs all four columns in that order.
CREATE TABLE address_txs (
  address       TEXT    NOT NULL,
  block_height  INTEGER NOT NULL,
  txid          bytea   NOT NULL,
  block_pos     INTEGER NOT NULL
) PARTITION BY RANGE (block_height);

-- Support the reversal delete. Every disconnect runs one
--
--   DELETE FROM address_txs WHERE block_height = $1
--
-- per reversed block. `address_txs` is the only base table whose reversal has
-- no other index to stand on: both btrees above lead with `address` and
-- neither can be probed by height alone. Range partitioning narrowed the
-- delete to one partition and nothing narrowed it further, leaving a
-- sequential scan of that whole partition.
--
-- Measured on an 860,000-block seed (46.5M address_txs rows, partition p8
-- holding 8.47M of them, 883 MB of heap), parallelism off, in a rolled-back
-- transaction. Warm first, then again on a freshly cloned database so the
-- buffer pool had never seen the data:
--
--                          latency      buffers (hit/read)   rows found
--   before, warm            337-421 ms   112,980 / 0          62-131
--   before, COLD          14,263 ms        2,110 / 110,932        62
--   after, warm heap           81 ms          134 / 3            131
--   after, COLD partition      18 ms           13 / 3             11
--
-- The cold row is the one that matters, and it is why the warm reading was not
-- worth trusting: 110,932 physical block reads, 867 MB pulled through an 8 GB
-- shared_buffers on a 61 GB database, and 14.3 SECONDS for a single block's
-- reversal. Warm understated it by 42x. A reorg arrives while a sync is using
-- that cache for something else, so cold is the normal case, not the bad one —
-- and a multi-block reorg pays it per block.
--
-- The index path does not care either way: 3 physical reads on a partition the
-- buffer pool had never touched, because it reads an index page and the handful
-- of heap pages the matching rows actually live on.
--
-- Cost: 325 MB across the nine partitions of the seed, against 6.5 GB for the
-- primary keys the same table used to carry. Insert-side maintenance is the
-- cheap case for a btree — heights arrive monotonically, so every insert lands
-- on the rightmost leaf.
--
-- Why btree and not BRIN
-- ---------------------
-- BRIN is the tempting answer for a column this correlated, and it works: the
-- same delete measured 0.386 ms against a BRIN index on the same data. It is
-- rejected anyway. At 280 kB for all nine partitions it is a thousand times
-- smaller and builds in 2.4 s, but it is lossy: that run rechecked 6,229 heap
-- rows to find 33, and the recheck set grows as heap correlation decays —
-- precisely what a re-derived partition and reused free space do. The delete is
-- on the reorg path, where predictable is worth more than small, and 325 MB
-- against a 61 GB database (0.5%) is not the constraint. A future BRIN swap is
-- a one-statement change if the size ever matters.
--
-- Why the parent, and not the deferred set
-- ---------------------------------------
-- This index is deliberately NOT deferrable, for the same reason `utxo_set`'s
-- and every primary key are not: the ingest path itself reads it. It is read
-- on disconnect, and while stepped mode drops back to realtime before
-- servicing a reorg, "realtime" is where the index has to already exist. So it
-- keeps its partitioned parent and is created here.
CREATE INDEX idx_address_txs_block_height ON address_txs (block_height);

-- Validator (masternode / evonode) registry, current state.
CREATE TABLE validators (
  protx_hash              bytea   PRIMARY KEY,
  collateral_hash         bytea   NOT NULL,
  collateral_index        INTEGER NOT NULL,
  collateral_address      TEXT,
  owner_address           TEXT,
  operator_pubkey         bytea,
  voting_address          TEXT,
  addr_core_p2p           TEXT[],
  addr_platform_p2p       TEXT[],
  addr_platform_https     TEXT[],
  payout_address          TEXT,
  operator_payout_address TEXT,
  operator_reward         INTEGER        NOT NULL DEFAULT 0,
  validator_type          validator_type NOT NULL DEFAULT 'regular',
  status                  TEXT           NOT NULL DEFAULT 'registered',
  registered_height       INTEGER        NOT NULL,
  last_updated_height     INTEGER        NOT NULL
);
CREATE INDEX idx_validator_status ON validators (status);
CREATE INDEX idx_validator_collateral ON validators (collateral_hash, collateral_index);
CREATE INDEX idx_validator_registered_height ON validators (registered_height);

-- ===========================================================================
-- internal — bounded shallow-fork undo
-- ===========================================================================
-- Itself chain-derivable (an inversion of derived rows); never exposed through
-- any query trait. Shallow forks replay these; deep forks discard them and
-- fall back to full re-derivation from ser_blocks.

-- Bounded shallow-fork undo: the pre-image a connected block overwrote, so a
-- disconnect is a structured replay rather than a full re-derive. Spent UTXOs
-- are NOT stored here — disconnect rebuilds them by joining `spends` back to the
-- still-present `tx_outputs`. Only the accumulations, which a block mutates in
-- place and cannot otherwise be inverted, keep a pre-image.
CREATE TABLE undo_blocks (
  height                      INTEGER PRIMARY KEY,
  block_hash                  bytea   NOT NULL,
  prev_hash                   bytea   NOT NULL,
  -- Singleton summary totals as they stood before this block.
  summary_tx_total            BIGINT  NOT NULL,
  summary_supply_total        BIGINT  NOT NULL,
  summary_supply_included     BIGINT  NOT NULL,
  summary_supply_unspendable  BIGINT  NOT NULL
) PARTITION BY RANGE (height);

-- What a disconnect cannot recompute about the `address_stats` rows a block
-- touched -- and nothing else. This is the highest-volume undo table in the
-- database (36,680,096 inserts and 93 autovacuum passes over a 967k-block
-- mainnet sync, the most-vacuumed object there is), so every column it does
-- not need is paid for on every block forever.
--
-- `balance`, `total_received`, `tx_count` and `utxo_count` are *additive*:
-- connect adds a per-address delta to each and nothing clamps, saturates or
-- conditions the addition, so a disconnect recovers the prior value by
-- subtracting the same delta. The delta itself needs no storage either --
-- it is a pure function of the block's own base facts (`tx_outputs` at this
-- height for what the block created, `spends` joined back to `tx_outputs`
-- for what it consumed), which are still in place when the balance stage
-- reverses (stages disconnect in reverse order, and the base stage is first
-- on connect, hence last on disconnect).
--
-- `first_seen_height` is written once, by the INSERT that creates the row,
-- and is never touched by the ON CONFLICT branch -- so for a row that
-- existed it is unchanged by definition, and for a row that did not the
-- whole row goes away. It needs no pre-image either.
--
-- That leaves the two genuinely non-invertible facts:
--   existed          -- whether the row predates this block, which decides
--                       delete-versus-restore. Not derivable from the block.
--   last_seen_height -- overwritten with this block's height unconditionally,
--                       so arithmetic cannot recover what it was.
CREATE TABLE undo_address_stats (
  height             INTEGER NOT NULL,
  address            TEXT    NOT NULL,
  existed            BOOLEAN NOT NULL,
  last_seen_height   INTEGER,
  PRIMARY KEY (height, address)
) PARTITION BY RANGE (height);

-- The prior `validators` row for each validator a block mutated. Unlike
-- `undo_address_stats` above, none of these columns is additive — they are
-- registry facts, not accumulations — so a full pre-image is the only way
-- back. `existed` = false means the block registered it, so disconnect
-- deletes the row instead of restoring columns. (Populated once
-- validator ingest lands — task #7.)
CREATE TABLE undo_validators (
  height                  INTEGER NOT NULL,
  protx_hash              bytea   NOT NULL,
  existed                 BOOLEAN NOT NULL,
  collateral_hash         bytea,
  collateral_index        INTEGER,
  collateral_address      TEXT,
  owner_address           TEXT,
  operator_pubkey         bytea,
  voting_address          TEXT,
  addr_core_p2p           TEXT[],
  addr_platform_p2p       TEXT[],
  addr_platform_https     TEXT[],
  payout_address          TEXT,
  operator_payout_address TEXT,
  operator_reward         INTEGER,
  validator_type          validator_type,
  status                  TEXT,
  registered_height       INTEGER,
  last_updated_height     INTEGER,
  PRIMARY KEY (height, protx_hash)
) PARTITION BY RANGE (height);

-- ===========================================================================
-- The offline post-IBD maintenance pass, as a runbook
-- ===========================================================================
--
-- No DDL below this line. This section records the one maintenance step the
-- schema wants an operator to run and cannot run for itself, next to the
-- schema it applies to, because an operator reading `sql/` is the one who will
-- need it. `COMMENT ON DATABASE` is the only executable form of the same note
-- and it cannot be issued portably from inside a migration, so this is a
-- comment block on purpose. (It was once its own DDL-free file, for the same
-- reason.)
--
-- WHEN
-- ----
-- Once, after initial sync completes, with the daemon stopped. Not on a
-- schedule, and not while syncing — B1's relaxed autovacuum settings exist so
-- that IBD does *not* pay for this, and doing it mid-sync just means doing it
-- again.
--
-- WHAT
-- ----
--
--   vacuumdb --freeze --analyze --jobs=N -d <database>
--
-- or per table, `VACUUM (FREEZE, ANALYZE) <table>;`. Two things are wanted and
-- neither is compaction:
--
--   FREEZE   IBD writes ~3.16 XIDs per block and every base table is
--            append-only afterwards, so the whole history is permanently
--            visible tuples that autovacuum will nonetheless have to revisit
--            for wraparound one day — as an anti-wraparound vacuum, at some
--            unchosen moment, on a running server. Freezing it while the
--            daemon is down converts that future forced scan into a chosen
--            one, and lets index-only scans use the visibility map.
--   ANALYZE  Statistics from a table that grew by four orders of magnitude
--            during sync describe a database that no longer exists. This is
--            the cheap half and the one whose absence shows up as a plan
--            cliff on the first query after sync.
--
-- Measured on a clone of an 860,000-block database (61 GB, 46.5M address_txs
-- rows): `VACUUM (FREEZE, ANALYZE) address_txs_p8` — one 100,000-block
-- partition, 883 MB of heap — took 10.6 s warm. It scales with heap size, it
-- takes no exclusive lock, and it needs no free space beyond the visibility
-- and freeze map updates. That per-partition rate is the figure to plan with;
-- a whole-database `vacuumdb --freeze --analyze -j 3` run on the same clone was
-- abandoned partway, for a reason worth knowing before scheduling one: the WAL
-- it generates makes checkpoints continuous, and every `CREATE DATABASE` /
-- `DROP DATABASE` on that server then waits on a checkpoint. This is an
-- OFFLINE, whole-server maintenance window, not something to slip in beside
-- other work on a shared instance.
--
-- NOT `VACUUM FULL`
-- -----------------
-- `VACUUM FULL` is a rewrite: it builds a complete new copy of the table and
-- its indexes, then swaps, so peak usage is the table's current size PLUS the
-- rewritten size, and it holds an ACCESS EXCLUSIVE lock throughout. Sizes from
-- the retained mainnet database at height 967,013 (84 GB total):
--
--   address_txs   24 GB      tx_inputs    13 GB      utxo_set        3.0 GB
--   tx_outputs    18 GB      spends      9.2 GB      address_stats   5.5 GB
--   transactions  4.3 GB
--
-- so a full-database `VACUUM FULL` needs ~84 GB of headroom, and the largest
-- single table alone needs ~24 GB — at a few hundred GB (the full chain, or a
-- larger network) that is a spare volume, not a maintenance window. It also
-- buys nothing this schema needs: the base tables are append-only, so they
-- carry no dead tuples to compact. `utxo_set` is the one table that churns,
-- and it is deliberately kept at a low fillfactor with aggressive autovacuum
-- (see `utxo_set` above) precisely so its free space is *reused* rather than reclaimed.
--
-- If physical compaction is ever genuinely wanted — after a mass deletion, or
-- a retention change — do it table by table, largest last, and size the
-- headroom against that one table rather than the database:
--
--   VACUUM FULL <one table>;   -- or one partition at a time
--
-- Partitioning is the lever that makes this survivable: every large table here
-- is range-partitioned by height, so the unit of rewrite is a ~2-5 GB
-- partition, not a 24 GB table. `pg_repack` is the other answer, if the
-- exclusive lock rather than the space is the binding constraint.
