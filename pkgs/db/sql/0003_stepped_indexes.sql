--
-- Copyright (c) 2026-present, The Dash Core developers
-- SPDX-License-Identifier: MIT
-- See the accompanying file LICENSE or https://opensource.org/license/MIT
--

-- ===========================================================================
-- 0003 — take the deferrable query-only indexes off the partitioned parents
-- ===========================================================================
--
-- Stepped sync (task A3) defers the query-only indexes of the partition that
-- is currently being filled and builds them in one pass when that partition
-- closes. Building an index costs O(table), not O(delta), so paying it once
-- per partition (~25 times over the chain) beats paying random-write index
-- maintenance on every one of ~3 GB of index per partition.
--
-- Why the parent indexes have to go
-- ---------------------------------
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
-- reachable if the index has no partitioned parent at all.
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
-- primary key — are deliberately not in this list and keep their parents.
--
-- Cost of this migration
-- ----------------------
-- On a fresh database there are no partitions yet, so this is seven catalog
-- deletions and nothing else. On a populated one it rebuilds these seven
-- indexes for every existing partition, because dropping a partitioned index
-- drops its children with it. There is no cheaper path: detaching each
-- partition first would preserve the child indexes but make re-attaching scan
-- every partition to re-prove its bound.

DO $$
DECLARE
  spec   record;
  part   record;
  suffix text;
BEGIN
  FOR spec IN
    SELECT * FROM (VALUES
      ('idx_transactions_txid',            'transactions', '(txid)'),
      ('idx_transactions_list',            'transactions',
       '(block_height DESC, block_pos DESC, txid DESC)'),
      ('idx_transactions_display_txid',    'transactions',
       '(display_hex(txid) text_pattern_ops)'),
      ('idx_tx_outputs_txid_vout',         'tx_outputs',   '(txid, vout_index)'),
      ('idx_tx_outputs_address',           'tx_outputs',
       '(address) WHERE address IS NOT NULL'),
      ('idx_tx_outputs_outpoint',          'tx_outputs',   '(outpoint)'),
      ('idx_address_txs_addr_height_desc', 'address_txs',
       '(address, block_height DESC, block_pos DESC, txid DESC)')
    ) AS v(idx, tbl, def)
  LOOP
    EXECUTE format('DROP INDEX IF EXISTS %I', spec.idx);

    FOR part IN
      SELECT c.relname
        FROM pg_inherits i
        JOIN pg_class c ON c.oid = i.inhrelid
       WHERE i.inhparent = spec.tbl::regclass
    LOOP
      -- The per-partition name is the parent index name plus the partition's
      -- own `_pN` suffix, which is the same rule the daemon applies when it
      -- builds one at partition close.
      suffix := substring(part.relname from '_(p[0-9]+)$');
      IF suffix IS NULL THEN
        RAISE EXCEPTION
          'partition % of % does not follow the _pN naming rule', part.relname, spec.tbl;
      END IF;
      EXECUTE format(
        'CREATE INDEX IF NOT EXISTS %I ON %I %s',
        spec.idx || '_' || suffix, part.relname, spec.def
      );
    END LOOP;
  END LOOP;
END $$;
