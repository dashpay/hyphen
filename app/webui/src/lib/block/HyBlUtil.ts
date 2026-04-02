import { formatLocaleNumber } from "../data/format";

/** Formats a block height label for compact row metadata. */
export function HyBlFormatHeightLabel(height: number): string {
  return "#" + formatLocaleNumber(height);
}

/** Formats a block transaction count for compact row metadata. */
export function HyBlFormatTransactionCount(count: number): string {
  return formatLocaleNumber(count) + " txns";
}
