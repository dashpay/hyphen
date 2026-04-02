#!/usr/bin/env python3
# coding: latin-1

# Copyright (c) 2026-present, Kittywhiskers Van Gogh
# SPDX-License-Identifier: MIT
# See the accompanying file LICENSE or https://opensource.org/license/MIT

from __future__ import annotations

import argparse
import shutil
import subprocess
import sys
from pathlib import Path

DEFAULT_CONFIGS: tuple[str, ...] = (
  "rust_general.yml",
  "svelte_general.yml",
  "ts_general.yml",
)


def find_repo_root(start: Path) -> Path:
  """Walk upward from *start* until a pyproject.toml file is found."""
  for directory in (start, *start.parents):
    if (directory / "pyproject.toml").is_file():
      return directory

  raise FileNotFoundError("pyproject.toml not found")


def build_command(
  semgrep_bin: str,
  config_dir: Path,
  configs: tuple[str, ...],
  targets: tuple[Path, ...],
) -> list[str]:
  """Build the semgrep CLI command for the selected configs and targets."""
  command = [
    semgrep_bin,
    "scan",
    "--error",
    "--disable-version-check",
    "--metrics=off",
  ]

  for config_name in configs:
    command.extend(["--config", str(config_dir / config_name)])

  for target in targets:
    command.append(str(target))

  return command


def main() -> int:
  parser = argparse.ArgumentParser(
    description="Run the repo-local semgrep module-discipline rules.",
  )
  parser.add_argument(
    "targets",
    metavar="TARGET",
    nargs="*",
    type=Path,
    help="Optional files or directories to scan. Defaults to the repo root.",
  )
  parser.add_argument(
    "--config-dir",
    type=Path,
    help="Directory that holds the semgrep rule packs.",
  )
  parser.add_argument(
    "--config",
    dest="configs",
    action="append",
    help="Rule pack filename to run. Defaults to all repo rule packs.",
  )
  args = parser.parse_args()

  script_path = Path(__file__).resolve()
  repo_root = find_repo_root(script_path.parent)
  config_dir = args.config_dir or (repo_root / "contrib" / "r2c")
  configs = tuple(args.configs) if args.configs else DEFAULT_CONFIGS
  semgrep_bin = shutil.which("semgrep")

  if semgrep_bin is None:
    print("error: semgrep binary not found in PATH", file=sys.stderr)
    return 2

  missing_configs = [
    config_name for config_name in configs if not (config_dir / config_name).is_file()
  ]
  if missing_configs:
    print(
      "error: missing semgrep config(s): " + ", ".join(sorted(missing_configs)),
      file=sys.stderr,
    )
    return 2

  targets = (
    tuple(path.resolve() for path in args.targets) if args.targets else (repo_root,)
  )
  command = build_command(semgrep_bin, config_dir.resolve(), configs, targets)
  result = subprocess.run(command, check=False)  # noqa: S603
  return result.returncode


if __name__ == "__main__":
  sys.exit(main())
