-- Consolidated schema for hyphend PostgreSQL indices.
-- Drop the database and apply this file to start fresh.
--
-- All DDL uses idempotent guards (IF NOT EXISTS, DO $$ EXCEPTION $$)
-- so the schema can be safely re-applied on a partially-existing database
-- (e.g. after a targeted per-stage reset).

-- Enums

DO $$ BEGIN
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
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

DO $$ BEGIN
  CREATE TYPE address_type AS ENUM (
    'p2pkh',
    'p2sh',
    'p2pk',
    'unknown'
  );
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

-- Sync state

CREATE TABLE IF NOT EXISTS sync_state (
  stage           TEXT    PRIMARY KEY,
  indexed_height  INTEGER NOT NULL DEFAULT -1,
  updated_at      BIGINT  NOT NULL DEFAULT 0
);

INSERT INTO sync_state (stage) VALUES
  ('block'),
  ('transaction'),
  ('metrics'),
  ('validator'),
  ('script'),
  ('spent')
ON CONFLICT DO NOTHING;

-- Durable chain progress

CREATE TABLE IF NOT EXISTS chain_progress (
  singleton               BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  canonical_tip_height    INTEGER NOT NULL DEFAULT -1,
  canonical_tip_hash      BYTEA,
  projection_tip_height   INTEGER NOT NULL DEFAULT -1,
  undo_tip_height         INTEGER NOT NULL DEFAULT -1,
  recovery_state          TEXT    NOT NULL DEFAULT 'clean'
    CHECK (recovery_state IN ('clean', 'forward_applying', 'disconnecting', 'projection_replay')),
  recovery_height         INTEGER,
  updated_at              BIGINT  NOT NULL DEFAULT 0
);

-- Blocks

CREATE TABLE IF NOT EXISTS blocks (
  height            INTEGER              PRIMARY KEY,
  hash              BYTEA                NOT NULL,
  prev_hash         BYTEA                NOT NULL,
  merkle_root       BYTEA                NOT NULL,
  time              INTEGER              NOT NULL,
  bits              INTEGER              NOT NULL,
  nonce             BIGINT               NOT NULL,
  version           INTEGER              NOT NULL,
  size              INTEGER              NOT NULL,
  tx_count          INTEGER              NOT NULL,
  chainwork         TEXT,
  cb_height         INTEGER,
  cb_merkle_root    BYTEA,
  cb_merkle_flags   INTEGER,
  cb_best_cl_height INTEGER,
  cb_best_cl_sig    BYTEA,
  cb_credit_pool    BIGINT
);

CREATE UNIQUE INDEX IF NOT EXISTS idx_blocks_hash ON blocks (hash);
CREATE INDEX IF NOT EXISTS idx_blocks_time ON blocks (time);

-- Transactions

CREATE TABLE IF NOT EXISTS transactions (
  txid          BYTEA    NOT NULL,
  block_height  INTEGER  NOT NULL REFERENCES blocks(height) ON DELETE CASCADE,
  block_pos     INTEGER  NOT NULL,
  version       INTEGER  NOT NULL,
  tx_type       tx_type  NOT NULL DEFAULT 'normal',
  lock_time     INTEGER  NOT NULL,
  size          INTEGER  NOT NULL,
  value_out     BIGINT   NOT NULL,
  input_count   INTEGER  NOT NULL,
  output_count  INTEGER  NOT NULL,
  fee           BIGINT
) PARTITION BY RANGE (block_height);

DO $$ BEGIN
  ALTER TABLE transactions DROP CONSTRAINT IF EXISTS transactions_pkey;
EXCEPTION WHEN undefined_table THEN NULL;
END $$;

DO $$
BEGIN
  IF NOT EXISTS (
    SELECT 1
      FROM pg_constraint AS c
      JOIN pg_class AS t ON t.oid = c.conrelid
      JOIN pg_namespace AS n ON n.oid = t.relnamespace
     WHERE n.nspname = 'public'
       AND t.relname = 'transactions'
       AND c.contype = 'p'
  ) THEN
    ALTER TABLE transactions
      ADD CONSTRAINT transactions_pkey PRIMARY KEY (block_height, txid);
  END IF;
