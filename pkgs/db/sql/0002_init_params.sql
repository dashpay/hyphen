-- The storage-layout contract: `[db.layout]` as it stood when this database
-- was created.
--
-- Why this is 0002 and not part of 0001: sqlx checksums every applied
-- migration, so editing `0001_schema.sql` would make every database that has
-- already run it — including the 84 GB mainnet baseline — refuse to open with
-- a version-mismatch error. The layout table is therefore its own migration.
-- It still runs before any block is written, which is the property the
-- contract actually needs: `hyphen-db` resolves the layout immediately after
-- `MIGRATOR.run`, on a table that is guaranteed to exist by then.
--
-- The rows are the record of what the bytes in this database *mean*. They are
-- written once, on the first startup against a fresh database, and after that
-- a disagreement between them and the configuration refuses startup rather
-- than reinterpreting data already on disk. See `src/layout.rs` for the
-- grades (hard / rebuildable / managed) and their remedies.
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
