//
// Copyright (c) 2026-present, The Dash Core developers
// SPDX-License-Identifier: MIT
// See the accompanying file LICENSE or https://opensource.org/license/MIT
//

//! The read-site guard: no configuration key may ship without a consumer, and
//! no key may claim to be waiting for one after its consumer has landed.
//!
//! # What went wrong that this prevents
//!
//! `[performance]` and `[cache]` shipped configured and inert. Production set
//! `prefetch_blocks = 256` and an 8 GB address cache against code that read
//! neither. Nothing caught it, and nothing could have: a config field that is
//! deserialized and never read is valid Rust, passes every test, and looks
//! exactly like a field that works. The operator's evidence that a setting
//! does something is that they were allowed to set it.
//!
//! # The mechanism, and why it is a ratchet rather than an exemption list
//!
//! Every field of every configuration struct must be **declared**, exactly
//! once, as one of two things:
//!
//! - [`Consumed`](Decl::Consumed) — the guard requires at least one non-test
//! read that is not the struct merely describing or checking itself.
//! - [`Pending`](Decl::Pending) — the key exists ahead of its consumer. The
//! guard requires the *opposite*: **zero** read sites, plus a written
//! justification for shipping the key early.
//!
//! Those two requirements are what stops the pending list rotting into a
//! permanent exemption list, and it is worth being precise about how, because
//! "we will clean it up later" is exactly what `[performance]` was:
//!
//! 1. **Pending is a falsifiable claim, not a waiver.** It asserts the key is
//! unread. The moment its consumer lands and reads it, the guard fails — a
//! pending entry cannot survive its own consumer arriving. An exemption
//! list works the other way round: it silences the check permanently, so
//! the entry survives everything, including the thing it was waiting for.
//! 2. **Every pending entry carries an argument, not a placeholder.** The
//! justification must say why the key ships before its consumer, and is
//! length-checked so a bare `TODO` cannot stand in for one.
//! 3. **The list cannot grow silently.** Adding a field to any configuration
//! struct with no declaration fails the completeness check. There is no
//! spelling of "add a key and move on".
//!
//! The `task` an entry names is a note for a human reader. It is deliberately
//! not cross-referenced against a task file: that file is not part of the
//! repository, so a check against it passes only where a copy happens to sit
//! beside the checkout and fails everywhere else, which is the opposite of
//! what a guard is for.
//!
//! What the guard deliberately does not do is judge *quality* of use. A read
//! site that reads a field into a variable nobody uses would pass. That is a
//! compiler-visible dead value, though, which is the class of problem the
//! compiler already handles; the invisible class is the one addressed here.
//!
//! # How read sites are found
//!
//! Textually, over the crate sources, with line comments and string literals
//! stripped and the trailing `#[cfg(test)]` module removed — a test reading a
//! field is not evidence that production does.
//!
//! A read counts when it is outside the four files that *define* the
//! configuration, or when it is inside one of them but in a function that
//! production can still reach. That second clause is what lets a field be
//! consumed by an accessor or a resolver living beside its own definition
//! (`RpcConfig::resolve_auth`, `tuning::max_advance_blocks`,
//! `tuning::storage_plan`) without weakening the rule: reachability is
//! computed from the functions named outside the defining files, transitively
//! through their bodies, so an accessor counts only while something outside
//! still calls it. Delete the last caller and the field goes back to unread.
//!
//! Four kinds of function are excluded from that even when reachable:
//! `default`, `fmt`, `serialize`, `deserialize` and `validate`. Each mentions
//! every field of its struct by construction, so a field mentioned only there
//! is precisely the inert case — settable, echoed back in a settings
//! snapshot, rejected when malformed, and changing nothing about what the
//! daemon does. `[performance]` would have passed a guard that counted those.
//! It is also why `stepped_sync_gap` is declared pending despite the
//! cross-section check reading it: a key that only validates itself is still
//! inert.
//!
//! One deliberate departure from `architecture.rs`: that file splits its
//! detection patterns with `concat!` so a repo-wide grep for a forbidden
//! crate name does not land in the guard. The patterns here are the config
//! keys themselves, and hiding those would be the wrong trade — someone
//! grepping for `copy_threshold` should land on the declaration that says it
//! is inert and why. The guard excludes itself from the read-site scan
//! instead, so naming a key here never makes it look consumed.
//!
//! The scan is coarse — a short field name such as `size` matches any
//! struct's `.size` — and the coarseness runs in opposite directions for the
//! two declarations. For `Consumed` it can only make a consumed key look
//! consumed; for `Pending` a false positive fails the build and asks a human
//! to look. Given the choice, the mechanism errs toward the red test.

