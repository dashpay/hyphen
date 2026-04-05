//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! SQL helpers for live governance proposal and vote state.

use std::collections::HashMap;

use sqlx::PgPool;

use crate::error::DaemonError;

const PUBLIC_VALIDATOR_TYPE_FROM_VOTE_SQL: &str = "CASE
         WHEN lower(regexp_replace(COALESCE(vps.validator_type, validators.validator_type, ''), '[^a-zA-Z0-9]+', '', 'g'))
           IN ('highperformance', 'evo', 'evonode')
           THEN 'evonode'
         WHEN lower(regexp_replace(COALESCE(vps.validator_type, validators.validator_type, ''), '[^a-zA-Z0-9]+', '', 'g')) = 'regular'
           THEN 'regular'
         ELSE 'unknown'
       END";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct GovernanceSummaryRow {
  pub proposal_total: i64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct GovernanceVoteTypeCountsRow {
  pub voted_regular_count: i64,
  pub voted_evo_count: i64,
}

/// Persisted governance object row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceObjectRow {
  pub object_hash: Vec<u8>,
  pub parent_hash: Option<Vec<u8>>,
  pub revision: Option<i32>,
  pub object_time: Option<i64>,
  pub object_type: i32,
  pub data_json: Option<serde_json::Value>,
  pub raw_object: Vec<u8>,
  pub updated_at: i64,
}

/// Persisted governance vote row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GovernanceVoteRow {
  pub vote_hash: Vec<u8>,
  pub object_hash: Vec<u8>,
  pub signal: Option<i32>,
  pub outcome: Option<i32>,
  pub vote_time: Option<i64>,
  pub raw_vote: Vec<u8>,
  pub updated_at: i64,
}

#[derive(Debug, Clone)]
pub struct GovernanceSearchRow {
  pub object_hash: Vec<u8>,
  pub updated_at: i64,
}

/// Replace all current governance objects in one refresh cycle.
///
/// Existing votes are removed by the `ON DELETE CASCADE` relationship and must
/// be repopulated by the caller in the same refresh flow.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects any delete or insert in
/// the refresh cycle.
pub async fn replace_objects(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  objects: &[GovernanceObjectRow],
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM governance_votes")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear governance votes: {e}")))?;
  sqlx::query("DELETE FROM governance_objects")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear governance objects: {e}")))?;

  if !objects.is_empty() {
    sqlx::query(
      "INSERT INTO governance_objects
         (object_hash, parent_hash, revision, object_time, object_type, data_json, raw_object, updated_at)
       SELECT * FROM unnest(
         $1::bytea[], $2::bytea[], $3::int[], $4::bigint[],
         $5::int[], $6::jsonb[], $7::bytea[], $8::bigint[]
       )",
    )
    .bind(objects.iter().map(|row| row.object_hash.clone()).collect::<Vec<_>>())
    .bind(objects.iter().map(|row| row.parent_hash.clone()).collect::<Vec<_>>())
    .bind(objects.iter().map(|row| row.revision).collect::<Vec<_>>())
    .bind(objects.iter().map(|row| row.object_time).collect::<Vec<_>>())
    .bind(objects.iter().map(|row| row.object_type).collect::<Vec<_>>())
    .bind(objects.iter().map(|row| row.data_json.clone()).collect::<Vec<_>>())
    .bind(objects.iter().map(|row| row.raw_object.clone()).collect::<Vec<_>>())
    .bind(objects.iter().map(|row| row.updated_at).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("insert governance objects: {e}")))?;
  }
  set_governance_summary_total(tx, objects.len() as i64).await?;
  Ok(())
}

/// Upsert one governance object row without disturbing other objects or votes.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the upsert.
pub async fn upsert_object(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  object: &GovernanceObjectRow,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO governance_objects
       (object_hash, parent_hash, revision, object_time, object_type, data_json, raw_object, updated_at)
     VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
     ON CONFLICT (object_hash) DO UPDATE SET
       parent_hash = EXCLUDED.parent_hash,
       revision = EXCLUDED.revision,
       object_time = EXCLUDED.object_time,
       object_type = EXCLUDED.object_type,
       data_json = EXCLUDED.data_json,
       raw_object = EXCLUDED.raw_object,
       updated_at = EXCLUDED.updated_at",
  )
  .bind(&object.object_hash)
  .bind(&object.parent_hash)
  .bind(object.revision)
  .bind(object.object_time)
  .bind(object.object_type)
  .bind(&object.data_json)
  .bind(&object.raw_object)
  .bind(object.updated_at)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("upsert governance object: {e}")))?;
  refresh_governance_summary(tx).await?;
  Ok(())
}

