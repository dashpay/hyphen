//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The `[db.layout]` immutability contract, against a real database.
//!
//! `src/layout.rs` unit-tests the decision table with no server. This file
//! tests the half that only a live database can show: that migration 0002
//! actually creates `init_params`, that a first run writes every key with the
//! daemon version that wrote it, and that each of the five startup cases does
//! to the *store* what the decision table said it would.
//!
//! `verify` is called directly rather than through `HyphenDb::connect` on
//! purpose: `connect` also installs the resolved layout process-wide, and a
//! test binary that ran several different layouts through one process would be
//! testing the thing the contract exists to forbid.

mod common;

use common::TempDb;
use hyphen_db::layout::{self, HyDbLayout, HyDbLayoutCfg, HyDbLayoutGrade, LAYOUT_KEYS};
use sqlx::PgPool;

type Res = Result<(), Box<dyn std::error::Error>>;

const CREATED_BY: &str = "hyphend/test";

fn cfg_from(layout: &HyDbLayout) -> HyDbLayoutCfg {
  HyDbLayoutCfg {
    network: Some(layout.network.clone()),
    partition_span: Some(layout.partition_span),
    p2pkh_version: Some(layout.p2pkh_version),
    p2sh_version: Some(layout.p2sh_version),
    rich_list_floor: Some(layout.rich_list_floor),
    undo_span: Some(layout.undo_span),
    schema_version: Some(layout.schema_version),
  }
}

/// The layout a freshly migrated database ends up with: defaults, except that
/// `schema_version` follows the embedded migration set.
async fn baseline(pool: &PgPool) -> Result<HyDbLayout, Box<dyn std::error::Error>> {
  let rows = layout::read_store(pool).await?.unwrap_or_default();
  let mut base = HyDbLayout::default();
  for row in &rows {
    if row.key == "schema_version" {
      base.schema_version = row.value.parse().unwrap_or_default();
    }
  }
  Ok(base)
}

async fn store_keys(pool: &PgPool) -> Result<Vec<String>, Box<dyn std::error::Error>> {
  Ok(
    layout::read_store(pool)
      .await?
      .unwrap_or_default()
      .into_iter()
      .map(|r| r.key)
      .collect(),
  )
}

async fn delete_key(pool: &PgPool, key: &str) -> Res {
  sqlx::query("DELETE FROM init_params WHERE key = $1")
    .bind(key)
    .execute(pool)
    .await?;
  Ok(())
}

#[tokio::test]
async fn case1_first_run_records_the_whole_layout() -> Res {
  let temp = TempDb::create().await?;
  let result = run_case1_first_run_records_the_whole_layout(&temp).await;
  temp.drop_now().await;
  result
}

async fn run_case1_first_run_records_the_whole_layout(temp: &TempDb) -> Res {
  // `open()` migrates and runs the contract for the first time.
  let _db = temp.open().await?;
  let pool = temp.seed_pool().await?;

  let rows = layout::read_store(&pool).await?.unwrap_or_default();
  assert_eq!(
    rows.len(),
    LAYOUT_KEYS.len(),
    "first run must record every layout key, got {:?}",
    rows.iter().map(|r| &r.key).collect::<Vec<_>>()
  );
  for key in &LAYOUT_KEYS {
    let row = rows.iter().find(|r| r.key == key.name);
    assert!(row.is_some(), "`{}` was not recorded", key.name);
    if let Some(row) = row {
      assert!(
        !row.created_by.is_empty(),
        "`{}` recorded with an empty created_by",
        key.name
      );
      assert!(
        !row.created_at.is_empty(),
        "`{}` recorded with an empty created_at",
        key.name
      );
    }
  }
  // Sanity: the values are the ones the engine actually runs on.
  let floor = rows.iter().find(|r| r.key == "rich_list_floor");
  assert_eq!(
    floor.map(|r| r.value.as_str()),
    Some(layout::rich_list_floor().to_string().as_str())
  );

  pool.close().await;
  Ok(())
}

#[tokio::test]
async fn case2_unchanged_layout_is_accepted_and_rewrites_nothing() -> Res {
  let temp = TempDb::create().await?;
  let result = run_case2_unchanged_layout_is_accepted_and_rewrites_nothing(&temp).await;
  temp.drop_now().await;
  result
}