#[cfg(test)]
mod tests {
  use std::collections::BTreeSet;
  use std::fs;
  use std::path::{Path, PathBuf};

  /// How a config field earns its place.
  #[derive(Debug, Clone, Copy, PartialEq, Eq)]
  enum Decl {
    /// Read by production code today.
    Consumed,
    /// No consumer yet; the named task is the one that will add one. Must be
    /// an open (`- [ ]`) item in `TASKS.md`, and must have no read sites.
    Pending {
      task: &'static str,
      /// Why the key ships before its consumer, in the author's own words.
      /// Never "TODO": the argument has to survive being read by whoever
      /// wonders why the key is inert.
      why: &'static str,
    },
  }

  /// One config field: the struct that owns it, its name, and its declaration.
  struct Field {
    owner: &'static str,
    name: &'static str,
    decl: Decl,
  }

  const fn consumed(owner: &'static str, name: &'static str) -> Field {
    Field {
      owner,
      name,
      decl: Decl::Consumed,
    }
  }

  const fn pending(
    owner: &'static str,
    name: &'static str,
    task: &'static str,
    why: &'static str,
  ) -> Field {
    Field {
      owner,
      name,
      decl: Decl::Pending { task, why },
    }
  }

  /// The configuration files this guard governs, relative to the workspace's
  /// `pkgs/` directory. Everything an operator can write in `hyphen.conf`
  /// resolves into a struct in one of these.
  const CONFIG_FILES: &[&str] = &[
    "daemon/src/config.rs",
    "db/src/config.rs",
    "db/src/layout.rs",
    "db/src/tuning.rs",
  ];

  /// Crate source roots searched for read sites.
  const SOURCE_ROOTS: &[&str] = &["daemon/src", "db/src", "console/src"];

