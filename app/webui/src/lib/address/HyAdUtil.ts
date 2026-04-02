import { tr } from "@repo/hyphen-i18n";

/** Formats an address transaction count for panel metadata. */
export function HyAdFormatTransactionCount(count: number): string {
  return tr("{count, plural, one {# transaction} other {# transactions}}", { count });
}
