import { genericPayload, isMeaningfulPayloadString } from "./types";
import type { TransactionPayloadField, TransactionRenderer } from "./types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function renderProviderUpdateRevokePayload(
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

  if (isMeaningfulPayloadString(tx.payload["reason"])) {
    fields.push({ label: "Reason", value: tx.payload["reason"] });
  }

  return fields.length > 0 ? fields : genericPayload(tx);
}

export const proUpRevRenderer: TransactionRenderer = {
  type: "provider_update_revoke",
  label: "Revoke",
  tone: "prouprev",
  renderMode: "payload",
  matches: (tx): boolean => tx.txType === "provider_update_revoke",
  parsePayload: renderProviderUpdateRevokePayload,
};