EXCEPTION WHEN duplicate_object THEN NULL;
END $$;

CREATE TABLE IF NOT EXISTS transactions_summary (
  singleton  BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  total      BIGINT  NOT NULL DEFAULT 0 CHECK (total >= 0)
);

CREATE TABLE IF NOT EXISTS block_undo_meta (
  height                                   INTEGER PRIMARY KEY,
  block_hash                               BYTEA   NOT NULL,
  prev_hash                                BYTEA   NOT NULL,
  transactions_total_after_disconnect      BIGINT  NOT NULL DEFAULT 0 CHECK (transactions_total_after_disconnect >= 0),
  address_balances_total_after_disconnect  BIGINT  NOT NULL DEFAULT 0 CHECK (address_balances_total_after_disconnect >= 0),
  address_supply_after_disconnect          BIGINT  NOT NULL DEFAULT 0 CHECK (address_supply_after_disconnect >= 0),
  created_at                               BIGINT  NOT NULL DEFAULT 0
);

CREATE TABLE IF NOT EXISTS block_undo_outputs (
  height             INTEGER NOT NULL,
  prev_txid          BYTEA   NOT NULL,
  prev_vout          INTEGER NOT NULL,
  prev_spent_txid    BYTEA,
  prev_spent_vin     INTEGER,
  prev_spent_height  INTEGER,
  PRIMARY KEY (height, prev_txid, prev_vout)
);

CREATE INDEX IF NOT EXISTS idx_block_undo_outputs_height
  ON block_undo_outputs (height);

CREATE TABLE IF NOT EXISTS block_undo_address_stats (
  height                  INTEGER NOT NULL,
  address                 TEXT    NOT NULL,
  existed_before          BOOLEAN NOT NULL,
  prev_balance            BIGINT,
  prev_total_received     BIGINT,
  prev_tx_count           INTEGER,
  prev_utxo_count         INTEGER,
  prev_first_seen_height  INTEGER,
  prev_last_seen_height   INTEGER,
  PRIMARY KEY (height, address)
);

CREATE INDEX IF NOT EXISTS idx_block_undo_address_stats_height
  ON block_undo_address_stats (height);

CREATE TABLE IF NOT EXISTS block_undo_address_balances (
  height         INTEGER NOT NULL,
  address        TEXT    NOT NULL,
  existed_before BOOLEAN NOT NULL,
  prev_balance   BIGINT,
  PRIMARY KEY (height, address)
);

CREATE INDEX IF NOT EXISTS idx_block_undo_address_balances_height
  ON block_undo_address_balances (height);

CREATE INDEX IF NOT EXISTS idx_tx_block ON transactions (block_height);
CREATE INDEX IF NOT EXISTS idx_tx_block_pos_desc ON transactions (block_height DESC, block_pos DESC);
CREATE INDEX IF NOT EXISTS idx_tx_type ON transactions (tx_type);
CREATE INDEX IF NOT EXISTS idx_transactions_txid ON transactions (txid);
CREATE INDEX IF NOT EXISTS idx_tx_history_cover
  ON transactions (txid)
  INCLUDE (block_height, block_pos, version, tx_type, size, value_out, fee);

-- Transaction inputs

