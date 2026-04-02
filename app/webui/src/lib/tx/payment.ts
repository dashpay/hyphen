import type { TransactionRenderer } from "./types";

export const paymentRenderer: TransactionRenderer = {
  type: "normal",
  label: "Payment",
  tone: "default",
  renderMode: "payment",
  matches: (tx): boolean => tx.txType === "normal",
};
