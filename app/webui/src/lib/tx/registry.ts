import type { ApiTxDetail } from "../api";
import { assetLockRenderer } from "./assetlock";
import { assetUnlockRenderer } from "./assetunlock";
import { coinbaseRenderer } from "./coinbase";
import { dataRenderer } from "./data";
import { mnhfRenderer } from "./mnhf";
import { paymentRenderer } from "./payment";
import { proRegRenderer } from "./proreg";
import { proUpRegRenderer } from "./proupreg";
import { proUpRevRenderer } from "./prouprev";
import { proUpServRenderer } from "./proupserv";
import { quorumRenderer } from "./quorum";
import type { TransactionRenderer } from "./types";

function isCoinbaseTransaction(tx: ApiTxDetail): boolean {
  return tx.blockPos === 0 || tx.vin.some((input): boolean => input.coinbase != null);
}

function toTitleCase(value: string): string {
  return value
    .split(/[_\s-]+/u)
    .filter((part): boolean => part.length > 0)
    .map((part): string => part.slice(0, 1).toUpperCase() + part.slice(1).toLowerCase())
    .join(" ");
}

const defaultRenderer: TransactionRenderer = {
  type: "default",
  label: "Transaction",
  tone: "default",
  renderMode: "payment",
  matches: (): boolean => true,
};

function inferRenderMode(tx: ApiTxDetail): TransactionRenderer["renderMode"] {
  if (tx.payload != null) {
    return "payload";
  }

  if (isCoinbaseTransaction(tx)) {
    return "both";
  }

  return "payment";
}

const renderers: readonly TransactionRenderer[] = [
  assetLockRenderer,
  assetUnlockRenderer,
  coinbaseRenderer,
  dataRenderer,
  mnhfRenderer,
  paymentRenderer,
  proRegRenderer,
  proUpRegRenderer,
  proUpRevRenderer,
  proUpServRenderer,
  quorumRenderer,
  defaultRenderer,
];

/** Selects the renderer that owns one transaction detail payload. */
export function getTransactionRenderer(tx: ApiTxDetail): TransactionRenderer {
  if (isCoinbaseTransaction(tx)) {
    return coinbaseRenderer;
  }

  for (const renderer of renderers) {
    if (renderer.matches(tx)) {
      if (renderer === defaultRenderer) {
        return {
          ...defaultRenderer,
          type: tx.txType,
          label: toTitleCase(tx.txType),
          tone: "default",
          renderMode: inferRenderMode(tx),
        };
      }

      return renderer;
    }
  }

  return {
    ...defaultRenderer,
    type: tx.txType,
    label: toTitleCase(tx.txType),
    tone: "default",
    renderMode: inferRenderMode(tx),
  };
}