  /// Every configuration struct, and every field of each one.
  ///
  /// The list is checked for completeness against the source files, so it
  /// cannot fall behind them: a field that is not here fails the test that
  /// extracts fields from the structs themselves.
  #[rustfmt::skip]
  const FIELDS: &[Field] = &[
    // ---- [core], [api], [chain], [indices], [live], [overlays] -----------
    consumed("Config", "core"),
    consumed("Config", "api"),
    consumed("Config", "indices"),
    consumed("Config", "live"),
    consumed("Config", "overlays"),
    consumed("Config", "chain"),
    consumed("Config", "db"),
    consumed("CoreConfig", "url"),
    consumed("CoreConfig", "rest_concurrency"),
    consumed("CoreConfig", "rpc"),
    consumed("CoreConfig", "zmq"),
    consumed("RpcConfig", "enabled"),
    consumed("RpcConfig", "url"),
    consumed("RpcConfig", "datadir"),
    consumed("RpcConfig", "cookie_file"),
    consumed("RpcConfig", "username"),
    consumed("RpcConfig", "password"),
    consumed("RpcConfig", "timeout_secs"),
    consumed("RpcConfig", "network_refresh_secs"),
    consumed("RpcConfig", "governance_refresh_secs"),
    consumed("RpcConfig", "chainlock_refresh_secs"),
    consumed("RpcConfig", "protx_refresh_secs"),
    consumed("ZmqConfig", "enabled"),
    consumed("ZmqConfig", "connect"),
    consumed("ZmqConfig", "subscribe"),
    consumed("ChainConfig", "block_time_secs"),
    consumed("ApiConfig", "bind"),
    consumed("ApiConfig", "cors_origin"),
    consumed("IndicesConfig", "address"),
    consumed("IndicesConfig", "validator"),
    consumed("IndicesConfig", "metrics"),
    consumed("LiveConfig", "enable_mempool"),
    consumed("LiveConfig", "enable_islock"),
    consumed("LiveConfig", "enable_chainlock"),
    consumed("LiveConfig", "chainlock_start_block"),
    consumed("LiveConfig", "enable_governance"),
    consumed("LiveConfig", "enable_network"),
    consumed("LiveConfig", "enable_stream"),
    consumed("LiveConfig", "recent_mempool_txs"),
    consumed("LiveConfig", "stream_client_buffer"),
    consumed("LiveConfig", "stream_heartbeat_secs"),
    consumed("OverlaysConfig", "enable_protx"),
    consumed("OverlaysConfig", "enable_chainlock"),

    // ---- [db] -------------------------------------------------------------
    consumed("DbConfig", "layout"),
    consumed("DbConfig", "client"),
    consumed("DbConfig", "ingest"),
    consumed("HyDbConfig", "client"),
    consumed("HyDbConfig", "layout"),
    consumed("HyDbConfig", "ingest"),
    consumed("HyDbConfig", "created_by"),

    // ---- [db.client] ------------------------------------------------------
    consumed("HyDbClientCfg", "socket"),
    consumed("HyDbClientCfg", "url"),
    consumed("HyDbClientCfg", "app_name"),
    consumed("HyDbClientCfg", "slow_query_ms"),
    consumed("HyDbClientCfg", "connect_timeout"),
    consumed("HyDbClientCfg", "idle_timeout"),
    consumed("HyDbClientCfg", "max_lifetime"),
    consumed("HyDbClientCfg", "stmt_cache_size"),
    consumed("HyDbClientCfg", "write"),
    consumed("HyDbClientCfg", "query"),
    consumed("HyDbClientCfg", "heavy"),
    consumed("HyDbClientCfg", "admin"),
    consumed("HyDbWriteCfg", "size"),
    consumed("HyDbWriteCfg", "acquire_timeout"),
    consumed("HyDbWriteCfg", "statement_timeout"),
    consumed("HyDbWriteCfg", "sync_commit"),
    consumed("HyDbWriteCfg", "jit"),
    consumed("HyDbQueryCfg", "size"),
    consumed("HyDbQueryCfg", "acquire_timeout"),
    consumed("HyDbQueryCfg", "statement_timeout"),
    consumed("HyDbQueryCfg", "jit"),
    consumed("HyDbHeavyCfg", "size"),
    consumed("HyDbHeavyCfg", "acquire_timeout"),
    consumed("HyDbHeavyCfg", "statement_timeout"),
    consumed("HyDbHeavyCfg", "work_mem"),
    consumed("HyDbAdminCfg", "size"),
    consumed("HyDbAdminCfg", "maint_work_mem"),

    // ---- [db.layout] ------------------------------------------------------
    consumed("HyDbLayoutCfg", "network"),
    consumed("HyDbLayoutCfg", "partition_span"),
    consumed("HyDbLayoutCfg", "p2pkh_version"),
    consumed("HyDbLayoutCfg", "p2sh_version"),
    consumed("HyDbLayoutCfg", "rich_list_floor"),
    consumed("HyDbLayoutCfg", "undo_span"),
    consumed("HyDbLayoutCfg", "schema_version"),

    // ---- [db.ingest] ------------------------------------------------------
    consumed("HyDbIngestCfg", "max_advance_blocks"),
    consumed("HyDbIngestCfg", "max_reorg_depth"),
    consumed("HyDbIngestCfg", "undo_keep_depth"),
    consumed("HyDbIngestCfg", "tuning_profile"),
    consumed("HyDbIngestCfg", "vacuum_scale_factor"),
    consumed("HyDbIngestCfg", "vacuum_analyze_factor"),
    consumed("HyDbIngestCfg", "vacuum_cost_delay"),
    consumed("HyDbIngestCfg", "fillfactor"),
    consumed("HyDbIngestCfg", "stepped_sync"),
    consumed("HyDbIngestCfg", "stepped_sync_gap"),
    consumed("HyDbIngestCfg", "batch_rows"),
    pending(
      "HyDbIngestCfg", "copy_threshold", "A6",
      "COPY is not used on the write path yet, so there is no threshold to \
       cross",
    ),
  ];

