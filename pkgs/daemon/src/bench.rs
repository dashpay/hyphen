//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Benchmark support and shared runtime state for the sibling benchmark package.

use std::fmt;
use std::sync::OnceLock;

use crate::error::DaemonError;
use crate::sql;

static BENCH_RUNTIME: OnceLock<BenchRuntime> = OnceLock::new();

/// One named benchmark workload group exposed by the runner.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorkloadGroup {
  Ingest,
  Spent,
  Index,
  Live,
  Write,
}

impl WorkloadGroup {
  fn parse(value: &str) -> Result<Self, String> {
    match value {
      "ingest" => Ok(Self::Ingest),
      "spent" => Ok(Self::Spent),
      "index" => Ok(Self::Index),
      "live" => Ok(Self::Live),
      "write" => Ok(Self::Write),
      _ => Err(format!(
        "unknown benchmark group '{value}', expected one of: ingest, spent, index, live, write"
      )),
    }
  }

  fn as_str(self) -> &'static str {
    match self {
      Self::Ingest => "ingest",
      Self::Spent => "spent",
      Self::Index => "index",
      Self::Live => "live",
      Self::Write => "write",
    }
  }
}

/// One named benchmark dataset size.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BenchDataset {
  Small,
  Default,
  Large,
}

impl BenchDataset {
  pub fn parse(value: &str) -> Result<Self, String> {
    match value {
      "small" => Ok(Self::Small),
      "default" => Ok(Self::Default),
      "large" => Ok(Self::Large),
      _ => Err(format!(
        "unknown benchmark dataset '{value}', expected one of: small, default, large"
      )),
    }
  }

  pub fn as_str(self) -> &'static str {
    match self {
      Self::Small => "small",
      Self::Default => "default",
      Self::Large => "large",
    }
  }
}

#[derive(Debug)]
struct BenchArgs {
  database_url: String,
  setup: bool,
  dataset: Option<BenchDataset>,
  group: Option<WorkloadGroup>,
}

/// Shared benchmark runtime state available to benchmark definitions.
#[derive(Debug)]
pub struct BenchRuntime {
  pool: sql::SqlPool,
  runtime: tokio::runtime::Runtime,
  dataset: BenchDataset,
  group: Option<WorkloadGroup>,
}

impl BenchRuntime {
  fn should_run(&self, group: WorkloadGroup) -> bool {
    self.group.is_none_or(|selected| selected == group)
  }

  /// Run one asynchronous benchmark operation on the shared runtime.
  pub fn run_async<T, F>(&self, future: F) -> T
  where
    F: std::future::Future<Output = T>,
  {
    self.runtime.block_on(future)
  }

  /// Borrow the primed SQL pool used by timed benchmark operations.
  pub fn pool(&self) -> &sql::SqlPool {
    &self.pool
  }

  /// Return the selected dataset size for this benchmark run.
  pub fn dataset(&self) -> BenchDataset {
    self.dataset
  }
}

/// Initialize benchmark support from process arguments and then run one benchmark main.
///
/// # Errors
///
/// Returns an error when benchmark setup fails, when the database is not
/// primed, or when CLI parsing fails.
pub fn run_with<F>(main: F) -> Result<(), DaemonError>
where
  F: FnOnce(),
{
  let args = parse_args().map_err(DaemonError::Config)?;
  let runtime = build_runtime()?;

  if args.setup {
    let dataset = args.dataset.unwrap_or(BenchDataset::Default);
    runtime.block_on(sql::bench::prime_database(&args.database_url, dataset))?;
    println!(
      "Primed benchmark schema '{}' for dataset '{}'",
      sql::bench::schema_name(dataset),
      dataset.as_str()
    );
    return Ok(());
  }

  let dataset = runtime.block_on(sql::bench::resolve_dataset(
    &args.database_url,
    args.dataset,
  ))?;
  let pool = runtime.block_on(sql::bench::open_primed_pool(&args.database_url, dataset))?;
  let state = BenchRuntime {
    pool,
    runtime,
    dataset,
    group: args.group,
  };
  BENCH_RUNTIME
    .set(state)
    .map_err(|_| DaemonError::Config("benchmark runtime already initialized".to_owned()))?;
  println!(
    "Running benchmark dataset='{}' group='{}' schema='{}'",
    dataset.as_str(),
    args.group.map(WorkloadGroup::as_str).unwrap_or("all"),
    sql::bench::schema_name(dataset)
  );
  main();
  Ok(())
}

/// Return the shared benchmark runtime.
pub fn runtime() -> &'static BenchRuntime {
  BENCH_RUNTIME
    .get()
    .unwrap_or_else(|| panic!("benchmark runtime must be initialized before running benches"))
}

