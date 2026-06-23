//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Live schema validation helpers.

use crate::error::DaemonError;
use crate::sql::pg_err;

#[derive(sqlx::FromRow)]
struct ForeignKeySupportRow {
  child_table: String,
  constraint_name: String,
  child_columns: Vec<String>,
}

#[derive(sqlx::FromRow)]
struct IndexNameRow {
  indexname: String,
}

#[derive(sqlx::FromRow)]
struct PartitionedTableRow {
  relname: String,
}

/// Validate the live schema against expected tables and columns.
/// Returns a list of problems (empty = schema is valid).
///
/// # Errors
///
/// Returns `DaemonError::Pg` when the information schema query fails.
pub async fn validate(pool: &sqlx::PgPool) -> Result<Vec<String>, DaemonError> {
  let expected: &[(&str, &[&str])] = &[
    ("sync_state", &["stage", "indexed_height", "updated_at"]),
    (
      "chain_progress",
      &[
        "singleton",
        "canonical_tip_height",
        "canonical_tip_hash",
        "projection_tip_height",
        "undo_tip_height",
        "recovery_state",
        "recovery_height",
        "updated_at",
      ],
    ),
    (
      "blocks",
      &["height", "hash", "prev_hash", "time", "size", "tx_count"],
    ),
    (
      "transactions",
      &[
        "txid",
        "block_height",
        "block_pos",
        "tx_type",
        "size",
        "value_out",
      ],
    ),
    ("transactions_summary", &["singleton", "total"]),
    (
      "block_undo_meta",
      &[
        "height",
        "block_hash",
        "prev_hash",
        "transactions_total_after_disconnect",
        "address_balances_total_after_disconnect",
        "address_supply_after_disconnect",
        "created_at",
      ],
    ),
    (
      "block_undo_outputs",
      &[
        "height",
        "prev_txid",
        "prev_vout",
        "prev_spent_txid",
        "prev_spent_vin",
        "prev_spent_height",
      ],
    ),
    (
      "block_undo_address_stats",
      &[
        "height",
        "address",
        "existed_before",
        "prev_balance",
        "prev_total_received",
        "prev_tx_count",
        "prev_utxo_count",
        "prev_first_seen_height",
        "prev_last_seen_height",
      ],
    ),
    (
      "block_undo_address_balances",
      &["height", "address", "existed_before", "prev_balance"],
    ),
    (
      "tx_outputs",
      &[
        "address",
        "address_type",
        "spent_txid",
        "spent_vin",
        "spent_height",
      ],
    ),
    ("tx_inputs", &["block_height", "prev_address", "prev_value"]),
    (
      "address_stats",
      &[
        "address",
        "balance",
        "total_received",
        "tx_count",
        "utxo_count",
        "first_seen_height",
        "last_seen_height",
      ],
    ),
    ("address_balances", &["address", "balance"]),
    (
      "address_txs",
      &["address", "block_height", "txid", "block_pos"],
    ),
    (
      "special_tx_payloads",
      &["block_height", "txid", "tx_type", "payload"],
    ),
    (
      "script_output_history",
      &[
        "height",
        "txid",
        "vout_index",
        "prev_address",
        "prev_address_type",
      ],
    ),
    (
      "block_metrics",
      &[
        "height",
        "tx_count",
        "utxos_spendable",
        "utxos_unspendable",
        "blockchain_size",
        "credit_pool_balance",
        "circulating_supply",
        "difficulty",
        "hashrate",
      ],
    ),
    (
      "validators",
      &[
        "pro_tx_hash",
        "owner_address",
        "status",
        "registered_height",
        "voting_address",
        "addr_core_p2p",
        "addr_platform_p2p",
        "addr_platform_https",
        "operator_payout_address",
      ],
    ),
    (
      "validator_history",
      &[
        "height",
        "sequence",
        "pro_tx_hash",
        "existed",
        "operator_payout_address",
      ],
    ),
    ("validator_counts", &["status", "validator_type", "total"]),
    (
      "spent_output_history",
      &[
        "height",
        "spending_txid",
        "spending_vin",
        "prev_txid",
        "prev_vout",
      ],
    ),
    (
      "mempool_summary",
      &["singleton_key", "tx_count", "total_bytes", "updated_at"],
    ),
    (
      "mempool_recent_txs",
      &["txid", "raw_tx", "first_seen_at", "last_seen_at"],
    ),
    (
      "mempool_tx_details",
      &[
        "txid",
        "tx_type",
        "size",
        "value_out",
        "first_seen_at",
        "last_seen_at",
      ],
    ),
    ("islock_current", &["txid", "raw_islock", "seen_at"]),
    (
      "chainlock_snapshot",
      &[
        "singleton_key",
        "block_hash",
        "block_height",
        "core_height",
        "raw_chainlock",
        "seen_at",
      ],
    ),
    (
      "validator_protx_snapshot",
      &[
        "pro_tx_hash",
        "validator_type",
        "current_status",
        "last_paid_height",
        "consecutive_payments",
        "pose_penalty",
        "pose_revived_height",
        "pose_ban_height",
        "revocation_reason",
        "updated_at",
      ],
    ),
    (
      "governance_objects",
      &["object_hash", "raw_object", "updated_at"],
    ),
    ("governance_summary", &["singleton", "proposal_total"]),
    (
      "governance_votes",
      &["vote_hash", "object_hash", "raw_vote", "updated_at"],
    ),
    (
      "network_snapshot",
      &["singleton_key", "payload", "updated_at"],
    ),
  ];

  let rows: Vec<(String, String)> = sqlx::query_as(
    "SELECT table_name, column_name
     FROM information_schema.columns
     WHERE table_schema = 'public'",
  )
  .fetch_all(pool)
  .await
  .map_err(|e| pg_err("validate query", e))?;

  let mut existing: std::collections::HashMap<String, std::collections::HashSet<String>> =
    std::collections::HashMap::new();
  for (table, column) in &rows {
    existing
      .entry(table.clone())
      .or_default()
      .insert(column.clone());
  }

  let mut problems = Vec::new();
  for &(table, columns) in expected {
    match existing.get(table) {
      None => problems.push(format!("{table}: missing table")),
      Some(cols) => {
        for &col in columns {
          if !cols.contains(col) {
            problems.push(format!("{table}: missing column {col}"));
          }
        }
      }
    }
  }

  let fk_support_rows: Vec<ForeignKeySupportRow> = sqlx::query_as(
    r"
    WITH foreign_keys AS (
      SELECT c.oid AS constraint_oid,
             c.conname AS constraint_name,
             c.conrelid AS child_relid,
             child.relname AS child_table,
             c.conkey AS child_attnums
        FROM pg_constraint AS c
        JOIN pg_class AS child ON child.oid = c.conrelid
        JOIN pg_namespace AS n ON n.oid = child.relnamespace
       WHERE c.contype = 'f'
         AND n.nspname = 'public'
    )
    SELECT fk.child_table,
           fk.constraint_name,
           ARRAY(
             SELECT a.attname
               FROM unnest(fk.child_attnums) WITH ORDINALITY AS cols(attnum, ord)
               JOIN pg_attribute AS a
                 ON a.attrelid = fk.child_relid
                AND a.attnum = cols.attnum
              ORDER BY cols.ord
           ) AS child_columns
     FROM foreign_keys AS fk
     WHERE NOT EXISTS (
             SELECT 1
               FROM pg_index AS i
              WHERE i.indrelid = fk.child_relid
                AND i.indisvalid
                AND i.indpred IS NULL
                AND i.indexprs IS NULL
                AND i.indnkeyatts >= cardinality(fk.child_attnums)
                AND (string_to_array(i.indkey::text, ' ')::smallint[])[
                      1:cardinality(fk.child_attnums)
                    ] = fk.child_attnums
           )
     ORDER BY fk.child_table, fk.constraint_name
    ",
  )
  .fetch_all(pool)
  .await
  .map_err(|e| pg_err("validate foreign key index query", e))?;

  for row in fk_support_rows {
    problems.push(format!(
      "{}: foreign key {} lacks supporting index on ({})",
      row.child_table,
      row.constraint_name,
      row.child_columns.join(", ")
    ));
  }

  let partitioned_tables: Vec<PartitionedTableRow> = sqlx::query_as(
    "SELECT c.relname
       FROM pg_partitioned_table AS p
       JOIN pg_class AS c ON c.oid = p.partrelid
       JOIN pg_namespace AS n ON n.oid = c.relnamespace
      WHERE n.nspname = 'public'
        AND c.relname = ANY($1)",
  )
  .bind(["transactions", "tx_outputs"].as_slice())
  .fetch_all(pool)
  .await
  .map_err(|e| pg_err("validate partitioned table query", e))?;
  let existing_partitioned = partitioned_tables
    .into_iter()
    .map(|row| row.relname)
    .collect::<std::collections::HashSet<_>>();
  for table in ["transactions", "tx_outputs"] {
    if !existing_partitioned.contains(table) {
      problems.push(format!("{table}: expected partitioned parent"));
    }
  }

  let expected_indexes = [
    "idx_address_balances",
    "idx_block_undo_address_balances_height",
    "idx_governance_objects_updated_at",
    "idx_governance_objects_data",
    "idx_transactions_txid",
    "idx_tx_outputs_txid_vout",
    "idx_tx_inputs_txid_vin",
    "idx_stp_txid",
  ];
  let index_rows: Vec<IndexNameRow> = sqlx::query_as(
    "SELECT indexname
       FROM pg_indexes
      WHERE schemaname = 'public'
        AND indexname = ANY($1)",
  )
  .bind(expected_indexes.as_slice())
  .fetch_all(pool)
  .await
  .map_err(|e| pg_err("validate required index query", e))?;
  let existing_indexes = index_rows
    .into_iter()
    .map(|row| row.indexname)
    .collect::<std::collections::HashSet<_>>();
  for index in expected_indexes {
    if !existing_indexes.contains(index) {
      problems.push(format!("missing index {index}"));
    }
  }

  Ok(problems)
}