async fn run_case2_unchanged_layout_is_accepted_and_rewrites_nothing(temp: &TempDb) -> Res {
  let _db = temp.open().await?;
  let pool = temp.seed_pool().await?;
  let base = baseline(&pool).await?;

  let before = layout::read_store(&pool).await?.unwrap_or_default();
  let again = layout::verify(&pool, &cfg_from(&base), CREATED_BY).await;
  assert!(again.is_ok(), "matching layout must start: {again:?}");
  assert_eq!(again.ok(), Some(base));

  let after = layout::read_store(&pool).await?.unwrap_or_default();
  assert_eq!(before.len(), after.len());
  for (b, a) in before.iter().zip(after.iter()) {
    assert_eq!(b.key, a.key);
    assert_eq!(b.value, a.value);
    assert_eq!(
      b.created_at, a.created_at,
      "created_at must not be rewritten"
    );
  }

  pool.close().await;
  Ok(())
}

#[tokio::test]
async fn case3_mismatch_names_every_key_with_its_own_remedy() -> Res {
  let temp = TempDb::create().await?;
  let result = run_case3_mismatch_names_every_key_with_its_own_remedy(&temp).await;
  temp.drop_now().await;
  result
}

async fn run_case3_mismatch_names_every_key_with_its_own_remedy(temp: &TempDb) -> Res {
  let _db = temp.open().await?;
  let pool = temp.seed_pool().await?;
  let base = baseline(&pool).await?;

  // Three changes at once: two hard, one rebuildable. An operator who changed
  // three things must be told about three things.
  let mut cfg = cfg_from(&base);
  cfg.network = Some("testnet".to_owned());
  cfg.p2pkh_version = Some(140);
  cfg.rich_list_floor = Some(500_000_000);

  let err = layout::verify(&pool, &cfg, CREATED_BY).await.err();
  let msg = err.map(|e| e.to_string()).unwrap_or_default();
  assert!(!msg.is_empty(), "a changed layout must refuse to start");
  assert!(msg.contains("3 key(s) disagree"), "{msg}");
  assert!(
    msg.contains("network [hard]: stored=mainnet configured=testnet"),
    "{msg}"
  );
  assert!(
    msg.contains("p2pkh_version [hard]: stored=76 configured=140"),
    "{msg}"
  );
  assert!(
    msg.contains("rich_list_floor [rebuildable]: stored=1000000000 configured=500000000"),
    "{msg}"
  );

  // The two grades must not read the same. A rebuildable key that says
  // "resync" is a false alarm, and a hard key that does not is a data-loss
  // trap. Each refusal is a key line followed by its indented remedy line.
  let entry = |name: &str| -> String {
    let lines: Vec<&str> = msg.lines().collect();
    lines
      .iter()
      .position(|l| l.trim_start().starts_with(name))
      .map(|i| lines[i..(i + 2).min(lines.len())].join("\n"))
      .unwrap_or_default()
  };
  let floor_entry = entry("rich_list_floor");
  assert!(floor_entry.contains("NO RESYNC IS NEEDED"), "{floor_entry}");
  assert!(floor_entry.contains("rebuild"), "{floor_entry}");
  assert!(
    !floor_entry.contains("resync into a fresh database"),
    "{floor_entry}"
  );
  let network_entry = entry("network");
  assert!(network_entry.contains("resync"), "{network_entry}");
  assert!(
    !network_entry.contains("NO RESYNC IS NEEDED"),
    "{network_entry}"
  );

  // Nothing was written on the way out.
  let after = store_keys(&pool).await?;
  assert_eq!(after.len(), LAYOUT_KEYS.len());

  pool.close().await;
  Ok(())
}

#[tokio::test]
async fn case4_missing_key_is_adopted_when_defaultable_and_refused_when_not() -> Res {
  let temp = TempDb::create().await?;
  let result = run_case4_missing_key_is_adopted_when_defaultable_and_refused_when_not(&temp).await;
  temp.drop_now().await;
  result
}

