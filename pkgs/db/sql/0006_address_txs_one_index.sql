--
-- Copyright (c) 2026-present, The Dash Core developers
-- SPDX-License-Identifier: MIT
-- See the accompanying file LICENSE or https://opensource.org/license/MIT
--

-- ===========================================================================
-- 0006 — collapse address_txs's two btrees into one, built UNIQUE at close
-- ===========================================================================
--
-- `address_txs` carried two btrees whose keys are a prefix relationship:
--
--   address_txs_pkey                  (address, block_height, txid)
--   idx_address_txs_addr_height_desc  (address, block_height DESC,
--                                      block_pos DESC, txid DESC)
--
-- The first is a strict key-prefix subset of the second. On the retained
-- mainnet database that redundancy is 8.8 GB of the table's 17.6 GB of index;
-- on the 860,000-block seed, 6,527 MB of 13 GB. Nothing needs it any more:
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
--     idx_address_txs_block_height (0004), not on the pkey — neither btree
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
-- declaring the wide index UNIQUE would force it to be maintained on every
-- insert during IBD and lose stepped sync's cheap scheme entirely. Instead the
-- index stays in stepped sync's deferred set (`0003_stepped_indexes.sql`,
-- `stepped.rs::DEFERRED_INDEXES`) and is built with CREATE UNIQUE INDEX when
-- the partition closes. That build both creates the index and retroactively
-- proves the partition holds no duplicate key: if A5's dedup ever breaks, or a
-- new path bypasses it, the close fails loudly with the offending partition in
-- the index name, instead of corrupting silently. Near the tip, in realtime
-- mode, the index exists and enforcement is per-row again.
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
--
-- Cost of this migration
-- ----------------------
-- A database that is migrated *while* stepped mode has the newest partition
-- deferred gets that partition's index created here and dropped again by the
-- daemon's first reconcile, which is a wasted build and not a wrong state.
--
-- On a fresh database: one catalog deletion. On a populated one it drops the
-- pkey of every existing partition (reclaiming its share of the 6.5 GB) and
-- rebuilds each partition's wide index as unique — ~50 s per 100,000-block
-- partition at `maintenance_work_mem` = 2 GB. It fails, by design, if any
-- existing partition already holds a duplicate.
--
-- Reversibility:
--   ALTER TABLE address_txs ADD PRIMARY KEY (address, block_height, txid);
--   -- and re-create each idx_address_txs_addr_height_desc_pN non-unique.

ALTER TABLE address_txs DROP CONSTRAINT address_txs_pkey;

DO $$
DECLARE
  part   record;
  suffix text;
  idx    text;
BEGIN
  FOR part IN
    SELECT c.relname
      FROM pg_inherits i
      JOIN pg_class c ON c.oid = i.inhrelid
     WHERE i.inhparent = 'address_txs'::regclass
  LOOP
    -- The per-partition name is the stem plus the partition's own `_pN`
    -- suffix: the same rule 0003 applies and the daemon applies at close.
    suffix := substring(part.relname from '_(p[0-9]+)$');
    IF suffix IS NULL THEN
      RAISE EXCEPTION
        'partition % of address_txs does not follow the _pN naming rule', part.relname;
    END IF;
    idx := 'idx_address_txs_addr_height_desc_' || suffix;
    EXECUTE format('DROP INDEX IF EXISTS %I', idx);
    EXECUTE format(
      'CREATE UNIQUE INDEX %I ON %I '
      '(address, block_height DESC, block_pos DESC, txid DESC)',
      idx, part.relname
    );
  END LOOP;
END $$;
