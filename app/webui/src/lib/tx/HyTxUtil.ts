import { tr } from "@repo/hyphen-i18n";

import type { ApiTxDetail } from "$lib/api";

import type { TransactionTone } from "./display";

export interface HyTxExpandableRow {
  readonly amountPrefix?: string;
  readonly amountTone?: "default" | "negative" | "positive";
  readonly label: string;
  readonly metaLabel: string | null;
  readonly metaValue: string;
  readonly size: number;
  readonly tone: TransactionTone;
  readonly txid: string;
  readonly valueOut: number;
}

/** Formats one address-side preview for compact transaction rows. */
export function HyTxFormatAddressSide(addresses: readonly string[]): string | null {
  const [first] = addresses;

  if (first == null) {
    return null;
  }

  if (addresses.length === 1) {
    return first;
  }

  return first + " (and " + String(addresses.length - 1) + " more)";
}

/** Formats one compact from-to preview for payment-like rows. */
export function HyTxFormatTransactionFlow(transaction: ApiTxDetail | null): string | null {
  if (transaction == null) {
    return null;
  }

  const isCoinbase = transaction.blockPos === 0 || transaction.vin.some((input): boolean => input.coinbase != null);
  const isPayment =
    transaction.txType === "normal" || transaction.txType === "payment" || transaction.txType === "standard";

  if (!isCoinbase && !isPayment) {
    return null;
  }

  const fromAddresses = transaction.vin
    .map((input): string | null => {
      if (input.coinbase != null) {
        return tr("Reward");
      }

      return input.address;
    })
    .filter((value): value is string => value != null);
  const toAddresses = transaction.vout
    .map((output): string | null => output.address)
    .filter((value): value is string => value != null);
  const from = HyTxFormatAddressSide([...new Set(fromAddresses)]);
  const to = HyTxFormatAddressSide([...new Set(toAddresses)]);

  if (from == null || to == null) {
    return null;
  }

  return from + " > " + to;
}
