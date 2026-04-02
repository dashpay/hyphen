import { genericPayload, isMeaningfulPayloadString } from "./types";
import type { TransactionPayloadField, TransactionRenderer } from "./types";
import { formatLocaleNumber } from "../data/format";

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function renderQuorumPayload(tx: Parameters<TransactionRenderer["matches"]>[0]): readonly TransactionPayloadField[] {
  if (!isRecord(tx.payload)) {
    return genericPayload(tx);
  }

  const fields: TransactionPayloadField[] = [];

  if (typeof tx.payload["version"] === "number") {
    fields.push({ label: "Version", value: String(tx.payload["version"]) });
  }

  if (typeof tx.payload["height"] === "number") {
    fields.push({ label: "Height", value: formatLocaleNumber(tx.payload["height"]) });
  }

  if (typeof tx.payload["commitmentVersion"] === "number") {
    fields.push({ label: "Commitment Version", value: String(tx.payload["commitmentVersion"]) });
  }

  if (isMeaningfulPayloadString(tx.payload["llmqType"])) {
    fields.push({ label: "LLMQ Type", value: tx.payload["llmqType"] });
  }

  if (isMeaningfulPayloadString(tx.payload["quorumHash"])) {
    fields.push({ label: "Quorum Hash", value: tx.payload["quorumHash"], mono: true });
  }

  if (typeof tx.payload["quorumIndex"] === "number") {
    fields.push({ label: "Quorum Index", value: String(tx.payload["quorumIndex"]) });
  }

  return fields.length > 0 ? fields : genericPayload(tx);
}

export const quorumRenderer: TransactionRenderer = {
  type: "quorum_commitment",
  label: "Commit",
  tone: "quorum",
  renderMode: "payload",
  matches: (tx): boolean => tx.txType === "quorum_commitment",
  parsePayload: renderQuorumPayload,
};