/// Replace all current governance votes in one refresh cycle.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects any delete or insert in
/// the refresh cycle.
pub async fn replace_votes(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  votes: &[GovernanceVoteRow],
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM governance_votes")
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear governance votes: {e}")))?;

  if !votes.is_empty() {
    sqlx::query(
      "INSERT INTO governance_votes
         (vote_hash, object_hash, signal, outcome, vote_time, raw_vote, updated_at)
       SELECT * FROM unnest(
         $1::bytea[], $2::bytea[], $3::int[], $4::int[], $5::bigint[], $6::bytea[], $7::bigint[]
       )",
    )
    .bind(
      votes
        .iter()
        .map(|row| row.vote_hash.clone())
        .collect::<Vec<_>>(),
    )
    .bind(
      votes
        .iter()
        .map(|row| row.object_hash.clone())
        .collect::<Vec<_>>(),
    )
    .bind(votes.iter().map(|row| row.signal).collect::<Vec<_>>())
    .bind(votes.iter().map(|row| row.outcome).collect::<Vec<_>>())
    .bind(votes.iter().map(|row| row.vote_time).collect::<Vec<_>>())
    .bind(
      votes
        .iter()
        .map(|row| row.raw_vote.clone())
        .collect::<Vec<_>>(),
    )
    .bind(votes.iter().map(|row| row.updated_at).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("insert governance votes: {e}")))?;
  }
  Ok(())
}

/// Replace the current governance votes for one governance object.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the delete or any vote
/// insert for the object.
pub async fn replace_votes_for_object(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  object_hash: &[u8],
  votes: &[GovernanceVoteRow],
) -> Result<(), DaemonError> {
  sqlx::query("DELETE FROM governance_votes WHERE object_hash = $1")
    .bind(object_hash)
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("clear governance votes for object: {e}")))?;

  if !votes.is_empty() {
    sqlx::query(
      "INSERT INTO governance_votes
         (vote_hash, object_hash, signal, outcome, vote_time, raw_vote, updated_at)
       SELECT * FROM unnest(
         $1::bytea[], $2::bytea[], $3::int[], $4::int[], $5::bigint[], $6::bytea[], $7::bigint[]
       )",
    )
    .bind(
      votes
        .iter()
        .map(|row| row.vote_hash.clone())
        .collect::<Vec<_>>(),
    )
    .bind(
      votes
        .iter()
        .map(|row| row.object_hash.clone())
        .collect::<Vec<_>>(),
    )
    .bind(votes.iter().map(|row| row.signal).collect::<Vec<_>>())
    .bind(votes.iter().map(|row| row.outcome).collect::<Vec<_>>())
    .bind(votes.iter().map(|row| row.vote_time).collect::<Vec<_>>())
    .bind(
      votes
        .iter()
        .map(|row| row.raw_vote.clone())
        .collect::<Vec<_>>(),
    )
    .bind(votes.iter().map(|row| row.updated_at).collect::<Vec<_>>())
    .execute(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("insert governance votes for object: {e}")))?;
  }
  Ok(())
}

/// Read the current governance object count snapshot.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn governance_summary(pool: &PgPool) -> Result<GovernanceSummaryRow, DaemonError> {
  let proposal_total = sqlx::query_scalar::<_, Option<i64>>(
    "SELECT proposal_total
       FROM governance_summary
      WHERE singleton = TRUE",
  )
  .fetch_optional(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read governance summary: {e}")))?
  .flatten()
  .unwrap_or(0);
  Ok(GovernanceSummaryRow { proposal_total })
}

