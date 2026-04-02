import { genericPayload, isMeaningfulPayloadString } from "./types";
import type { TransactionPayloadField, TransactionRenderer } from "./types";
import { formatLocaleNumber } from "../data/format";

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function renderCoinbasePayload(tx: Parameters<TransactionRenderer["matches"]>[0]): readonly TransactionPayloadField[] {
  if (!isRecord(tx.payload)) {
    return genericPayload(tx);
  }

  const fields: TransactionPayloadField[] = [];

  if (typeof tx.payload["height"] === "number") {
    fields.push({ label: "Height", value: formatLocaleNumber(tx.payload["height"]) });
  }

  if (isMeaningfulPayloadString(tx.payload["merkleRootMnList"])) {
    fields.push({ label: "MN List Merkle", value: tx.payload["merkleRootMnList"], mono: true });
  }

  if (isMeaningfulPayloadString(tx.payload["merkleRootQuorums"])) {
    fields.push({ label: "Quorums Merkle", value: tx.payload["merkleRootQuorums"], mono: true });
  }

  if (typeof tx.payload["version"] === "number") {
    fields.push({ label: "Version", value: String(tx.payload["version"]) });
  }

  if (typeof tx.payload["bestClHeightDiff"] === "number") {
    fields.push({ label: "Best CL Height Diff", value: formatLocaleNumber(tx.payload["bestClHeightDiff"]) });
  }

  if (isMeaningfulPayloadString(tx.payload["bestClSignature"])) {
    fields.push({ label: "Best CL Signature", value: tx.payload["bestClSignature"], mono: true });
  }

  if (typeof tx.payload["creditPoolBalance"] === "number") {
    fields.push({ label: "Credit Pool", value: String(tx.payload["creditPoolBalance"]) });
  }

  return fields.length > 0 ? fields : genericPayload(tx);
}

export const coinbaseRenderer: TransactionRenderer = {
  type: "coinbase",
  label: "Reward",
  tone: "coinbase",
  renderMode: "both",
  matches: (tx): boolean => tx.txType === "coinbase",
  parsePayload: renderCoinbasePayload,
};
