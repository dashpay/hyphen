import { genericPayload, isMeaningfulPayloadString } from "./types";
import type { TransactionPayloadField, TransactionRenderer } from "./types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function renderMnhfPayload(tx: Parameters<TransactionRenderer["matches"]>[0]): readonly TransactionPayloadField[] {
  if (!isRecord(tx.payload)) {
    return genericPayload(tx);
  }

  const fields: TransactionPayloadField[] = [];

  if (typeof tx.payload["version"] === "number") {
    fields.push({ label: "Version", value: String(tx.payload["version"]) });
  }

  if (typeof tx.payload["versionBit"] === "number") {
    fields.push({ label: "Version Bit", value: String(tx.payload["versionBit"]) });
  }

  if (isMeaningfulPayloadString(tx.payload["quorumHash"])) {
    fields.push({ label: "Quorum Hash", value: tx.payload["quorumHash"], mono: true });
  }

  return fields.length > 0 ? fields : genericPayload(tx);
}

export const mnhfRenderer: TransactionRenderer = {
  type: "mnhf",
  label: "ForkSig",
  tone: "mnhf",
  renderMode: "payload",
  matches: (tx): boolean => tx.txType === "mnhf",
  parsePayload: renderMnhfPayload,
};
