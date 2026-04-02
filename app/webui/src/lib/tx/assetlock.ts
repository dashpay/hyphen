import { genericPayload, isMeaningfulPayloadString } from "./types";
import type { TransactionPayloadField, TransactionRenderer } from "./types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function renderAssetLockPayload(tx: Parameters<TransactionRenderer["matches"]>[0]): readonly TransactionPayloadField[] {
  if (!isRecord(tx.payload)) {
    return genericPayload(tx);
  }

  const fields: TransactionPayloadField[] = [];

  if (typeof tx.payload["version"] === "number") {
    fields.push({ label: "Version", value: String(tx.payload["version"]) });
  }

  if (Array.isArray(tx.payload["creditOutputs"])) {
    tx.payload["creditOutputs"].forEach((output, index): void => {
      if (output == null || typeof output !== "object") {
        return;
      }

      const creditOutput = output as Record<string, unknown>;
      if (typeof creditOutput["value"] === "number") {
        fields.push({
          label: "Credit Output " + String(index + 1) + " Value",
          value: String(creditOutput["value"]),
        });
      }

      if (isMeaningfulPayloadString(creditOutput["scriptPubKey"])) {
        fields.push({
          label: "Credit Output " + String(index + 1) + " Script",
          value: creditOutput["scriptPubKey"],
          mono: true,
        });
      }
    });
  }

  return fields.length > 0 ? fields : genericPayload(tx);
}

export const assetLockRenderer: TransactionRenderer = {
  type: "asset_lock",
  label: "Lock",
  tone: "assetlock",
  renderMode: "payload",
  matches: (tx): boolean => tx.txType === "asset_lock",
  parsePayload: renderAssetLockPayload,
};
