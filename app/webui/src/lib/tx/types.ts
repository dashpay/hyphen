import type { ApiTxDetail } from "../api";
import type { TransactionTone } from "./display";

export type TransactionRenderMode = "payment" | "payload" | "both";

export interface TransactionPayloadField {
  readonly label: string;
  readonly value: string;
  readonly mono?: boolean;
  readonly muted?: boolean;
}

export interface TransactionRenderer {
  readonly type: string;
  readonly label: string;
  readonly tone: TransactionTone;
  readonly renderMode: TransactionRenderMode;
  readonly matches: (tx: ApiTxDetail) => boolean;
  readonly parsePayload?: (tx: ApiTxDetail) => readonly TransactionPayloadField[];
}

function isZeroString(value: string): boolean {
  return /^0+$/u.test(value);
}

/** Filters payload values that should not surface as visible fields. */
export function isMeaningfulPayloadValue(value: unknown): boolean {
  if (value == null) {
    return false;
  }

  if (typeof value === "string") {
    const trimmed = value.trim();

    if (trimmed.length === 0) {
      return false;
    }

    if (isZeroString(trimmed)) {
      return false;
    }
  }

  return true;
}

/** Narrows one payload value to a visible non-empty string. */
export function isMeaningfulPayloadString(value: unknown): value is string {
  return typeof value === "string" && isMeaningfulPayloadValue(value);
}

function stringifyPayloadValue(value: unknown): string {
  if (typeof value === "string") return value;
  if (typeof value === "number" || typeof value === "boolean" || typeof value === "bigint") return String(value);
  if (value == null) return String(value);
  return JSON.stringify(value);
}

/** Falls back to a flat key-value payload view for unknown tx types. */
export function genericPayload(tx: ApiTxDetail): readonly TransactionPayloadField[] {
  if (tx.payload == null || typeof tx.payload !== "object") {
    return [];
  }

  return Object.entries(tx.payload)
    .filter(([, value]): boolean => isMeaningfulPayloadValue(value))
    .map(([key, value]) => ({
      label: key,
      value: stringifyPayloadValue(value),
      mono: typeof value === "string" && (value.length > 20 || /^[0-9a-f]+$/iu.test(value)),
    }));
}