async fn run_case4_missing_key_is_adopted_when_defaultable_and_refused_when_not(
  temp: &TempDb,
) -> Res {
  let _db = temp.open().await?;
  let pool = temp.seed_pool().await?;
  let base = baseline(&pool).await?;

  // A defaultable key the store has never seen is adopted and recorded.
  // `schema_version` is defaultable because nothing on disk was written
  // against it — it records which schema wrote the rest, not how to read it.
  delete_key(&pool, "schema_version").await?;
  let adopted = layout::verify(&pool, &cfg_from(&base), CREATED_BY).await;
  assert!(
    adopted.is_ok(),
    "defaultable key must be adopted: {adopted:?}"
  );
  assert!(
    store_keys(&pool)
      .await?
      .iter()
      .any(|k| k == "schema_version"),
    "an adopted key must be written back, or the next startup adopts it again"
  );

  // `rich_list_floor` is rebuildable but NOT defaultable: `address_rich` was
  // materialised at whatever floor was in force, so adopting a configured one
  // silently skips the rebuild that would make the table match it.
  delete_key(&pool, "rich_list_floor").await?;
  let refused = layout::verify(&pool, &cfg_from(&base), CREATED_BY).await;
  let msg = refused.err().map(|e| e.to_string()).unwrap_or_default();
  assert!(msg.contains("rich_list_floor"), "{msg}");
  assert!(msg.contains("stored=<absent>"), "{msg}");

  // A hard key the store has never seen is refused: adopting it would decide,
  // silently, how to read data that is already on disk.
  delete_key(&pool, "partition_span").await?;
  let refused = layout::verify(&pool, &cfg_from(&base), CREATED_BY).await;
  let msg = refused.err().map(|e| e.to_string()).unwrap_or_default();
  assert!(msg.contains("partition_span"), "{msg}");
  assert!(msg.contains("stored=<absent>"), "{msg}");
  assert!(
    !store_keys(&pool)
      .await?
      .iter()
      .any(|k| k == "partition_span"),
    "a refused key must not be written"
  );

  pool.close().await;
  Ok(())
}

#[tokio::test]
async fn case5_key_dropped_from_config_uses_the_stored_value() -> Res {
  let temp = TempDb::create().await?;
  let result = run_case5_key_dropped_from_config_uses_the_stored_value(&temp).await;
  temp.drop_now().await;
  result
}

async fn run_case5_key_dropped_from_config_uses_the_stored_value(temp: &TempDb) -> Res {
  let _db = temp.open().await?;
  let pool = temp.seed_pool().await?;
  let base = baseline(&pool).await?;

  // Pretend the database was built with a non-default floor, then drop the
  // key from the config entirely.
  let _ = sqlx::query("UPDATE init_params SET value = '7000000000' WHERE key = 'rich_list_floor'")
    .execute(&pool)
    .await;
  let mut cfg = cfg_from(&base);
  cfg.rich_list_floor = None;

  let resolved = layout::verify(&pool, &cfg, CREATED_BY).await;
  assert_eq!(
    resolved.ok().map(|l| l.rich_list_floor),
    Some(7_000_000_000),
    "a key absent from the config must resolve to the stored value, not the default"
  );

  pool.close().await;
  Ok(())
}

#[tokio::test]
async fn show_layout_renders_stored_beside_configured() -> Res {
  let temp = TempDb::create().await?;
  let result = run_show_layout_renders_stored_beside_configured(&temp).await;
  temp.drop_now().await;
  result
}

async fn run_show_layout_renders_stored_beside_configured(temp: &TempDb) -> Res {
  let _db = temp.open().await?;
  let pool = temp.seed_pool().await?;
  let base = baseline(&pool).await?;

  let mut cfg = cfg_from(&base);
  cfg.network = Some("testnet".to_owned());
  cfg.undo_span = None;

  let rendered = layout::show(&pool, &cfg).await.unwrap_or_default();
  assert!(rendered.contains("KEY"), "{rendered}");
  assert!(rendered.contains("row(s), first written by"), "{rendered}");
  // The disagreement is visible without starting the daemon...
  assert!(rendered.contains("REFUSED"), "{rendered}");
  assert!(rendered.contains("testnet"), "{rendered}");
  // ...and so is the built-in default standing in for an unstated key.
  assert!(rendered.contains("(500)"), "{rendered}");
  assert!(
    rendered.contains("using stored (not configured)"),
    "{rendered}"
  );

  pool.close().await;
  Ok(())
}

#[test]
fn grades_are_distinct_and_every_key_names_a_remedy() {
  let mut hard = 0;
  let mut rebuildable = 0;
  for key in &LAYOUT_KEYS {
    assert!(
      !key.remedy.is_empty(),
      "`{}` has no remedy to print",
      key.name
    );
    match key.grade {
      HyDbLayoutGrade::Hard => {
        hard += 1;
        assert!(
          key.remedy.contains("resync"),
          "hard key `{}` must name a resync",
          key.name
        );
      }
      HyDbLayoutGrade::Rebuildable => {
        rebuildable += 1;
        assert!(
          key.remedy.contains("NO RESYNC IS NEEDED"),
          "rebuildable key `{}` must say so first",
          key.name
        );
      }
      HyDbLayoutGrade::Managed => {}
    }
  }
  assert_eq!(hard, 4);
  assert_eq!(rebuildable, 2);
}
