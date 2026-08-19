//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! `HyDbSearch::prefix` and the four per-kind `search_prefix` probes are one
//! implementation, not two that happen to agree today.
//!
//! REST drives the per-kind probes directly; `prefix` is the consolidated entry
//! point that has no production caller yet. While both existed as separate SQL
//! they drifted: `prefix` ordered validators by `(last_updated_height DESC,
//! protx_hash ASC)` while `validators::search_prefix` tie-breaks on
//! `registered_height DESC` in between, so a prefix matching two validators
//! updated at the same height resolved them in different orders on the two
//! paths. The seed below is built to expose exactly that, so the test fails if
//! the copies are ever reintroduced.

mod common;

use common::TempDb;
use hyphen_db::query::{HyDbQrSearchHit, HyDbSearchKind};
use hyphen_db::HyDbHash;
use sqlx::PgPool;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// A 32-byte hash whose DISPLAY-order hex (the byte-reverse) starts with the
/// shared prefix `aa`, so all three validators are matched by one query and
/// only the ORDER BY decides which comes back first.
fn protx(tag: u8) -> Vec<u8> {
  let mut h = vec![tag; 32];
  h[31] = 0xaa;
  h
}

/// DISPLAY-order hex of a stored hash.
fn display_hex(hash: &HyDbHash) -> String {
  hash
    .as_bytes()
    .iter()
    .rev()
    .fold(String::new(), |mut s, b| {
      s.push_str(&format!("{b:02x}"));
      s
    })
}

/// Three validators sharing a display-hex prefix and a `last_updated_height`,
/// separated only by `registered_height` — the tie-break the two paths
/// disagreed about.
async fn seed_validators(pool: &PgPool) -> TestResult {
  sqlx::query(
    "INSERT INTO validators
       (protx_hash, collateral_hash, collateral_index, owner_address, addr_core_p2p,
        validator_type, status, registered_height, last_updated_height)
     VALUES ($1,$1,0,'yOwner1',ARRAY['1.1.1.1:1'],'evonode','registered',10,50),
            ($2,$2,1,'yOwner2',ARRAY['2.2.2.2:2'],'evonode','registered',30,50),
            ($3,$3,2,'yOwner3',ARRAY['3.3.3.3:3'],'regular','registered',20,50)",
  )
  .bind(protx(0x01))
  .bind(protx(0x02))
  .bind(protx(0x03))
  .execute(pool)
  .await?;
  Ok(())
}

#[tokio::test]
async fn the_consolidated_search_is_the_per_kind_probes() -> TestResult {
  let tmp = TempDb::create().await?;
  let result = run(&tmp).await;
  tmp.drop_now().await;
  result
}

async fn run(tmp: &TempDb) -> TestResult {
  let db = tmp.open().await?;
  let pool = tmp.seed_pool().await?;
  seed_validators(&pool).await?;

  // Every validator here matches `aa`, and all three share last_updated_height,
  // so this asserts the tie-break, which is where the two copies diverged.
  let direct: Vec<String> = db
    .validators()
    .search_prefix("aa".to_owned(), 50)
    .await?
    .iter()
    .map(display_hex)
    .collect();
  assert_eq!(direct.len(), 3, "all three validators match the prefix");

  let consolidated: Vec<String> = kind_ids(
    &db.search().prefix("aa".to_owned(), 50).await?,
    HyDbSearchKind::Validator,
  );
  assert_eq!(
    consolidated, direct,
    "the consolidated search must return the per-kind probe's order, \
     registered_height tie-break included"
  );

  // And the tie-break is really doing the ordering: registered_height DESC.
  assert_eq!(
    direct,
    vec![
      display_hex(&HyDbHash::from_slice(&protx(0x02))?),
      display_hex(&HyDbHash::from_slice(&protx(0x03))?),
      display_hex(&HyDbHash::from_slice(&protx(0x01))?),
    ],
    "registered heights 30, 20, 10 at equal last_updated_height"
  );

  // A query in neither charset still reaches no pool at all.
  assert!(db.search().prefix("%".to_owned(), 10).await?.is_empty());

  Ok(())
}

/// The ids of one kind, in the order the consolidated search returned them.
fn kind_ids(hits: &[HyDbQrSearchHit], kind: HyDbSearchKind) -> Vec<String> {
  hits
    .iter()
    .filter(|h| h.kind == kind)
    .map(|h| h.id.clone())
    .collect()
}