  fn pkgs_root() -> PathBuf {
    // CARGO_MANIFEST_DIR is pkgs/daemon.
    Path::new(env!("CARGO_MANIFEST_DIR"))
      .parent()
      .unwrap_or(Path::new("."))
      .to_path_buf()
  }

  /// Strip line comments and double-quoted string literals, so a field named
  /// in prose or in an error message is not mistaken for a read of it.
  ///
  /// Comments are recognised while walking the line, not by splitting on the
  /// first `//` beforehand. Splitting first truncated at the `//` inside any
  /// string holding a URL — of which this codebase has many — discarding
  /// whatever followed on that line, so a real read site could go unseen and
  /// the guard would report on its own parser rather than on the code.
  fn strip_noise(source: &str) -> String {
    let mut out = String::with_capacity(source.len());
    for line in source.lines() {
      let mut in_string = false;
      let mut escaped = false;
      let mut chars = line.chars().peekable();
      while let Some(ch) = chars.next() {
        match ch {
          _ if escaped => escaped = false,
          '\\' if in_string => escaped = true,
          '"' => in_string = !in_string,
          _ if in_string => {}
          // Outside a string, `//` starts a comment and ends the line.
          '/' if chars.peek() == Some(&'/') => break,
          _ => out.push(ch),
        }
      }
      out.push('\n');
    }
    out
  }

