import { genericPayload, isMeaningfulPayloadString } from "./types";
import type { TransactionPayloadField, TransactionRenderer } from "./types";
import { formatLocaleNumber } from "../data/format";

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function renderAssetUnlockPayload(
  tx: Parameters<TransactionRenderer["matches"]>[0]
): readonly TransactionPayloadField[] {
  if (!isRecord(tx.payload)) {
    return genericPayload(tx);
  }

  const fields: TransactionPayloadField[] = [];

  if (typeof tx.payload["version"] === "number") {
    fields.push({ label: "Version", value: String(tx.payload["version"]) });
  }

  if (typeof tx.payload["index"] === "number") {
    fields.push({ label: "Index", value: String(tx.payload["index"]) });
  }

  if (typeof tx.payload["fee"] === "number") {
    fields.push({ label: "Fee", value: String(tx.payload["fee"]) });
  }

  if (typeof tx.payload["requestedHeight"] === "number") {
    fields.push({ label: "Requested Height", value: formatLocaleNumber(tx.payload["requestedHeight"]) });
  }

  if (isMeaningfulPayloadString(tx.payload["quorumHash"])) {
    fields.push({ label: "Quorum Hash", value: tx.payload["quorumHash"], mono: true });
  }

  return fields.length > 0 ? fields : genericPayload(tx);
}

export const assetUnlockRenderer: TransactionRenderer = {
  type: "asset_unlock",
  label: "Unlock",
  tone: "assetunlock",
  renderMode: "payload",
  matches: (tx): boolean => tx.txType === "asset_unlock",
  parsePayload: renderAssetUnlockPayload,
};
