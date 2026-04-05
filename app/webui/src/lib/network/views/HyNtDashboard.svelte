<script lang="ts">
  /** Dashboard surface for live network counters and charts. */
  import { tr } from "@repo/hyphen-i18n";
  import { VwDonutChart, VwPanel, VwText } from "@repo/hyphen-views";
  import type { Component } from "svelte";

  import { getMetricsHistory, getWebUiConfig } from "$lib";
  import { HyUtilFormatBytes, HyUtilFormatNumber, HyUtilFormatSelectedAmount } from "$lib/HyUtil";
  import type {
    ApiMetrics,
    ApiMempoolSummary,
    ApiNetworkPeerInfo,
    ApiNetworkResponse,
  } from "$lib/api";
  import type { ChartPoint } from "$lib/data/chart";
  import { getTimeRangeSeconds, type TimeRange } from "$lib/data/hashrate";
  import type { HyDonutPreset } from "$lib/views";
  import { HyGetBoundedDonutPreset, HyGetDonutPreset, HyGraphWidget, HyPlainCounter } from "$lib/views";

  import IconSupply from "virtual:icons/mdi/circle-multiple-outline";
  import IconPool from "virtual:icons/mdi/pool";
  import IconMempool from "virtual:icons/mdi/tray-full";
  import IconChainstate from "virtual:icons/mdi/database-outline";
  import IconUtxo from "virtual:icons/mdi/file-tree-outline";
  import IconSpeed from "virtual:icons/mdi/speedometer";

  interface DonutSegment {
    readonly label: string;
    readonly value: number;
    readonly color: string;
  }

  interface CounterEntry {
    readonly icon: Component;
    readonly label: string;
    readonly value: string;
  }

  interface Props {
    readonly snapshot: ApiNetworkResponse;
    readonly mempool: ApiMempoolSummary | null;
    readonly validatorsEnabled?: boolean;
    readonly metricsHistory: readonly ApiMetrics[];
  }

  const { snapshot, mempool, validatorsEnabled = true, metricsHistory }: Props = $props();

  const DONUT_COLORS = ["#4C7EFF", "#60F6D2", "#FF6B8A", "#FFB74D", "#AB7AFF", "#4DD0E1"] as const;
  const STATUS_COLORS = {
    active: "#60F6D2",
    banned: "#FF6B8A",
    registered: "#FFB74D",
    unknown: "#4DD0E1",
  } as const;

  let hoveredStatus = $state<number | null>(null);
  let hoveredVersion = $state<number | null>(null);
  let dashboardWidth = $state(0);
  let nodeBodyWidth = $state(0);
  let nodeBodyHeight = $state(0);
  let versionBodyWidth = $state(0);
  let versionBodyHeight = $state(0);

  const latestMetric = $derived(metricsHistory[metricsHistory.length - 1] ?? null);
  const creditPoolStartHeight = $derived(getWebUiConfig().network.creditPoolStartHeight);

  const supplyPoints = $derived.by((): readonly ChartPoint[] => {
    return metricsHistory.map((metric): ChartPoint => ({ timestamp: metric.height, value: metric.circulatingSupply }));
  });

  const creditPoolPoints = $derived.by((): readonly ChartPoint[] => {
    return metricsHistory
      .filter((metric): boolean => metric.height >= creditPoolStartHeight)
      .map(
      (metric): ChartPoint => ({ timestamp: metric.height, value: metric.creditPoolBalance ?? 0 })
      );
  });

  const circulatingLabel = $derived.by((): string => {
    return latestMetric == null ? "—" : HyUtilFormatSelectedAmount(latestMetric.circulatingSupply, getWebUiConfig(), 0);
  });

  const creditPoolLabel = $derived.by((): string => {
    return latestMetric == null || latestMetric.height < creditPoolStartHeight || latestMetric.creditPoolBalance == null
      ? "—"
      : HyUtilFormatSelectedAmount(latestMetric.creditPoolBalance, getWebUiConfig(), 0);
  });

  const showCreditPoolGraph = $derived(creditPoolPoints.length > 0);

  const chainstateLabel = $derived.by((): string => {
    return latestMetric == null ? "—" : HyUtilFormatBytes(latestMetric.blockchainSize);
  });

  const utxoSetLabel = $derived.by((): string => {
    if (latestMetric == null) {
      return "—";
    }

    const utxoCount = latestMetric.utxosSpendable + latestMetric.utxosUnspendable;
    return HyUtilFormatNumber(utxoCount);
  });

  const mempoolPressureLabel = $derived.by((): string => {
    return mempool == null ? "—" : HyUtilFormatBytes(mempool.totalBytes);
  });

  const txVelocityLabel = $derived.by((): string => {
    if (metricsHistory.length < 2) {
      return "—";
    }

    const latest = metricsHistory[metricsHistory.length - 1];
    const previous = metricsHistory[metricsHistory.length - 2];

    if (latest == null || previous == null) {
      return "—";
    }

    const txDelta = Math.max(0, latest.txCount - previous.txCount);
    const heightDelta = Math.max(1, latest.height - previous.height);
    const timeSeconds = heightDelta * Math.max(getWebUiConfig().network.blockTimeSeconds, 1);
    const txVelocity = txDelta / timeSeconds;
    return txVelocity.toFixed(1) + " tx/s";
  });

  function readPeerSubversion(peer: ApiNetworkPeerInfo): string | null {
    const subversion = peer["subversion"];
    if (typeof subversion === "string" && subversion.length > 0) {
      return subversion;
    }

    const subver = peer["subver"];
    return typeof subver === "string" && subver.length > 0 ? subver : null;
  }

  function normalizeSubversion(subversion: string): string {
    const prefixIndex = subversion.indexOf(":");
    const suffixIndex = subversion.lastIndexOf("/");

    if (prefixIndex >= 0 && suffixIndex > prefixIndex) {
      return subversion.slice(prefixIndex + 1, suffixIndex);
    }

    return subversion.replaceAll("/", "");
  }

  const coreVersionSegments = $derived.by((): readonly DonutSegment[] => {
    const counts = new Map<string, number>();

    for (const peer of snapshot.peerInfo) {
      const subversion = readPeerSubversion(peer);
      if (subversion == null) {
        continue;
      }

      const version = normalizeSubversion(subversion);
      counts.set(version, (counts.get(version) ?? 0) + 1);
    }

    return [...counts.entries()]
      .sort((left, right): number => right[1] - left[1])
      .map(
        ([label, value], index): DonutSegment => ({
          color: DONUT_COLORS[index % DONUT_COLORS.length] ?? DONUT_COLORS[0],
          label,
          value,
        })
      );
  });

  const nodeStatusSegments = $derived.by((): readonly DonutSegment[] => {
    if (!validatorsEnabled) {
      return [];
    }

    const counts = new Map<string, { label: string; value: number; color: string }>();
    const summaries = [snapshot.validators.regular, snapshot.validators.evonode];

    for (const summary of summaries) {
      for (const [status, value] of Object.entries(summary.status)) {
        const normalized = status.toLowerCase();
        const key =
          normalized === "valid" || normalized === "enabled"
            ? "active"
            : normalized === "registered"
              ? "registered"
              : normalized === "banned" || normalized === "revoked" || normalized === "pose_banned"
                ? "banned"
                : normalized;
        const label =
          key === "active"
            ? tr("Active")
            : key === "registered"
              ? tr("Registered")
              : key === "banned"
                ? tr("Banned")
                : status;
        const color =
          key === "active"
            ? STATUS_COLORS.active
            : key === "registered"
              ? STATUS_COLORS.registered
              : key === "banned"
                ? STATUS_COLORS.banned
                : STATUS_COLORS.unknown;
        const previous = counts.get(key);
        counts.set(key, {
          label,
          color,
          value: (previous?.value ?? 0) + value,
        });
      }
    }

    return [...counts.values()].sort((left, right): number => right.value - left.value);
  });

  const statusTotal = $derived(nodeStatusSegments.reduce((sum, segment) => sum + segment.value, 0));
  const versionTotal = $derived(coreVersionSegments.reduce((sum, segment) => sum + segment.value, 0));
  const useSingleColumnLegend = $derived(dashboardWidth <= 768);
  const fallbackDonutPreset = $derived(HyGetDonutPreset(dashboardWidth / 2));
  const nodeDonutPreset = $derived.by((): HyDonutPreset => {
    if (nodeBodyWidth <= 0 || nodeBodyHeight <= 0) {
      return fallbackDonutPreset;
    }

    return HyGetBoundedDonutPreset(nodeBodyWidth, nodeBodyHeight, nodeStatusSegments.length, useSingleColumnLegend);
  });
  const versionDonutPreset = $derived.by((): HyDonutPreset => {
    if (versionBodyWidth <= 0 || versionBodyHeight <= 0) {
      return fallbackDonutPreset;
    }

    return HyGetBoundedDonutPreset(
      versionBodyWidth,
      versionBodyHeight,
      coreVersionSegments.length,
      useSingleColumnLegend
    );
  });

  const nodeStatusCenterLabel = $derived.by((): string => {
    const index = hoveredStatus;
    return index != null ? (nodeStatusSegments[index]?.label ?? tr("Validator Status")) : tr("Validator Status");
  });

  const nodeStatusCenterValue = $derived.by((): string => {
    const index = hoveredStatus;
    return String(index != null ? (nodeStatusSegments[index]?.value ?? 0) : (nodeStatusSegments[0]?.value ?? 0));
  });

  const versionCenterValue = $derived.by((): string => {
    if (versionTotal <= 0) {
      return "0.0%";
    }

    const index = hoveredVersion;
    const value = index != null ? (coreVersionSegments[index]?.value ?? 0) : (coreVersionSegments[0]?.value ?? 0);
    return ((value / versionTotal) * 100).toFixed(1) + "%";
  });

  function legendValue(segment: DonutSegment, total: number): string {
    if (total <= 0) {
      return "0.0%";
    }

    return ((segment.value / total) * 100).toFixed(1) + "%";
  }

  function formatAmountMetric(value: number): string {
    return HyUtilFormatSelectedAmount(value, getWebUiConfig(), 0);
  }

  async function loadMetricRange(
    range: TimeRange,
    mapValue: (metric: ApiMetrics) => number
  ): Promise<{ readonly bucketBlocks: number; readonly points: readonly ChartPoint[] }> {
    const blockTimeSeconds = getWebUiConfig().network.blockTimeSeconds;
    const blockHeight = metricsHistory[metricsHistory.length - 1]?.height ?? 0;

    if (blockHeight <= 0) {
      return { bucketBlocks: 1, points: [] };
    }

    const response =
      range === "All"
        ? await getMetricsHistory(blockHeight)
        : await (async (): Promise<Awaited<ReturnType<typeof getMetricsHistory>>> => {
            const rangeSeconds = getTimeRangeSeconds(range) ?? blockTimeSeconds;
            const blocks = Math.max(1, Math.ceil(rangeSeconds / Math.max(blockTimeSeconds, 1)));
            const points = Math.min(500, blocks);
            return getMetricsHistory(blocks, points);
          })();

    return {
      bucketBlocks: response.bucketBlocks,
      points: response.value.map(
        (metric): ChartPoint => ({
          timestamp: metric.height,
          value: mapValue(metric),
        })
      ),
    };
  }

  async function loadCreditPoolRange(
    range: TimeRange
  ): Promise<{ readonly bucketBlocks: number; readonly points: readonly ChartPoint[] }> {
    const result = await loadMetricRange(
      range,
      (metric): number => (metric.height >= creditPoolStartHeight ? (metric.creditPoolBalance ?? 0) : 0)
    );

    return {
      bucketBlocks: result.bucketBlocks,
      points: result.points.filter((point): boolean => point.timestamp >= creditPoolStartHeight),
    };
  }

  const lowerCounters = $derived.by((): readonly CounterEntry[] => {
    return [
      { icon: IconMempool, label: tr("Mempool Pressure"), value: mempoolPressureLabel },
      { icon: IconSpeed, label: tr("TX Velocity"), value: txVelocityLabel },
    ];
  });
