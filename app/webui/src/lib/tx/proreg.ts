import { genericPayload, isMeaningfulPayloadString } from "./types";
import type { TransactionPayloadField, TransactionRenderer } from "./types";

function isRecord(value: unknown): value is Record<string, unknown> {
  return value != null && typeof value === "object";
}

function renderProviderRegisterPayload(
  tx: Parameters<TransactionRenderer["matches"]>[0]
): readonly TransactionPayloadField[] {
  if (!isRecord(tx.payload)) {
    return genericPayload(tx);
  }

  const fields: TransactionPayloadField[] = [];

  if (typeof tx.payload["version"] === "number") {
    fields.push({ label: "Version", value: String(tx.payload["version"]) });
  }

  if (isMeaningfulPayloadString(tx.payload["mnType"])) {
    fields.push({ label: "Type", value: tx.payload["mnType"] });
  }

  if (isMeaningfulPayloadString(tx.payload["collateralHash"])) {
    fields.push({ label: "Collateral Hash", value: tx.payload["collateralHash"], mono: true });
  }

  if (typeof tx.payload["collateralIndex"] === "number") {
    fields.push({ label: "Collateral Index", value: String(tx.payload["collateralIndex"]) });
  }

  if (isMeaningfulPayloadString(tx.payload["ownerKeyId"])) {
    fields.push({ label: "Owner Key ID", value: tx.payload["ownerKeyId"], mono: true });
  }

  if (isMeaningfulPayloadString(tx.payload["operatorPubKey"])) {
    fields.push({ label: "Operator Pub Key", value: tx.payload["operatorPubKey"], mono: true });
  }

  if (isMeaningfulPayloadString(tx.payload["votingKeyId"])) {
    fields.push({ label: "Voting Key ID", value: tx.payload["votingKeyId"], mono: true });
  }

  if (typeof tx.payload["operatorReward"] === "number") {
    fields.push({ label: "Operator Reward", value: (tx.payload["operatorReward"] / 100).toFixed(2) + "%" });
  }

  return fields.length > 0 ? fields : genericPayload(tx);
}

export const proRegRenderer: TransactionRenderer = {
  type: "provider_register",
  label: "Register",
  tone: "proreg",
  renderMode: "payload",
  matches: (tx): boolean => tx.txType === "provider_register",
  parsePayload: renderProviderRegisterPayload,
};