CREATE TABLE IF NOT EXISTS tx_inputs (
  block_height  INTEGER NOT NULL,
  txid          BYTEA   NOT NULL,
  vin_index     INTEGER NOT NULL,
  prev_txid     BYTEA,
  prev_vout     INTEGER,
  script_sig    BYTEA   NOT NULL,
  sequence      BIGINT  NOT NULL,
  prev_address  TEXT,
  prev_value    BIGINT,
  PRIMARY KEY (block_height, txid, vin_index),
  FOREIGN KEY (block_height, txid)
    REFERENCES transactions(block_height, txid) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_tx_inputs_txid_vin ON tx_inputs (txid, vin_index);

-- Transaction outputs

CREATE TABLE IF NOT EXISTS tx_outputs (
  txid          BYTEA        NOT NULL,
  vout_index    INTEGER      NOT NULL,
  block_height  INTEGER      NOT NULL,
  value         BIGINT       NOT NULL,
  script_pubkey BYTEA        NOT NULL,
  address       TEXT,
  address_type  address_type,
  spent_txid    BYTEA,
  spent_vin     INTEGER,
  spent_height  INTEGER,
  PRIMARY KEY (block_height, txid, vout_index),
  FOREIGN KEY (block_height, txid)
    REFERENCES transactions(block_height, txid) ON DELETE CASCADE
) PARTITION BY RANGE (block_height);

CREATE INDEX IF NOT EXISTS idx_tx_outputs_block_height ON tx_outputs (block_height);
CREATE INDEX IF NOT EXISTS idx_tx_outputs_txid_vout ON tx_outputs (txid, vout_index);
CREATE INDEX IF NOT EXISTS idx_tx_outputs_address ON tx_outputs (address)
  WHERE address IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_tx_outputs_address_unspent_height ON tx_outputs (
  address,
  block_height DESC,
  vout_index
) WHERE address IS NOT NULL AND spent_txid IS NULL;
CREATE INDEX IF NOT EXISTS idx_tx_outputs_spent_height ON tx_outputs (spent_height)
  WHERE spent_height IS NOT NULL;

-- Address balances snapshot

CREATE TABLE IF NOT EXISTS address_balances (
  address  TEXT   PRIMARY KEY,
  balance  BIGINT NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_address_balances
  ON address_balances (balance DESC);

CREATE TABLE IF NOT EXISTS address_balances_summary (
  singleton        BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  total            BIGINT  NOT NULL DEFAULT 0 CHECK (total >= 0),
  included_supply  BIGINT  NOT NULL DEFAULT 0 CHECK (included_supply >= 0)
);

-- Address summary and history products

CREATE TABLE IF NOT EXISTS address_stats (
  address            TEXT    PRIMARY KEY,
  balance            BIGINT  NOT NULL DEFAULT 0,
  total_received     BIGINT  NOT NULL DEFAULT 0,
  tx_count           INTEGER NOT NULL DEFAULT 0,
  utxo_count         INTEGER NOT NULL DEFAULT 0,
  first_seen_height  INTEGER,
  last_seen_height   INTEGER
);

CREATE INDEX IF NOT EXISTS idx_address_stats_balance_rank
  ON address_stats (balance DESC, address ASC)
  WHERE balance > 0;

CREATE TABLE IF NOT EXISTS address_txs (
  address       TEXT    NOT NULL,
  block_height  INTEGER NOT NULL,
  txid          BYTEA   NOT NULL,
  block_pos     INTEGER NOT NULL,
  PRIMARY KEY (address, block_height, txid),
  FOREIGN KEY (block_height, txid)
    REFERENCES transactions(block_height, txid) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_address_txs_address_height_desc
  ON address_txs (address, block_height DESC, block_pos DESC, txid DESC);
CREATE INDEX IF NOT EXISTS idx_address_txs_block_height_txid
  ON address_txs (block_height, txid);
CREATE INDEX IF NOT EXISTS idx_address_txs_txid
  ON address_txs (txid);

CREATE TABLE IF NOT EXISTS script_output_history (
  height             INTEGER NOT NULL,
  txid               BYTEA   NOT NULL,
  vout_index         INTEGER NOT NULL,
  prev_address       TEXT,
  prev_address_type  address_type,
  PRIMARY KEY (height, txid, vout_index)
) PARTITION BY RANGE (height);

-- Block metrics (cumulative counters)

CREATE TABLE IF NOT EXISTS block_metrics (
  height              INTEGER          PRIMARY KEY,
  tx_count            BIGINT           NOT NULL DEFAULT 0,
  utxos_spendable     BIGINT           NOT NULL DEFAULT 0,
  utxos_unspendable   BIGINT           NOT NULL DEFAULT 0,
  blockchain_size     BIGINT           NOT NULL DEFAULT 0,
  credit_pool_balance BIGINT,
  circulating_supply  BIGINT           NOT NULL DEFAULT 0,
  difficulty          DOUBLE PRECISION NOT NULL DEFAULT 0,
  hashrate            DOUBLE PRECISION NOT NULL DEFAULT 0
);

CREATE INDEX IF NOT EXISTS idx_block_metrics_height ON block_metrics (height DESC);

-- Special transaction payloads

CREATE TABLE IF NOT EXISTS special_tx_payloads (
  block_height  INTEGER NOT NULL,
  txid          BYTEA   NOT NULL,
  tx_type       tx_type NOT NULL,
  pro_tx_hash   BYTEA,
  payload       JSONB   NOT NULL,
  PRIMARY KEY (block_height, txid),
  FOREIGN KEY (block_height, txid)
    REFERENCES transactions(block_height, txid) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_stp_txid ON special_tx_payloads (txid);
CREATE INDEX IF NOT EXISTS idx_stp_pro_tx_hash ON special_tx_payloads (pro_tx_hash)
  WHERE pro_tx_hash IS NOT NULL;
CREATE INDEX IF NOT EXISTS idx_stp_type ON special_tx_payloads (tx_type);

-- Validators (masternodes / evonodes)

CREATE TABLE IF NOT EXISTS validators (
  pro_tx_hash         BYTEA   PRIMARY KEY,
  collateral_hash     BYTEA   NOT NULL,
  collateral_index    INTEGER NOT NULL,
  collateral_address  TEXT,
  owner_address       TEXT,
  operator_pubkey     BYTEA,
  voting_address      TEXT,
  addr_core_p2p       TEXT[],
  addr_platform_p2p   TEXT[],
  addr_platform_https TEXT[],
  payout_address      TEXT,
  operator_payout_address TEXT,
  operator_reward     INTEGER NOT NULL DEFAULT 0,
  validator_type      TEXT    NOT NULL DEFAULT 'Regular',
  status              TEXT    NOT NULL DEFAULT 'registered',
  registered_height   INTEGER NOT NULL,
  last_updated_height INTEGER NOT NULL
);

ALTER TABLE validators
  ADD COLUMN IF NOT EXISTS operator_payout_address TEXT;

CREATE TABLE IF NOT EXISTS validator_counts (
  status          TEXT   NOT NULL DEFAULT '',
  validator_type  TEXT   NOT NULL DEFAULT '',
  total           BIGINT NOT NULL DEFAULT 0 CHECK (total >= 0),
  PRIMARY KEY (status, validator_type)
);

CREATE INDEX IF NOT EXISTS idx_validator_status ON validators (status);
CREATE INDEX IF NOT EXISTS idx_validator_collateral
  ON validators (collateral_hash, collateral_index);
CREATE INDEX IF NOT EXISTS idx_validator_registered_height
  ON validators (registered_height);

CREATE TABLE IF NOT EXISTS validator_history (
  height               INTEGER NOT NULL,
  sequence             INTEGER NOT NULL,
  pro_tx_hash          BYTEA   NOT NULL,
  existed              BOOLEAN NOT NULL,
  collateral_hash      BYTEA,
  collateral_index     INTEGER,
  collateral_address   TEXT,
  owner_address        TEXT,
  operator_pubkey      BYTEA,
  voting_address       TEXT,
  addr_core_p2p        TEXT[],
  addr_platform_p2p    TEXT[],
  addr_platform_https  TEXT[],
  payout_address       TEXT,
  operator_payout_address TEXT,
  operator_reward      INTEGER,
  validator_type       TEXT,
  status               TEXT,
  registered_height    INTEGER,
  last_updated_height  INTEGER,
  PRIMARY KEY (height, sequence, pro_tx_hash)
);

ALTER TABLE validator_history
  ADD COLUMN IF NOT EXISTS operator_payout_address TEXT;

CREATE INDEX IF NOT EXISTS idx_validator_history_height
  ON validator_history (height, sequence DESC);

CREATE TABLE IF NOT EXISTS spent_output_history (
  height             INTEGER NOT NULL,
  spending_txid      BYTEA   NOT NULL,
  spending_vin       INTEGER NOT NULL,
  prev_txid          BYTEA   NOT NULL,
  prev_vout          INTEGER NOT NULL,
  prev_spent_txid    BYTEA,
  prev_spent_vin     INTEGER,
  prev_spent_height  INTEGER,
  PRIMARY KEY (height, spending_txid, spending_vin)
) PARTITION BY RANGE (height);

-- Live current-state tables

CREATE TABLE IF NOT EXISTS mempool_summary (
  singleton_key  BOOLEAN PRIMARY KEY DEFAULT TRUE,
  tx_count       BIGINT  NOT NULL DEFAULT 0,
  total_bytes    BIGINT  NOT NULL DEFAULT 0,
  usage_bytes    BIGINT,
  max_bytes      BIGINT,
  updated_at     BIGINT  NOT NULL
);

CREATE TABLE IF NOT EXISTS mempool_recent_txs (
  txid           BYTEA   PRIMARY KEY,
  raw_tx         BYTEA   NOT NULL,
  first_seen_at  BIGINT  NOT NULL,
  last_seen_at   BIGINT  NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mempool_recent_txs_last_seen
  ON mempool_recent_txs (last_seen_at DESC);

CREATE TABLE IF NOT EXISTS mempool_tx_details (
  txid           BYTEA    PRIMARY KEY REFERENCES mempool_recent_txs(txid) ON DELETE CASCADE,
  version        SMALLINT NOT NULL,
  tx_type        TEXT     NOT NULL,
  lock_time      INTEGER  NOT NULL,
  size           INTEGER  NOT NULL,
  value_out      BIGINT   NOT NULL,
  input_count    INTEGER  NOT NULL,
  output_count   INTEGER  NOT NULL,
  fee            BIGINT,
  payload        JSONB,
  first_seen_at  BIGINT   NOT NULL,
  last_seen_at   BIGINT   NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_mempool_tx_details_last_seen
  ON mempool_tx_details (last_seen_at DESC);

CREATE TABLE IF NOT EXISTS mempool_tx_inputs (
  txid          BYTEA   NOT NULL REFERENCES mempool_tx_details(txid) ON DELETE CASCADE,
  vin_index     INTEGER NOT NULL,
  sequence      BIGINT  NOT NULL,
  prev_txid     BYTEA,
  prev_vout     INTEGER,
  coinbase      BYTEA,
  script_sig    BYTEA   NOT NULL,
  prev_address  TEXT,
  prev_value    BIGINT,
  PRIMARY KEY (txid, vin_index)
);

CREATE TABLE IF NOT EXISTS mempool_tx_outputs (
  txid           BYTEA   NOT NULL REFERENCES mempool_tx_details(txid) ON DELETE CASCADE,
  vout_index     INTEGER NOT NULL,
  value          BIGINT  NOT NULL,
  script_pubkey  BYTEA   NOT NULL,
  address        TEXT,
  PRIMARY KEY (txid, vout_index)
);

CREATE TABLE IF NOT EXISTS mempool_address_txs (
  address       TEXT   NOT NULL,
  txid          BYTEA  NOT NULL REFERENCES mempool_tx_details(txid) ON DELETE CASCADE,
  last_seen_at  BIGINT NOT NULL,
  PRIMARY KEY (address, txid)
);

CREATE INDEX IF NOT EXISTS idx_mempool_address_txs_last_seen
  ON mempool_address_txs (address, last_seen_at DESC);

CREATE TABLE IF NOT EXISTS islock_current (
  txid         BYTEA   PRIMARY KEY,
  raw_islock   BYTEA   NOT NULL,
  seen_at      BIGINT  NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_islock_current_seen_at
  ON islock_current (seen_at DESC);

CREATE TABLE IF NOT EXISTS chainlock_snapshot (
  singleton_key   BOOLEAN PRIMARY KEY DEFAULT TRUE,
  block_hash      BYTEA   NOT NULL,
  block_height    INTEGER,
  core_height     INTEGER,
  raw_chainlock   BYTEA   NOT NULL,
  seen_at         BIGINT  NOT NULL
);

CREATE TABLE IF NOT EXISTS validator_protx_snapshot (
  pro_tx_hash            BYTEA   PRIMARY KEY,
  validator_type         TEXT,
  current_status         TEXT,
  last_paid_height       INTEGER,
  consecutive_payments   INTEGER,
  pose_penalty           INTEGER,
  pose_revived_height    INTEGER,
  pose_ban_height        INTEGER,
  revocation_reason      INTEGER,
  updated_at             BIGINT  NOT NULL
);

ALTER TABLE validator_protx_snapshot
  ADD COLUMN IF NOT EXISTS validator_type TEXT;

CREATE INDEX IF NOT EXISTS idx_validator_protx_snapshot_updated_at
  ON validator_protx_snapshot (updated_at DESC);

CREATE TABLE IF NOT EXISTS governance_objects (
  object_hash    BYTEA   PRIMARY KEY,
  parent_hash    BYTEA,
  revision       INTEGER,
  object_time    BIGINT,
  object_type    INTEGER,
  data_json      JSONB,
  raw_object     BYTEA   NOT NULL,
  updated_at     BIGINT  NOT NULL
);

CREATE TABLE IF NOT EXISTS governance_summary (
  singleton       BOOLEAN PRIMARY KEY DEFAULT TRUE CHECK (singleton),
  proposal_total  BIGINT  NOT NULL DEFAULT 0 CHECK (proposal_total >= 0)
);

CREATE INDEX IF NOT EXISTS idx_governance_objects_updated_at
  ON governance_objects (updated_at DESC);

CREATE INDEX IF NOT EXISTS idx_governance_objects_data
  ON governance_objects USING gin(data_json);

CREATE TABLE IF NOT EXISTS governance_votes (
  vote_hash      BYTEA   PRIMARY KEY,
  object_hash    BYTEA   NOT NULL REFERENCES governance_objects(object_hash) ON DELETE CASCADE,
  signal         INTEGER,
  outcome        INTEGER,
  vote_time      BIGINT,
  raw_vote       BYTEA   NOT NULL,
  updated_at     BIGINT  NOT NULL
);

CREATE INDEX IF NOT EXISTS idx_governance_votes_object_hash
  ON governance_votes (object_hash);

CREATE INDEX IF NOT EXISTS idx_governance_votes_updated_at
  ON governance_votes (updated_at DESC);

CREATE TABLE IF NOT EXISTS network_snapshot (
  singleton_key  BOOLEAN PRIMARY KEY DEFAULT TRUE,
  payload        JSONB   NOT NULL,
  updated_at     BIGINT  NOT NULL
);

-- Orphan block archive

CREATE TABLE IF NOT EXISTS orphan_blocks (
  hash              BYTEA                PRIMARY KEY,
  height            INTEGER              NOT NULL,
  prev_hash         BYTEA                NOT NULL,
  merkle_root       BYTEA                NOT NULL,
  time              INTEGER              NOT NULL,
  bits              INTEGER              NOT NULL,
  nonce             BIGINT               NOT NULL,
  version           INTEGER              NOT NULL,
  size              INTEGER              NOT NULL,
  tx_count          INTEGER              NOT NULL,
  chainwork         TEXT,
  cb_height         INTEGER,
  cb_merkle_root    BYTEA,
  cb_merkle_flags   INTEGER,
  cb_best_cl_height INTEGER,
  cb_best_cl_sig    BYTEA,
  cb_credit_pool    BIGINT,
  orphaned_at       TIMESTAMPTZ          NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_orphan_blocks_height ON orphan_blocks(height);