</script>

<div bind:clientWidth={dashboardWidth} class="dashboard">
  <div class="col-left">
    <div class="counters-top desktop-only">
      <HyGraphWidget
        blockTimeSeconds={getWebUiConfig().network.blockTimeSeconds}
        data={supplyPoints}
        icon={IconSupply}
        label={tr("Circulating Supply")}
        loadRange={(
          range: TimeRange
        ): Promise<{ readonly bucketBlocks: number; readonly points: readonly ChartPoint[] }> =>
          loadMetricRange(range, (metric): number => metric.circulatingSupply)}
        value={circulatingLabel}
        valueFormatter={formatAmountMetric}
      />
      <div class="counters-stack">
        <HyPlainCounter icon={IconChainstate} label={tr("Chainstate")} value={chainstateLabel} />
        <HyPlainCounter icon={IconUtxo} label={tr("UTXO Set")} value={utxoSetLabel} />
      </div>
    </div>

    <div class="donut-panel">
      <VwPanel border={true} padding={0} radius="sm" variant="glass">
        <div class="donut-header">
          <VwText as="h2" size="md+" weight="extrabold">{tr("Validator Status")}</VwText>
        </div>
        <div bind:clientHeight={nodeBodyHeight} bind:clientWidth={nodeBodyWidth} class="donut-body">
          <div class="donut-center">
            <div
              class="donut-shell"
              style:height={String(nodeDonutPreset.diameter) + "px"}
              style:width={String(nodeDonutPreset.diameter) + "px"}
            >
              <VwDonutChart
                centerLabel={nodeStatusCenterLabel}
                centerValue={nodeStatusCenterValue}
                hoveredIndex={hoveredStatus}
                onhover={(index: number | null): void => {
                  hoveredStatus = index;
                }}
                segments={nodeStatusSegments}
                size={nodeDonutPreset.diameter}
                strokeWidth={nodeDonutPreset.strokeWidth}
              />
            </div>
          </div>
          <div class="legend">
            {#each nodeStatusSegments as segment, index (segment.label)}
              <div
                class:dimmed={hoveredStatus != null && hoveredStatus !== index}
                class="legend-row"
                onmouseenter={(): void => {
                  hoveredStatus = index;
                }}
                onmouseleave={(): void => {
                  hoveredStatus = null;
                }}
                role="listitem"
              >
                <span class="legend-dot" style:background={segment.color}></span>
                <span class="legend-label">{segment.label}</span>
                <span class="legend-value">{legendValue(segment, statusTotal)}</span>
              </div>
            {/each}
          </div>
        </div>
      </VwPanel>
    </div>
  </div>

  <div class="col-right">
    <div class="versions-panel">
      <VwPanel border={true} padding={0} radius="sm" variant="glass">
        <div class="donut-header">
          <VwText as="h2" size="md+" weight="extrabold">{tr("Core Versions")}</VwText>
        </div>
        <div bind:clientHeight={versionBodyHeight} bind:clientWidth={versionBodyWidth} class="donut-body">
          <div class="donut-center">
            <div
              class="donut-shell"
              style:height={String(versionDonutPreset.diameter) + "px"}
              style:width={String(versionDonutPreset.diameter) + "px"}
            >
              <VwDonutChart
                centerLabel={tr("Majority")}
                centerValue={versionCenterValue}
                hoveredIndex={hoveredVersion}
                onhover={(index: number | null): void => {
                  hoveredVersion = index;
                }}
                segments={coreVersionSegments}
                size={versionDonutPreset.diameter}
                strokeWidth={versionDonutPreset.strokeWidth}
              />
            </div>
          </div>
          <div class="legend">
            {#each coreVersionSegments as segment, index (segment.label)}
              <div
                class:dimmed={hoveredVersion != null && hoveredVersion !== index}
                class="legend-row"
                onmouseenter={(): void => {
                  hoveredVersion = index;
                }}
                onmouseleave={(): void => {
                  hoveredVersion = null;
                }}
                role="listitem"
              >
                <span class="legend-dot" style:background={segment.color}></span>
                <span class="legend-label">{segment.label}</span>
                <span class="legend-value">{legendValue(segment, versionTotal)}</span>
              </div>
            {/each}
          </div>
        </div>
      </VwPanel>
    </div>

    <div class="mobile-network-counters mobile-only">
      <HyPlainCounter icon={IconUtxo} label={tr("UTXO Set")} value={utxoSetLabel} />
      <HyPlainCounter icon={IconChainstate} label={tr("Chainstate")} value={chainstateLabel} />
    </div>

    <div class="counters-bottom desktop-only">
      <div class="counters-stack">
        {#each lowerCounters as counter (counter.label)}
          <HyPlainCounter icon={counter.icon} label={counter.label} value={counter.value} />
        {/each}
      </div>
      {#if showCreditPoolGraph}
        <HyGraphWidget
          blockTimeSeconds={getWebUiConfig().network.blockTimeSeconds}
          data={creditPoolPoints}
          icon={IconPool}
          label={tr("Credit Pool")}
          loadRange={loadCreditPoolRange}
          value={creditPoolLabel}
          valueFormatter={formatAmountMetric}
        />
      {:else}
        <HyPlainCounter icon={IconPool} label={tr("Credit Pool")} value={creditPoolLabel} />
      {/if}
    </div>
  </div>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .dashboard {
    container-type: inline-size;
    display: grid;
    gap: 12px;
    grid-template-columns: 1fr 1fr;
    min-height: 0;
    min-width: 0;
    height: 100%;
  }

  .col-left,
  .col-right {
    display: grid;
    gap: 12px;
    min-height: 0;
    min-width: 0;
  }

  .col-left {
    grid-template-rows: 200px minmax(0, 1fr);
  }

  .col-right {
    grid-template-rows: minmax(0, 1fr) 200px;
  }

  .counters-top,
  .counters-bottom {
    display: grid;
    gap: 12px;
    grid-template-columns: 1fr 1fr;
    min-width: 0;
  }

  .counters-stack {
    display: grid;
    gap: 8px;
    grid-template-rows: 1fr 1fr;
    min-width: 0;
  }

  .donut-panel,
  .versions-panel {
    min-height: 0;
    min-width: 0;
  }

  .mobile-only {
    display: none;
  }

  .donut-panel :global(.panel),
  .versions-panel :global(.panel) {
    height: 100%;
    display: flex;
    flex-direction: column;
    overflow: hidden;
  }

  .donut-header {
    padding: 16px 20px;
    border-bottom: 1px solid var(--glass-border);
    flex-shrink: 0;
  }

  .donut-body {
    display: flex;
    flex: 1;
    flex-direction: column;
    gap: 16px;
    min-height: 0;
    overflow: hidden;
    padding: 20px 24px;
  }

  .donut-center {
    align-items: center;
    display: flex;
    flex: 0 0 auto;
    justify-content: center;
    min-height: 0;
  }

  .donut-shell {
    flex: 0 0 auto;
    max-height: 100%;
    max-width: 100%;
  }

  .legend {
    align-content: start;
    display: grid;
    flex-shrink: 0;
    gap: 4px 12px;
    grid-template-columns: 1fr 1fr;
    min-width: 0;
    width: 100%;
  }

  .legend-row {
    align-items: center;
    border-radius: 6px;
    cursor: default;
    display: flex;
    gap: 8px;
    padding: 4px 8px;
    transition: opacity 150ms ease;
  }

  .legend-row:hover {
    background: var(--overlay-subtle);
  }

  .legend-row.dimmed {
    opacity: 0.35;
  }

  .legend-dot {
    border-radius: 50%;
    flex-shrink: 0;
    height: 10px;
    width: 10px;
  }

  .legend-label,
  .legend-value {
    font-size: var(--text-sm);
  }

  .legend-label {
    color: var(--color-fg);
    flex: 1;
  }

  .legend-value {
    color: var(--text-muted);
    font-family: var(--font-mono);
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .dashboard {
      display: grid;
      gap: 12px;
      grid-template-columns: minmax(0, 1fr);
      height: auto;
      min-height: 100%;
      overflow: visible;
      padding-bottom: var(--hy-mobile-dock-clearance);
    }

    .col-left,
    .col-right {
      display: contents;
      grid-template-rows: auto;
      min-height: auto;
    }

    .desktop-only {
      display: none;
    }

    .mobile-only {
      display: grid;
    }

    .versions-panel {
      order: 1;
    }

    .mobile-network-counters {
      gap: 12px;
      grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
      order: 2;
    }

    .donut-panel {
      order: 3;
    }

    .donut-panel :global(.panel) {
      height: auto;
      min-height: 320px;
    }

    .versions-panel :global(.panel) {
      height: auto;
      min-height: 400px;
    }

    .donut-body {
      overflow: visible;
    }

    .legend {
      grid-template-columns: 1fr;
    }
  }
</style>