/// Read unique voter type counts for the selected governance objects.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn vote_type_counts(
  pool: &PgPool,
  object_hashes: &[Vec<u8>],
) -> Result<HashMap<Vec<u8>, GovernanceVoteTypeCountsRow>, DaemonError> {
  use sqlx::Row;

  if object_hashes.is_empty() {
    return Ok(HashMap::new());
  }

  let query = format!(
    "WITH vote_members AS (
       SELECT DISTINCT
              gv.object_hash,
              lower(split_part(split_part(convert_from(gv.raw_vote, 'UTF8'), ':', 1), '-', 1)) AS pro_tx_hash_hex
         FROM governance_votes AS gv
        WHERE gv.object_hash = ANY($1::bytea[])
     )
     SELECT vm.object_hash,
            COUNT(*) FILTER (WHERE {public_type} = 'regular')::bigint AS voted_regular_count,
            COUNT(*) FILTER (WHERE {public_type} = 'evonode')::bigint AS voted_evo_count
       FROM vote_members AS vm
       LEFT JOIN validator_protx_snapshot AS vps
              ON encode(vps.pro_tx_hash, 'hex') = vm.pro_tx_hash_hex
       LEFT JOIN validators
              ON encode(validators.pro_tx_hash, 'hex') = vm.pro_tx_hash_hex
      GROUP BY vm.object_hash",
    public_type = PUBLIC_VALIDATOR_TYPE_FROM_VOTE_SQL,
  );

  let rows = sqlx::query(query.as_str())
    .bind(object_hashes)
    .fetch_all(pool)
    .await
    .map_err(|e| DaemonError::Pg(format!("read governance vote type counts: {e}")))?;

  Ok(
    rows
      .into_iter()
      .map(|row| {
        (
          row.get("object_hash"),
          GovernanceVoteTypeCountsRow {
            voted_regular_count: row.get("voted_regular_count"),
            voted_evo_count: row.get("voted_evo_count"),
          },
        )
      })
      .collect(),
  )
}

async fn refresh_governance_summary(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
) -> Result<(), DaemonError> {
  let total = sqlx::query_scalar::<_, i64>("SELECT COUNT(*)::bigint FROM governance_objects")
    .fetch_one(&mut **tx)
    .await
    .map_err(|e| DaemonError::Pg(format!("count governance objects: {e}")))?;
  set_governance_summary_total(tx, total).await
}

async fn set_governance_summary_total(
  tx: &mut sqlx::Transaction<'_, sqlx::Postgres>,
  total: i64,
) -> Result<(), DaemonError> {
  sqlx::query(
    "INSERT INTO governance_summary (singleton, proposal_total)
     VALUES (TRUE, $1)
     ON CONFLICT (singleton) DO UPDATE SET
       proposal_total = EXCLUDED.proposal_total",
  )
  .bind(total)
  .execute(&mut **tx)
  .await
  .map_err(|e| DaemonError::Pg(format!("set governance summary total: {e}")))?;
  Ok(())
}

/// Read paginated governance objects ordered by newest update first.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn objects(
  pool: &PgPool,
  limit: i64,
  after: Option<(i64, &[u8])>,
) -> Result<Vec<GovernanceObjectRow>, DaemonError> {
  use sqlx::Row;
  let rows = match after {
    Some((after_updated_at, after_object_hash)) => {
      sqlx::query(
        "SELECT object_hash, parent_hash, revision, object_time, object_type, data_json, raw_object, updated_at
         FROM governance_objects
         WHERE updated_at < $1
            OR (updated_at = $1 AND object_hash > $2)
         ORDER BY updated_at DESC, object_hash ASC
         LIMIT $3",
      )
      .bind(after_updated_at)
      .bind(after_object_hash)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    None => {
      sqlx::query(
        "SELECT object_hash, parent_hash, revision, object_time, object_type, data_json, raw_object, updated_at
         FROM governance_objects
         ORDER BY updated_at DESC, object_hash ASC
         LIMIT $1",
      )
      .bind(limit)
      .fetch_all(pool)
      .await
    }
  }
  .map_err(|e| DaemonError::Pg(format!("read governance objects: {e}")))?;
  Ok(
    rows
      .into_iter()
      .map(|row| GovernanceObjectRow {
        object_hash: row.get("object_hash"),
        parent_hash: row.get("parent_hash"),
        revision: row.get("revision"),
        object_time: row.get("object_time"),
        object_type: row.get("object_type"),
        data_json: row.get("data_json"),
        raw_object: row.get("raw_object"),
        updated_at: row.get("updated_at"),
      })
      .collect(),
  )
}

/// Read all current governance objects ordered by newest update first.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn all_objects(pool: &PgPool) -> Result<Vec<GovernanceObjectRow>, DaemonError> {
  use sqlx::Row;
  let rows = sqlx::query(
    "SELECT object_hash, parent_hash, revision, object_time, object_type, data_json, raw_object, updated_at
     FROM governance_objects
     ORDER BY updated_at DESC, object_hash",
  )
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read all governance objects: {e}")))?;
  Ok(
    rows
      .into_iter()
      .map(|row| GovernanceObjectRow {
        object_hash: row.get("object_hash"),
        parent_hash: row.get("parent_hash"),
        revision: row.get("revision"),
        object_time: row.get("object_time"),
        object_type: row.get("object_type"),
        data_json: row.get("data_json"),
        raw_object: row.get("raw_object"),
        updated_at: row.get("updated_at"),
      })
      .collect(),
  )
}

