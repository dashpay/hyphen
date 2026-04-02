<script lang="ts">
  /** Summary panel for the current mempool snapshot. */
  import { tr } from "@repo/hyphen-i18n";
  import { VwText } from "@repo/hyphen-views";

  import type { ApiMempoolSummary } from "$lib";
  import { HyUtilFormatBytes, HyUtilFormatFullDate, HyUtilFormatNumber } from "$lib/HyUtil";
  import { HyMpFormatTransactionCount, HyMpFormatUsageSummary } from "$lib/mempool/HyMpUtil";
  import { HyListPanel, HyMetricCard, HyMetricGrid, HyScrollBody } from "$lib/views";

  interface Props {
    readonly summary: ApiMempoolSummary;
  }

  const { summary }: Props = $props();
</script>

<HyListPanel subtitle={HyMpFormatTransactionCount(summary.count)} title="Mempool">
  <HyScrollBody container={true}>
    <div class="summary-body">
      <HyMetricGrid columns={2}>
        <HyMetricCard tone="accent">
          <VwText color="muted" size="xs" weight="bold">{tr("Transactions")}</VwText>
          <VwText as="span" size="lg" weight="extrabold">{HyUtilFormatNumber(summary.count)}</VwText>
        </HyMetricCard>
        <HyMetricCard>
          <VwText color="muted" size="xs" weight="bold">{tr("Total Size")}</VwText>
          <VwText as="span" size="lg" weight="extrabold">{HyUtilFormatBytes(summary.totalBytes)}</VwText>
        </HyMetricCard>
        <HyMetricCard>
          <VwText color="muted" size="xs" weight="bold">{tr("Memory Usage")}</VwText>
          <VwText as="span" size="lg" weight="extrabold"
            >{HyUtilFormatBytes(summary.usageBytes ?? summary.totalBytes)}</VwText
          >
        </HyMetricCard>
        <HyMetricCard>
          <VwText color="muted" size="xs" weight="bold">{tr("Capacity")}</VwText>
          <VwText as="span" size="lg" weight="extrabold">{HyMpFormatUsageSummary(summary)}</VwText>
        </HyMetricCard>
      </HyMetricGrid>

      <div class="detail-copy">
        <VwText color="muted" size="sm">
          {tr("Updated {}", HyUtilFormatFullDate(summary.updatedAt))}
        </VwText>
      </div>
    </div>
  </HyScrollBody>
</HyListPanel>

<style>
  .summary-body {
    display: grid;
    gap: var(--hy-browser-gap);
    padding: var(--hy-detail-section-padding);
  }

  .detail-copy {
    padding: 0 var(--hy-detail-chip-padding);
  }
</style>