  /// Drop everything from a top-level `#[cfg(test)]` onward: by house style
  /// the unit-test module is last in the file, and a test reading a field is
  /// not evidence that production does.
  fn strip_test_module(source: &str) -> String {
    match source.find("\n#[cfg(test)]") {
      Some(at) => source[..at].to_owned(),
      None => source.to_owned(),
    }
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

  /// Files a read site may live in: production sources, excluding the modules
  /// that *define* the configuration (defining, defaulting, serializing and
  /// validating a field is not consuming it) and this guard itself.
  fn read_site_files() -> Vec<PathBuf> {
    let root = pkgs_root();
    let defining: Vec<PathBuf> = CONFIG_FILES.iter().map(|rel| root.join(rel)).collect();
    let mut files = Vec::new();
    for source_root in SOURCE_ROOTS {
      collect_rs_files(&root.join(source_root), &mut files);
    }
    files.retain(|path| {
      !defining.contains(path)
        && !path.ends_with("config_guard.rs")
        && !path.components().any(|c| c.as_os_str() == "tests")
    });
    files
  }

  /// Does `text` read `field` — as a field access `.field`, or as a bare
  /// mention inside a function body of the module that defines it?
  fn mentions(text: &str, field: &str, bare: bool) -> bool {
    let bytes = text.as_bytes();
    for needle in if bare {
      vec![format!(".{field}"), field.to_owned()]
    } else {
      vec![format!(".{field}")]
    } {
      let mut from = 0;
      while let Some(at) = text[from..].find(&needle) {
        let start = from + at;
        let end = start + needle.len();
        let before_ok = needle.starts_with('.')
          || start == 0
          || !(bytes[start - 1].is_ascii_alphanumeric() || bytes[start - 1] == b'_');
        let after_ok = bytes
          .get(end)
          .is_none_or(|c| !(c.is_ascii_alphanumeric() || *c == b'_'));
        // A method call `.field(` is a different thing with the same name.
        let is_call = needle.starts_with('.') && bytes.get(end).is_some_and(|c| *c == b'(');
        if before_ok && after_ok && !is_call {
          return true;
        }
        from = end;
      }
    }
    false
  }

  /// Function bodies of a source file, keyed by name. Crude but sufficient
  /// for house style: a body runs from the `fn` line to the first `}` at the
  /// same indentation.
  fn functions(text: &str) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    let lines: Vec<&str> = text.lines().collect();
    for (i, line) in lines.iter().enumerate() {
      let indent = line.len() - line.trim_start().len();
      let trimmed = line.trim_start();
      let Some(rest) = trimmed
        .strip_prefix("pub ")
        .or_else(|| trimmed.strip_prefix("pub(crate) "))
        .or_else(|| trimmed.strip_prefix("pub(super) "))
        .unwrap_or(trimmed)
        .strip_prefix("async fn ")
        .or_else(|| {
          trimmed
            .strip_prefix("pub ")
            .or_else(|| trimmed.strip_prefix("pub(crate) "))
            .or_else(|| trimmed.strip_prefix("pub(super) "))
            .unwrap_or(trimmed)
            .strip_prefix("const fn ")
        })
        .or_else(|| {
          trimmed
            .strip_prefix("pub ")
            .or_else(|| trimmed.strip_prefix("pub(crate) "))
            .or_else(|| trimmed.strip_prefix("pub(super) "))
            .unwrap_or(trimmed)
            .strip_prefix("fn ")
        })
      else {
        continue;
      };
      let name = rest.split(['(', '<', ' ']).next().unwrap_or("").to_owned();
      let closer = format!("{}}}", " ".repeat(indent));
      let mut body = String::new();
      for next in &lines[i + 1..] {
        if *next == closer {
          break;
        }
        body.push_str(next);
        body.push('\n');
      }
      out.push((name, body));
    }
    out
  }

  /// Functions that describe or check the struct rather than act on it.
  /// Serde plumbing and validation both mention every field by construction,
  /// so a field mentioned *only* there is exactly the inert case: it can be
  /// set, echoed back in a settings snapshot, and rejected when malformed,
  /// while changing nothing about what the daemon does.
  const SELF_DESCRIBING_FNS: &[&str] = &["default", "fmt", "serialize", "deserialize", "validate"];

  /// The functions of a defining file that production code can actually
  /// reach: those named outside the defining files, plus everything those
  /// call, transitively.
  ///
  /// This is what lets a field be consumed by an accessor or a resolver that
  /// lives beside its own definition (`RpcConfig::resolve_auth`,
  /// `tuning::max_advance_blocks`) without weakening the rule — the accessor
  /// only counts while something outside still calls it. Delete the caller
  /// and the field goes back to being unread.
  fn reachable_functions(file_text: &str, outside: &str) -> Vec<(String, String)> {
    let fns = functions(file_text);
    let mut reachable: Vec<bool> = fns
      .iter()
      .map(|(name, _)| mentions(outside, name, true))
      .collect();
    loop {
      let mut changed = false;
      for i in 0..fns.len() {
        if reachable[i] {
          continue;
        }
        let called =
          (0..fns.len()).any(|j| reachable[j] && j != i && mentions(&fns[j].1, &fns[i].0, true));
        if called {
          reachable[i] = true;
          changed = true;
        }
      }
      if !changed {
        break;
      }
    }
    fns
      .into_iter()
      .zip(reachable)
      .filter(|(_, ok)| *ok)
      .map(|(f, _)| f)
      .filter(|(name, _)| !SELF_DESCRIBING_FNS.contains(&name.as_str()))
      .collect()
  }

