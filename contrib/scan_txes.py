#!/usr/bin/env python3
# coding: latin-1

# Copyright (c) 2026-present, Kittywhiskers Van Gogh
# SPDX-License-Identifier: MIT
# See the accompanying file LICENSE or https://opensource.org/license/MIT

from __future__ import annotations

import argparse
import json
import sys
import urllib.error
import urllib.parse
import urllib.request
from dataclasses import dataclass
from pathlib import Path
from typing import cast

_DEFAULT_BASE_URL = "http://127.0.0.1:18080"
_DEFAULT_PER_TYPE = 5
_DEFAULT_RANGE = 100_000

_TYPE_ORDER: tuple[str, ...] = (
  "coinbase",
  "cbtx",
  "data",
  "normal",
  "proregtx",
  "proupservtx",
  "proupregtx",
  "prouprevtx",
  "quorumcommitment",
  "mnhfsignal",
  "assetlocktx",
  "assetunlocktx",
)

_SPECIAL_TYPE_MAP: dict[int, str] = {
  1: "proregtx",
  2: "proupservtx",
  3: "proupregtx",
  4: "prouprevtx",
  5: "cbtx",
  6: "quorumcommitment",
  7: "mnhfsignal",
  8: "assetlocktx",
  9: "assetunlocktx",
}

JsonValue = (
  None
  | bool
  | int
  | float
  | str
  | list["JsonValue"]
  | dict[str, "JsonValue"]
)

@dataclass(frozen=True)
class ScanArgs:
  url: str
  start_block: int | None
  block_range: int
  output: Path
  max_bucket: int


class RestClient:
  def __init__(self, base_url: str) -> None:
    self._base_url = base_url.rstrip("/")

  def get_json(self, path: str) -> JsonValue:
    url = self._base_url + path
    parsed = urllib.parse.urlparse(url)
    if parsed.scheme not in {"http", "https"}:
      raise RuntimeError(f"unsupported URL scheme for {url}")

    request = urllib.request.Request(url)  # noqa: S310
    try:
      with urllib.request.urlopen(request, timeout=30) as response:  # noqa: S310
        return cast("JsonValue", json.load(response))
    except urllib.error.HTTPError as exc:
      message = exc.read().decode("utf-8", errors="replace").strip()
      raise RuntimeError(
        f"request failed for {path}: HTTP {exc.code} {message}"
      ) from exc
    except urllib.error.URLError as exc:
      raise RuntimeError(f"request failed for {path}: {exc.reason}") from exc


def parse_args() -> ScanArgs:
  parser = argparse.ArgumentParser(
    description=(
      "Scan Dash Core blocks and collect representative raw transactions by "
      "type."
    ),
  )
  parser.add_argument(
    "--url",
    default=_DEFAULT_BASE_URL,
    help=f"Dash Core REST base URL (default: {_DEFAULT_BASE_URL}).",
  )
  parser.add_argument(
    "--start-block",
    type=int,
    default=None,
    help="Block height to start scanning from. Defaults to the current chain tip.",
  )
  parser.add_argument(
    "--range",
    dest="block_range",
    type=int,
    default=_DEFAULT_RANGE,
    help=(
      "Number of blocks to scan descending from --start-block "
      f"(default: {_DEFAULT_RANGE})."
    ),
  )
  parser.add_argument(
    "--output",
    type=Path,
    default=None,
    help="Output JSON path. Defaults to contrib/scan_txes_<start>_<range>.json.",
  )
  parser.add_argument(
    "--max-bucket",
    type=int,
    default=_DEFAULT_PER_TYPE,
    help=(
      "Maximum number of transactions to keep per type "
      f"(default: {_DEFAULT_PER_TYPE})."
    ),
  )
  raw = parser.parse_args()

  if raw.block_range <= 0:
    parser.error("--range must be greater than zero")
  if raw.max_bucket <= 0:
    parser.error("--max-bucket must be greater than zero")

  output = raw.output
  if output is None:
    start_hint = "tip" if raw.start_block is None else str(raw.start_block)
    output = Path("contrib") / f"scan_txes_{start_hint}_{raw.block_range}.json"

  return ScanArgs(
    url=str(raw.url),
    start_block=raw.start_block,
    block_range=int(raw.block_range),
    output=output,
    max_bucket=int(raw.max_bucket),
  )


def read_dash_core_version() -> str | None:
  return None