/// Read one governance object by hash.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn object(
  pool: &PgPool,
  hash: &[u8],
) -> Result<Option<GovernanceObjectRow>, DaemonError> {
  use sqlx::Row;
  let row = sqlx::query(
    "SELECT object_hash, parent_hash, revision, object_time, object_type, data_json, raw_object, updated_at
     FROM governance_objects
     WHERE object_hash = $1",
  )
  .bind(hash)
  .fetch_optional(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("read governance object: {e}")))?;
  Ok(row.map(|row| GovernanceObjectRow {
    object_hash: row.get("object_hash"),
    parent_hash: row.get("parent_hash"),
    revision: row.get("revision"),
    object_time: row.get("object_time"),
    object_type: row.get("object_type"),
    data_json: row.get("data_json"),
    raw_object: row.get("raw_object"),
    updated_at: row.get("updated_at"),
  }))
}

/// Read paginated current votes for one governance object.
pub async fn votes(
  pool: &PgPool,
  object_hash: &[u8],
  limit: i64,
  after: Option<(i64, &[u8])>,
) -> Result<Vec<GovernanceVoteRow>, DaemonError> {
  use sqlx::Row;
  let rows = match after {
    Some((after_updated_at, after_vote_hash)) => {
      sqlx::query(
        "SELECT vote_hash, object_hash, signal, outcome, vote_time, raw_vote, updated_at
         FROM governance_votes
         WHERE object_hash = $1
           AND (
             updated_at < $2
             OR (updated_at = $2 AND vote_hash > $3)
           )
         ORDER BY updated_at DESC, vote_hash ASC
         LIMIT $4",
      )
      .bind(object_hash)
      .bind(after_updated_at)
      .bind(after_vote_hash)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
    None => {
      sqlx::query(
        "SELECT vote_hash, object_hash, signal, outcome, vote_time, raw_vote, updated_at
         FROM governance_votes
         WHERE object_hash = $1
         ORDER BY updated_at DESC, vote_hash ASC
         LIMIT $2",
      )
      .bind(object_hash)
      .bind(limit)
      .fetch_all(pool)
      .await
    }
  }
  .map_err(|e| DaemonError::Pg(format!("read governance votes: {e}")))?;
  Ok(
    rows
      .into_iter()
      .map(|row| GovernanceVoteRow {
        vote_hash: row.get("vote_hash"),
        object_hash: row.get("object_hash"),
        signal: row.get("signal"),
        outcome: row.get("outcome"),
        vote_time: row.get("vote_time"),
        raw_vote: row.get("raw_vote"),
        updated_at: row.get("updated_at"),
      })
      .collect(),
  )
}

/// Read governance proposal suggestions by canonical hash prefix or stored text.
///
/// # Errors
///
/// Returns `DaemonError::Pg` when PostgreSQL rejects the query.
pub async fn search_objects_by_query(
  pool: &PgPool,
  query: &str,
  limit: i64,
) -> Result<Vec<GovernanceSearchRow>, DaemonError> {
  use sqlx::Row;

  let hash_pattern = format!("{query}%");
  let text_pattern = format!("%{query}%");
  let rows = sqlx::query(
    "SELECT object_hash, updated_at
       FROM governance_objects
      WHERE encode(object_hash, 'hex') LIKE $1
         OR COALESCE(data_json::text, convert_from(raw_object, 'UTF8')) ILIKE $2
      ORDER BY updated_at DESC, object_hash ASC
      LIMIT $3",
  )
  .bind(hash_pattern)
  .bind(text_pattern)
  .bind(limit)
  .fetch_all(pool)
  .await
  .map_err(|e| DaemonError::Pg(format!("search governance objects: {e}")))?;

  Ok(
    rows
      .into_iter()
      .map(|row| GovernanceSearchRow {
        object_hash: row.get("object_hash"),
        updated_at: row.get("updated_at"),
      })
      .collect(),
  )
}

#[cfg(test)]
mod tests {
  use super::{
    governance_summary, replace_objects, replace_votes, replace_votes_for_object, upsert_object,
    votes,
  };

  fn sample_object(
    object_hash: Vec<u8>,
    name: &str,
    updated_at: i64,
  ) -> super::GovernanceObjectRow {
    super::GovernanceObjectRow {
      object_hash,
      parent_hash: None,
      revision: Some(1),
      object_time: Some(updated_at),
      object_type: 1,
      data_json: Some(serde_json::json!({ "name": name })),
      raw_object: format!("{{\"name\":\"{name}\"}}").into_bytes(),
      updated_at,
    }
  }

  #[tokio::test]
  async fn replace_votes_for_object_preserves_other_objects_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let object_a = vec![0x11; 32];
    let object_b = vec![0x22; 32];

    let mut tx = pool.begin().await?;
    replace_objects(
      &mut tx,
      &[
        sample_object(object_a.clone(), "a", 10),
        sample_object(object_b.clone(), "b", 11),
      ],
    )
    .await?;
    replace_votes(
      &mut tx,
      &[
        super::GovernanceVoteRow {
          vote_hash: vec![0x31; 32],
          object_hash: object_a.clone(),
          signal: Some(1),
          outcome: Some(1),
          vote_time: Some(20),
          raw_vote: b"a-1".to_vec(),
          updated_at: 20,
        },
        super::GovernanceVoteRow {
          vote_hash: vec![0x32; 32],
          object_hash: object_a.clone(),
          signal: Some(1),
          outcome: Some(2),
          vote_time: Some(21),
          raw_vote: b"a-2".to_vec(),
          updated_at: 21,
        },
        super::GovernanceVoteRow {
          vote_hash: vec![0x41; 32],
          object_hash: object_b.clone(),
          signal: Some(1),
          outcome: Some(1),
          vote_time: Some(22),
          raw_vote: b"b-1".to_vec(),
          updated_at: 22,
        },
      ],
    )
    .await?;
    tx.commit().await?;

    let mut tx = pool.begin().await?;
    replace_votes_for_object(
      &mut tx,
      &object_a,
      &[super::GovernanceVoteRow {
        vote_hash: vec![0x33; 32],
        object_hash: object_a.clone(),
        signal: Some(2),
        outcome: Some(3),
        vote_time: Some(30),
        raw_vote: b"a-3".to_vec(),
        updated_at: 30,
      }],
    )
    .await?;
    tx.commit().await?;

    assert_eq!(votes(&pool, &object_a, 10, None).await?.len(), 1);
    assert_eq!(votes(&pool, &object_b, 10, None).await?.len(), 1);
    assert_eq!(governance_summary(&pool).await?.proposal_total, 2);

    Ok(())
  }

  #[tokio::test]
  async fn governance_summary_tracks_replace_and_upsert_when_pg_is_configured(
  ) -> Result<(), Box<dyn std::error::Error>> {
    let Some(pool) = crate::tests::pg_pool().await? else {
      return Ok(());
    };
    crate::sql::bootstrap::reset(&pool).await?;

    let mut tx = pool.begin().await?;
    replace_objects(
      &mut tx,
      &[
        sample_object(vec![0x11; 32], "a", 10),
        sample_object(vec![0x22; 32], "b", 11),
      ],
    )
    .await?;
    tx.commit().await?;
    assert_eq!(governance_summary(&pool).await?.proposal_total, 2);

    let mut tx = pool.begin().await?;
    upsert_object(&mut tx, &sample_object(vec![0x33; 32], "c", 12)).await?;
    tx.commit().await?;
    assert_eq!(governance_summary(&pool).await?.proposal_total, 3);

    Ok(())
  }
}
