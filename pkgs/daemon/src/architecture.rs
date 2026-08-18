//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! Architecture boundary tests for the daemon crate.

#[cfg(test)]
mod tests {
  use std::fs;
  use std::path::{Path, PathBuf};

  const PRIMITIVE_PATTERNS: &[&str] = &[
    "sha2::",
    "ripemd::",
    "bs58::",
    "bitcoin_io::Cursor",
    "consensus_decode_tx(",
    "consensus_decode_block(",
    "read_compact_size(",
    "read_compact_bytes(",
    "write_compact_size(",
    "double_sha256(",
  ];

  // Crate-level references to the retired storage backends. The daemon owns no
  // SQL or RocksDB storage anymore — all chain data lives behind `hyphen-db` —
  // so these crates must not reappear in daemon source. The pattern strings are
  // assembled with `concat!` so this guard file does not itself contain the
  // literal crate names (keeping repo-wide substring greps clean).
  const RETIRED_STORAGE_PATTERNS: &[&str] = &[
    concat!("use sq", "lx"),
    concat!("sq", "lx::"),
    concat!("use rocks", "db"),
    concat!("rocks", "db::"),
  ];

  #[test]
  fn runtime_primitive_work_stays_inside_primitives() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&root, &mut files);

    let offenders: Vec<String> = files
      .into_iter()
      .filter(|path| file_contains_primitive_work(path))
      .filter(|path| !allowed_primitive_file(path))
      .map(|path| path.display().to_string())
      .collect();

    assert!(
      offenders.is_empty(),
      "runtime primitive work must stay inside dash-primitives; test-only exceptions must be reviewed explicitly: {}",
      offenders.join(", ")
    );
  }

  #[test]
  fn no_retired_storage_backends_in_daemon_source() {
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut files = Vec::new();
    collect_rs_files(&root, &mut files);

    let offenders: Vec<String> = files
      .into_iter()
      // This file names the patterns in string literals, so exempt it.
      .filter(|path| !allowed_primitive_file(path))
      .filter(|path| {
        fs::read_to_string(path).is_ok_and(|contents| {
          RETIRED_STORAGE_PATTERNS
            .iter()
            .any(|pattern| contents.contains(pattern))
        })
      })
      .map(|path| path.display().to_string())
      .collect();

    assert!(
      offenders.is_empty(),
      "the retired SQL and RocksDB backends are gone; all storage must go through hyphen-db: {}",
      offenders.join(", ")
    );
  }

  fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(root) else {
      return;
    };
    for entry in entries.flatten() {
      let path = entry.path();
      if path.is_dir() {
        collect_rs_files(&path, out);
      } else if path.extension().is_some_and(|ext| ext == "rs") {
        out.push(path);
      }
    }
  }

  fn file_contains_primitive_work(path: &Path) -> bool {
    let Ok(contents) = fs::read_to_string(path) else {
      return false;
    };
    PRIMITIVE_PATTERNS
      .iter()
      .any(|pattern| contents.contains(pattern))
  }

  fn allowed_primitive_file(path: &Path) -> bool {
    let rel = path
      .strip_prefix(Path::new(env!("CARGO_MANIFEST_DIR")).join("src"))
      .unwrap_or(path);
    matches!(rel.to_str(), Some("architecture.rs"))
  }
}