def classify_transaction(tx: dict[str, JsonValue]) -> str | None:
  tx_type = int(tx.get("type", 0))
  if tx_type == 0:
    vins = tx.get("vin")
    if isinstance(vins, list) and any(
      isinstance(vin, dict) and "coinbase" in vin for vin in vins
    ):
      return "coinbase"

    vouts = tx.get("vout")
    if isinstance(vouts, list):
      for vout in vouts:
        if not isinstance(vout, dict):
          continue
        script_pub_key = vout.get("scriptPubKey")
        if (
          isinstance(script_pub_key, dict)
          and script_pub_key.get("type") == "nulldata"
        ):
          return "data"

    return "normal"

  return _SPECIAL_TYPE_MAP.get(tx_type)


def get_tip_height(client: RestClient) -> int:
  chain_info = client.get_json("/rest/chaininfo.json")
  blocks = chain_info.get("blocks")
  if not isinstance(blocks, int):
    raise RuntimeError("chaininfo response did not include an integer 'blocks' field")
  return blocks


def get_chain_network(client: RestClient) -> str:
  chain_info = client.get_json("/rest/chaininfo.json")
  chain = chain_info.get("chain")
  if not isinstance(chain, str) or len(chain) == 0:
    raise RuntimeError("chaininfo response did not include a valid 'chain' field")
  return chain


def get_block_hash(client: RestClient, height: int) -> str:
  payload = client.get_json(f"/rest/blockhashbyheight/{height}.json")
  block_hash = payload.get("blockhash")
  if not isinstance(block_hash, str) or len(block_hash) == 0:
    raise RuntimeError(
      "blockhashbyheight response did not include a valid hash "
      f"for height {height}"
    )
  return block_hash


def get_block(client: RestClient, block_hash: str) -> dict[str, JsonValue]:
  block = client.get_json(f"/rest/block/{block_hash}.json")
  if not isinstance(block, dict):
    raise RuntimeError(f"block response was not an object for hash {block_hash}")
  return block


def scan_transactions(args: ScanArgs) -> tuple[dict[str, JsonValue], int]:
  client = RestClient(args.url)
  version = read_dash_core_version()
  network = get_chain_network(client)
  tip_height = get_tip_height(client)
  start_height = (
    tip_height if args.start_block is None else min(args.start_block, tip_height)
  )
  end_height = max(0, start_height - args.block_range + 1)

  grouped: dict[str, list[dict[str, str]]] = {tx_type: [] for tx_type in _TYPE_ORDER}
  scanned_blocks = 0

  for height in range(start_height, end_height - 1, -1):
    block_hash = get_block_hash(client, height)
    block = get_block(client, block_hash)
    txs = block.get("tx")
    if not isinstance(txs, list):
      raise RuntimeError(f"block {block_hash} did not include a tx array")

    for tx in txs:
      if not isinstance(tx, dict):
        continue
      tx_class = classify_transaction(tx)
      if tx_class is None:
        continue
      entries = grouped[tx_class]
      if len(entries) >= args.max_bucket:
        continue
      txid = tx.get("txid")
      raw_hex = tx.get("hex")
      if not isinstance(txid, str) or not isinstance(raw_hex, str):
        continue
      entries.append(
        {
          "type": tx_class,
          "txid": txid,
          "raw": raw_hex,
          "block_hash": block_hash,
        }
      )

    scanned_blocks += 1
    if scanned_blocks % 1000 == 0:
      print(
        f"[scan_txes] scanned {scanned_blocks} blocks "
        f"({height}..{start_height})",
        file=sys.stderr,
      )

  return (
    {
      "info": {
        "version": version,
        "network": network,
      },
      **grouped,
    },
    scanned_blocks,
  )


def write_output(path: Path, payload: dict[str, JsonValue]) -> None:
  path.parent.mkdir(parents=True, exist_ok=True)
  path.write_text(json.dumps(payload, indent=2) + "\n", encoding="utf-8")


def build_summary(
  payload: dict[str, JsonValue],
  scanned_blocks: int,
  output_path: Path,
) -> str:
  total = 0
  parts: list[str] = []
  for tx_type in _TYPE_ORDER:
    entries = payload.get(tx_type)
    count = len(entries) if isinstance(entries, list) else 0
    total += count
    parts.append(f"{count} {tx_type}")

  return (
    f"Scanned {scanned_blocks} blocks, found {total} txes "
    f"({', '.join(parts)})\n"
    f"Saved to {output_path}"
  )


def main() -> int:
  args = parse_args()
  payload, scanned_blocks = scan_transactions(args)
  write_output(args.output, payload)
  print(build_summary(payload, scanned_blocks, args.output))
  return 0


if __name__ == "__main__":
  sys.exit(main())