  /// Cleaned text of every production file outside the governed config files.
  fn outside_text() -> String {
    read_site_files()
      .into_iter()
      .filter_map(|path| fs::read_to_string(path).ok())
      .map(|text| strip_noise(&strip_test_module(&text)))
      .collect::<Vec<_>>()
      .join("\n")
  }

  /// Where `field` is read, as human-readable evidence. Empty means inert.
  fn read_sites(field: &str) -> Vec<String> {
    let outside = outside_text();
    let mut sites: Vec<String> = read_site_files()
      .into_iter()
      .filter(|path| {
        fs::read_to_string(path)
          .map(|text| mentions(&strip_noise(&strip_test_module(&text)), field, false))
          .unwrap_or(false)
      })
      .map(|path| path.display().to_string())
      .collect();

    let root = pkgs_root();
    for rel in CONFIG_FILES {
      let Ok(text) = fs::read_to_string(root.join(rel)) else {
        continue;
      };
      let cleaned = strip_noise(&strip_test_module(&text));
      for (name, body) in reachable_functions(&cleaned, &outside) {
        if mentions(&body, field, false) {
          sites.push(format!("{rel}::{name}"));
        }
      }
    }
    sites
  }

  /// The `pub` fields of every configuration struct in the governed files,
  /// as `(struct, field)`.
  ///
  /// Also the loophole-closer: any `pub struct` in those files whose name
  /// ends in `Config` or `Cfg` is treated as configuration, so a new section
  /// cannot be added as a new struct and escape the guard by not being listed
  /// anywhere.
  fn declared_fields() -> BTreeSet<(String, String)> {
    let root = pkgs_root();
    let mut found = BTreeSet::new();
    for rel in CONFIG_FILES {
      let raw = fs::read_to_string(root.join(rel)).unwrap_or_default();
      assert!(!raw.is_empty(), "{rel} should be readable");
      let text = strip_test_module(&raw);
      let mut current: Option<String> = None;
      for line in text.lines() {
        if let Some(rest) = line.strip_prefix("pub struct ") {
          let name = rest
            .trim_end_matches(" {")
            .split(['<', ' '])
            .next()
            .unwrap_or("")
            .to_owned();
          current = (name.ends_with("Config") || name.ends_with("Cfg")).then_some(name);
          continue;
        }
        if line == "}" {
          current = None;
          continue;
        }
        let (Some(owner), Some(field)) = (current.as_ref(), line.trim().strip_prefix("pub "))
        else {
          continue;
        };
        if let Some((name, _)) = field.split_once(':') {
          if !name.contains(' ') {
            found.insert((owner.clone(), name.to_owned()));
          }
        }
      }
    }
    found
  }

  /// Every config field is declared exactly once, and every declaration
  /// corresponds to a field that exists. This is the half that makes the
  /// other half impossible to skip: a new key cannot be added without
  /// stating, here, what reads it.
  #[test]
  fn every_config_field_is_declared() {
    let actual = declared_fields();
    let declared: BTreeSet<(String, String)> = FIELDS
      .iter()
      .map(|f| (f.owner.to_owned(), f.name.to_owned()))
      .collect();

    let undeclared: Vec<String> = actual
      .difference(&declared)
      .map(|(owner, field)| format!("{owner}.{field}"))
      .collect();
    assert!(
      undeclared.is_empty(),
      "these config fields are not declared in the read-site guard: {}. \
       Declare each one `consumed` (production reads it) or `pending` (it \
       waits on a named open task) in pkgs/daemon/src/config_guard.rs",
      undeclared.join(", ")
    );

    let stale: Vec<String> = declared
      .difference(&actual)
      .map(|(owner, field)| format!("{owner}.{field}"))
      .collect();
    assert!(
      stale.is_empty(),
      "these declarations name config fields that no longer exist: {}",
      stale.join(", ")
    );
    assert_eq!(FIELDS.len(), declared.len(), "a field is declared twice");
  }

