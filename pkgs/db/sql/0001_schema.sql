-- hyphen-db schema — PostgreSQL 18.
--
-- Chain-derivable data only. Every row here is rebuildable from `ser_blocks`
-- plus the caller-supplied deriver; nothing that is application state or live
-- node state lives in this database.
--
-- Applied once by sqlx as migration 0001. Do not add IF NOT EXISTS guards —
-- migrations run exactly once and are tracked in `_sqlx_migrations`.
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

-- Lookup a transaction by txid alone (the PK leads with block_height).
CREATE INDEX idx_transactions_txid ON transactions (txid);
-- Newest-first keyset pagination of the global transaction list.
CREATE INDEX idx_transactions_list
  ON transactions (block_height DESC, block_pos DESC, txid DESC);
-- Prefix search on the display-order hex txid (see the search contract above).
CREATE INDEX idx_transactions_display_txid
  ON transactions (display_hex(txid) text_pattern_ops);

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
-- A transaction's outputs, and input resolution by consumed (txid, vout).
CREATE INDEX idx_tx_outputs_txid_vout ON tx_outputs (txid, vout_index);
CREATE INDEX idx_tx_outputs_address ON tx_outputs (address) WHERE address IS NOT NULL;
-- The generated packed key exists to be joined on (spent-status lookups and
-- the UTXO-restore step of every disconnect); without this index those joins
-- full-scan every partition.
CREATE INDEX idx_tx_outputs_outpoint ON tx_outputs (outpoint);

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
);
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
) WITH (fillfactor = 70, autovacuum_vacuum_scale_factor = 0.02);
-- Keyset pagination of an address's UTXOs, newest-first. `outpoint` packs
-- txid||vout big-endian, so DESC on it orders by txid then vout, matching the
-- historical (block_height, txid, vout) ordering.
CREATE INDEX idx_utxo_set_address_key
  ON utxo_set (address, block_height DESC, outpoint DESC)
  WHERE address IS NOT NULL;
CREATE INDEX idx_utxo_set_block_height ON utxo_set (block_height);

-- An address's transactions, newest-first, keyset-paginated. Partitioned by
-- height so each page is a bounded index range independent of address size.
CREATE TABLE address_txs (
  address       TEXT    NOT NULL,
  block_height  INTEGER NOT NULL,
  txid          bytea   NOT NULL,
  block_pos     INTEGER NOT NULL,
  PRIMARY KEY (address, block_height, txid)
) PARTITION BY RANGE (block_height);
CREATE INDEX idx_address_txs_addr_height_desc
  ON address_txs (address, block_height DESC, block_pos DESC, txid DESC);

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
