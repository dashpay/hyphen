import { genericPayload, isMeaningfulPayloadString } from "./types";
import type { TransactionPayloadField, TransactionRenderer } from "./types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function renderProviderUpdateServicePayload(
  tx: Parameters<TransactionRenderer["matches"]>[0]
): readonly TransactionPayloadField[] {
  if (!isRecord(tx.payload)) {
    return genericPayload(tx);
  }

  const fields: TransactionPayloadField[] = [];

  if (typeof tx.payload["version"] === "number") {
    fields.push({ label: "Version", value: String(tx.payload["version"]) });
  }

  if (isMeaningfulPayloadString(tx.payload["proTxHash"])) {
    fields.push({ label: "ProTxHash", value: tx.payload["proTxHash"], mono: true });
  }

  return fields.length > 0 ? fields : genericPayload(tx);
}

export const proUpServRenderer: TransactionRenderer = {
  type: "provider_update_service",
  label: "Update",
  tone: "proupserv",
  renderMode: "payload",
  matches: (tx): boolean => tx.txType === "provider_update_service",
  parsePayload: renderProviderUpdateServicePayload,
};