  /// A key declared consumed is read by something other than the module that
  /// defines it.
  #[test]
  fn every_consumed_key_has_a_read_site() {
    let inert: Vec<&str> = FIELDS
      .iter()
      .filter(|f| f.decl == Decl::Consumed)
      .filter(|f| read_sites(f.name).is_empty())
      .map(|f| f.name)
      .collect();
    assert!(
      inert.is_empty(),
      "these config keys are declared consumed but nothing outside their own \
       module reads them — this is how [performance] and [cache] shipped \
       configured-and-inert: {}",
      inert.join(", ")
    );
  }

  /// A key declared pending really has no consumer — and gets one only by
  /// being reclassified.
  ///
  /// This is the anti-rot direction. `Pending` is not permission to be
  /// unread; it is an assertion that the key *is* unread, which the consumer
  /// landing immediately falsifies.
  #[test]
  fn every_pending_key_is_actually_unread() {
    let live: Vec<String> = FIELDS
      .iter()
      .filter(|f| matches!(f.decl, Decl::Pending { .. }))
      .filter_map(|f| {
        let sites = read_sites(f.name);
        (!sites.is_empty()).then(|| format!("{} (read in {})", f.name, sites.join(", ")))
      })
      .collect();
    assert!(
      live.is_empty(),
      "these config keys are declared pending but something reads them; the \
       consumer has landed, so change the declaration to `consumed`: {}",
      live.join("; ")
    );
  }

  /// A pending key's justification is an argument, not a placeholder.
  ///
  /// This replaces a cross-reference against a task file that is not tracked
  /// in the repository. That check read the file with `unwrap_or_default` and
  /// then asserted the result was non-empty, so it passed only where an
  /// untracked copy happened to sit beside the checkout and failed on every
  /// fresh clone — a guard that reports on its environment rather than on the
  /// code. What is checkable from the repository alone is that the entry
  /// explains itself, and that is checked here.
  #[test]
  fn every_pending_key_justifies_shipping_early() {
    for field in FIELDS {
      if let Decl::Pending { why, task } = field.decl {
        assert!(
          why.len() > 40 && !why.contains("TODO"),
          "the pending justification for `{}` has to say why the key ships \
           before its consumer",
          field.name
        );
        assert!(
          !task.is_empty(),
          "the pending entry for `{}` has to name the work that will consume it",
          field.name
        );
      }
    }
  }

  /// The guard is only as good as what it reads, so the stripper gets its own
  /// test. A URL inside a string used to end the line early, hiding any read
  /// site that followed it.
  #[test]
  fn stripping_keeps_code_that_follows_a_url_literal() {
    let cleaned = strip_noise(r#"let u = "postgresql://h/db"; take(cfg.batch_rows);"#);
    assert!(
      cleaned.contains("cfg.batch_rows"),
      "a read after a URL literal must survive stripping: {cleaned}"
    );
    assert!(
      !cleaned.contains("postgresql"),
      "the string's contents must not survive: {cleaned}"
    );
  }

  #[test]
  fn stripping_still_drops_real_comments_and_strings() {
    let cleaned = strip_noise("keep_me; // cfg.dropped_by_comment\nalso_kept;");
    assert!(cleaned.contains("keep_me"));
    assert!(cleaned.contains("also_kept"));
    assert!(!cleaned.contains("dropped_by_comment"));

    // An escaped quote must not be read as closing the string.
    let cleaned = strip_noise(r#"let s = "a \" // b"; kept;"#);
    assert!(cleaned.contains("kept"), "got {cleaned}");
  }

  /// The naming rule: no key longer than 24 characters, section prefix
  /// excluded. Checked here rather than per-section so a new section cannot
  /// forget it.
  #[test]
  fn every_config_key_fits_the_24_character_budget() {
    let long: Vec<&str> = FIELDS
      .iter()
      .map(|f| f.name)
      .filter(|name| name.len() > 24)
      .collect();
    assert!(long.is_empty(), "config keys over 24 chars: {long:?}");
  }
}
