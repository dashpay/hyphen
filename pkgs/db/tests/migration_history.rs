//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! What `migrate::apply` does about a recorded history it cannot continue.
//!
//! The pre-migration case — tables present, `_sqlx_migrations` absent — was
//! already refused. The case this file covers is the opposite shape and was
//! not: the tracking table is present and populated, but by a build whose
//! schema was stated differently. A database synced against a generation that
//! carried its schema as a sequence of deltas has exactly that history once
//! those deltas are restated as one file, and the refusal it used to get was
//! sqlx complaining about a checksum with nothing said about the remedy.

mod common;

use common::TempDb;

type Res = Result<(), Box<dyn std::error::Error>>;

/// A version this build does not carry means the database was written by a
/// schema generation this one cannot continue from.
#[tokio::test]
async fn a_history_holding_an_unknown_version_is_refused() -> Res {
  let tmp = TempDb::create().await?;
  let result = run_unknown_version(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_unknown_version(tmp: &TempDb) -> Res {
  // Opening once applies the embedded set and records it.
  let db = tmp.open().await?;
  drop(db);
  let pool = tmp.seed_pool().await?;

  // Stand in for a database that ran migrations this build no longer has.
  sqlx::query(
    "INSERT INTO _sqlx_migrations \
       (version, description, installed_on, success, checksum, execution_time) \
     VALUES (6, 'address txs one index', now(), true, '\\x00'::bytea, 0)",
  )
  .execute(&pool)
  .await?;

  let err = tmp
    .open()
    .await
    .err()
    .map(|e| e.to_string())
    .unwrap_or_default();
  assert!(
    err.contains("different schema generation"),
    "the refusal must say the database is from another generation; got {err}"
  );
  assert!(
    err.contains('6'),
    "the refusal must name the version it does not carry; got {err}"
  );
  assert!(
    err.contains("re-sync"),
    "the refusal must say what to do about it; got {err}"
  );

  Ok(())
}

/// The same version applied from different SQL is the collapse case: the
/// number matches, the contents do not.
#[tokio::test]
async fn a_history_whose_checksum_disagrees_is_refused() -> Res {
  let tmp = TempDb::create().await?;
  let result = run_rewritten_checksum(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_rewritten_checksum(tmp: &TempDb) -> Res {
  let db = tmp.open().await?;
  drop(db);
  let pool = tmp.seed_pool().await?;

  let changed =
    sqlx::query("UPDATE _sqlx_migrations SET checksum = '\\xdeadbeef'::bytea WHERE version = 1")
      .execute(&pool)
      .await?
      .rows_affected();
  assert_eq!(changed, 1, "version 1 must have been recorded");

  let err = tmp
    .open()
    .await
    .err()
    .map(|e| e.to_string())
    .unwrap_or_default();
  assert!(
    err.contains("different schema generation"),
    "a rewritten migration must be refused by the pre-flight; got {err}"
  );
  assert!(
    err.contains("different SQL"),
    "the refusal must distinguish a rewrite from a missing version; got {err}"
  );

  Ok(())
}

/// The pre-flight must not refuse the ordinary case it sits in front of.
#[tokio::test]
async fn an_untouched_history_reopens_cleanly() -> Res {
  let tmp = TempDb::create().await?;
  let result = run_clean_reopen(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run_clean_reopen(tmp: &TempDb) -> Res {
  let db = tmp.open().await?;
  drop(db);
  // Second open goes through the same pre-flight with a history this build
  // wrote itself, and has to be a no-op rather than a refusal.
  let db = tmp.open().await?;
  drop(db);
  Ok(())
}
