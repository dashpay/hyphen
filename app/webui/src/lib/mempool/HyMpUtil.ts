import { tr } from "@repo/hyphen-i18n";

import { getTransactionDisplay } from "$lib/tx/display";
import type { ApiMempoolSummary, ApiTxTags } from "$lib/api";

/** Formats one mempool transaction count for panel subtitles. */
export function HyMpFormatTransactionCount(count: number): string {
  return tr("{count, plural, one {# pending} other {# pending}}", { count });
}

/** Formats one mempool capacity summary from current usage fields. */
export function HyMpFormatUsageSummary(summary: ApiMempoolSummary): string {
  if (summary.maxBytes == null || summary.maxBytes <= 0) {
    return tr("Usage limit unavailable");
  }

  const pct = (summary.usageBytes ?? summary.totalBytes) / summary.maxBytes;
  return tr("{} used", Math.max(0, pct * 100).toFixed(1) + "%");
}

/** Returns compact status labels for one mempool transaction. */
export function HyMpGetTransactionLabels(txType: string | null | undefined, tags: ApiTxTags): readonly string[] {
  const labels: string[] = [getTransactionDisplay(txType ?? "normal").label];

  if (tags.coinbase) {
    labels.unshift(tr("Reward"));
  }

  return labels;
}