/// Return true when the given workload group should run in this invocation.
pub fn should_run(group: WorkloadGroup) -> bool {
  runtime().should_run(group)
}

/// Run one real ingest admission decision on the shared benchmark context.
///
/// # Errors
///
/// Returns `DaemonError` when the admission path fails unexpectedly.
pub fn ingest_admit_batch_bench() -> Result<usize, DaemonError> {
  crate::ingest::bench_admit_batch()
}

/// Run one real ingest fetch on the shared benchmark context.
///
/// # Errors
///
/// Returns `DaemonError` when the fetch path fails unexpectedly.
pub fn ingest_fetch_batch_bench() -> Result<usize, DaemonError> {
  crate::ingest::bench_fetch_batch()
}

/// Run one real ingest commit on the shared benchmark context.
///
/// # Errors
///
/// Returns `DaemonError` when the commit path fails unexpectedly.
pub fn ingest_commit_batch_bench() -> Result<usize, DaemonError> {
  crate::ingest::bench_commit_batch()
}

/// Run one real ingest admit-fetch-commit turn on the shared benchmark context.
///
/// # Errors
///
/// Returns `DaemonError` when the turn fails unexpectedly.
pub fn ingest_turn_admit_fetch_commit_bench() -> Result<usize, DaemonError> {
  crate::ingest::bench_turn_admit_fetch_commit()
}

fn build_runtime() -> Result<tokio::runtime::Runtime, DaemonError> {
  tokio::runtime::Builder::new_multi_thread()
    .enable_all()
    .build()
    .map_err(|source| DaemonError::Io {
      context: "build benchmark runtime",
      source,
    })
}

fn parse_args() -> Result<BenchArgs, String> {
  parse_args_from(std::env::args().skip(1))
}

fn parse_args_from<I>(arguments: I) -> Result<BenchArgs, String>
where
  I: IntoIterator<Item = String>,
{
  let mut database_url: Option<String> = None;
  let mut setup = false;
  let mut dataset: Option<BenchDataset> = None;
  let mut group: Option<WorkloadGroup> = None;

  for argument in arguments {
    if let Some(value) = argument.strip_prefix("--url=") {
      database_url = Some(value.to_owned());
      continue;
    }
    if argument == "--setup" {
      setup = true;
      continue;
    }
    if let Some(value) = argument.strip_prefix("--dataset=") {
      dataset = Some(BenchDataset::parse(value)?);
      continue;
    }
    if argument.starts_with("--") {
      return Err(format!("unknown benchmark argument '{argument}'"));
    }
    if group.is_some() {
      return Err(format!(
        "unexpected extra positional benchmark group '{argument}'"
      ));
    }
    group = Some(WorkloadGroup::parse(argument.as_str())?);
  }

  let database_url = database_url.ok_or_else(|| {
    "missing required benchmark argument --url=postgresql://... or --url=postgresql:///db?host=/tmp"
      .to_owned()
  })?;
  if !setup && dataset.is_some() {
    return Err("benchmark argument --dataset=... is only valid with --setup".to_owned());
  }

  Ok(BenchArgs {
    database_url,
    setup,
    dataset,
    group,
  })
}

impl fmt::Display for BenchDataset {
  fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
    f.write_str(self.as_str())
  }
}

#[cfg(test)]
mod tests {
  use super::{parse_args_from, BenchDataset, WorkloadGroup};

  #[test]
  fn parse_args_defaults_dataset_and_group() {
    let args = parse_args_from(["--url=postgresql:///hyphen?host=/tmp".to_owned()])
      .expect("benchmark args should parse");

    assert_eq!(args.database_url, "postgresql:///hyphen?host=/tmp");
    assert_eq!(args.dataset, None);
    assert_eq!(args.group, None);
    assert!(!args.setup);
  }

  #[test]
  fn parse_args_supports_setup_dataset_and_group() {
    let args = parse_args_from([
      "--url=postgresql:///hyphen?host=/tmp".to_owned(),
      "--setup".to_owned(),
      "--dataset=large".to_owned(),
      "live".to_owned(),
    ])
    .expect("benchmark args should parse");

    assert!(args.setup);
    assert_eq!(args.dataset, Some(BenchDataset::Large));
    assert_eq!(args.group, Some(WorkloadGroup::Live));
  }

  #[test]
  fn parse_args_rejects_dataset_without_setup() {
    let error = super::parse_args_from([
      "--url=postgresql:///hyphen?host=/tmp".to_owned(),
      "--dataset=large".to_owned(),
    ])
    .expect_err("dataset without setup should fail");

    assert_eq!(
      error,
      "benchmark argument --dataset=... is only valid with --setup"
    );
  }
}
