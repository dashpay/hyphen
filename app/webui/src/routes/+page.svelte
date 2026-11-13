<script lang="ts">
  import { goto } from "$app/navigation";
  import { tr } from "@repo/hyphen-i18n";
  import { onMount } from "svelte";

  import IconBlock from "virtual:icons/mdi/cube-outline";
  import IconClock from "virtual:icons/mdi/clock-outline";
  import IconDatabase from "virtual:icons/mdi/database-outline";
  import IconSupply from "virtual:icons/mdi/cash-multiple";
  import IconTxVolume from "virtual:icons/mdi/chart-timeline-variant";
  import IconDifficulty from "virtual:icons/roentgen/stone";

  import type { ApiBlockSummary, ApiMetrics, ApiMempoolTxSummary } from "$lib";
  import { explorer, getTransaction, getWebUiConfig, getMetricsHistory, getMempoolTxs } from "$lib";
  import {
    HyUtilFormatNumber,
    HyUtilFormatBytes,
    HyUtilFormatDifficulty,
    HyUtilFormatFullDate,
    HyUtilFormatHashRate,
    HyUtilFormatSelectedAmount,
    HyUtilWebUiName,
  } from "$lib/HyUtil";
  import type { ChartPoint } from "$lib/data/chart";
  import { getTimeRangeSeconds, type TimeRange } from "$lib/data/hashrate";
  import { HyBlList } from "$lib/block/views";
  import { HyTxFormatTransactionFlow } from "$lib/tx/HyTxUtil";
  import { HyTxList } from "$lib/tx/views";
  import {
    HyCounterPair,
    HyCounterStack,
    HyDashboardLayout,
    HyGraphWidget,
    HyPlainCounter,
    HyPageFrame,
  } from "$lib/views";
  import { getTransactionDisplay } from "$lib/tx/display";

  interface DashboardTransaction {
    readonly summary: ApiMempoolTxSummary;
    readonly detail: Awaited<ReturnType<typeof getTransaction>>;
  }

  let transactions = $state<readonly DashboardTransaction[]>([]);
  let metricsHistory = $state<readonly ApiMetrics[]>([]);
  let metricsLoadedHeight = $state<number | null>(null);

  const displayBlocks = $derived.by((): readonly ApiBlockSummary[] => explorer.blocks);
  const displayTransactions = $derived.by(() => {
    return transactions.map((transaction) => ({
      hash: transaction.summary.txHash,
      txid: transaction.summary.txHash,
      detail: getTransactionDisplay(transaction.summary.txType ?? "normal", null).label,
      detailTone: getTransactionDisplay(transaction.summary.txType ?? "normal", null).tone,
      amount:
        transaction.summary.valueOut == null
          ? "—"
          : HyUtilFormatSelectedAmount(transaction.summary.valueOut, getWebUiConfig()),
      blockLabel: null,
      metaLabel: HyUtilFormatFullDate(transaction.summary.lastSeenAt),
      sizeLabel: HyUtilFormatBytes(transaction.summary.txSize),
      flow: HyTxFormatTransactionFlow(transaction.detail),
    }));
  });

  const hashRatePoints = $derived.by((): readonly ChartPoint[] => {
    return metricsHistory.map(
      (metric): ChartPoint => ({
        timestamp: metric.height,
        value: metric.hashrate,
      })
    );
  });

  const processedTransactionPoints = $derived.by((): readonly ChartPoint[] => {
    return metricsHistory.map(
      (metric): ChartPoint => ({
        timestamp: metric.height,
        value: metric.txCount,
      })
    );
  });

  const circulatingSupplyValue = $derived.by((): string => {
    const supply = explorer.circulatingSupply;
    return supply > 0 ? HyUtilFormatSelectedAmount(supply, getWebUiConfig(), 0) : tr("Unavailable");
  });

  function formatIntegerMetric(value: number): string {
    return HyUtilFormatNumber(Math.round(value));
  }

  async function loadMetricRange(
    range: TimeRange,
    mapValue: (metric: ApiMetrics) => number
  ): Promise<{ readonly bucketBlocks: number; readonly points: readonly ChartPoint[] }> {
    const blockHeight = explorer.blockHeight;
    const blockTimeSeconds = getWebUiConfig().network.blockTimeSeconds;

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

  onMount((): void => {
    void Promise.allSettled([getMempoolTxs(8)]).then((results): void => {
      const [transactionResult] = results;

      if (transactionResult.status === "fulfilled") {
        void Promise.all(
          transactionResult.value.value.map(async (transaction: ApiMempoolTxSummary): Promise<DashboardTransaction> => {
            const detail = await getTransaction(transaction.txHash);
            return {
              summary: {
                ...transaction,
                txType: detail?.txType ?? transaction.txType ?? null,
              },
              detail,
            };
          })
        ).then((detailedTransactions): void => {
          transactions = detailedTransactions;
        });
      } else {
        transactions = [];
      }
    });
  });

  $effect((): void => {
    const blockHeight = explorer.blockHeight;

    if (blockHeight <= 0 || metricsLoadedHeight === blockHeight) {
      return;
    }

    void getMetricsHistory(blockHeight, 500)
      .then((response): void => {
        metricsHistory = response.value;
        metricsLoadedHeight = response.asOfHeight;
      })
      .catch((caught: unknown): void => {
        metricsHistory = [];
        void caught;
      });
  });
</script>

<svelte:head>
  <title>{tr("{} | {}", HyUtilWebUiName, tr("Overview"))}</title>
</svelte:head>

<HyPageFrame>
  <HyDashboardLayout>
    {#snippet left()}
      <div class="overview-section desktop-only overview-desktop-hero">
        <HyCounterPair>
          <HyCounterStack>
            <HyPlainCounter
              icon={IconBlock}
              label={tr("Block Height")}
              value={HyUtilFormatNumber(explorer.blockHeight)}
            />
            <HyPlainCounter
              icon={IconDifficulty}
              label={tr("Difficulty")}
              value={HyUtilFormatDifficulty(explorer.difficulty)}
            />
          </HyCounterStack>

          <HyGraphWidget
            blockTimeSeconds={getWebUiConfig().network.blockTimeSeconds}
            data={hashRatePoints}
            icon={IconClock}
            label={tr("Hash Rate")}
            loadRange={(
              range: TimeRange
            ): Promise<{ readonly bucketBlocks: number; readonly points: readonly ChartPoint[] }> =>
              loadMetricRange(range, (metric): number => metric.hashrate)}
            value={HyUtilFormatHashRate(explorer.latestMetrics?.hashrate ?? 0)}
            valueFormatter={HyUtilFormatHashRate}
          />
        </HyCounterPair>
      </div>

      <div class="overview-section mobile-only overview-mobile-summary">
        <HyPlainCounter icon={IconBlock} label={tr("Block Height")} value={HyUtilFormatNumber(explorer.blockHeight)} />
      </div>

      <div class="overview-section mobile-only overview-mobile-blocks">
        <HyBlList
          mobileExpand={true}
          blocks={displayBlocks.map((block) => ({
            height: block.blockHeight,
            hash: block.blockHash,
            time: HyUtilFormatFullDate(block.blockTime),
            txCount: block.txCount,
            size: HyUtilFormatBytes(block.blockSize),
          }))}
          onviewall={(): void => {
            void goto("/blocks");
          }}
          onblockclick={(hash: string): void => {
            void goto("/block?hash=" + hash);
          }}
        />
      </div>

      <div class="overview-section mobile-only overview-mobile-graphs">
        <HyCounterPair>
          <HyGraphWidget
            blockTimeSeconds={getWebUiConfig().network.blockTimeSeconds}
            data={processedTransactionPoints}
            icon={IconTxVolume}
            label={tr("Processed Transactions")}
            loadRange={(
              range: TimeRange
            ): Promise<{ readonly bucketBlocks: number; readonly points: readonly ChartPoint[] }> =>
              loadMetricRange(range, (metric): number => metric.txCount)}
            value={formatIntegerMetric(explorer.latestTransactionCount)}
            valueFormatter={formatIntegerMetric}
          />

          <HyGraphWidget
            blockTimeSeconds={getWebUiConfig().network.blockTimeSeconds}
            data={hashRatePoints}
            icon={IconClock}
            label={tr("Hash Rate")}
            loadRange={(
              range: TimeRange
            ): Promise<{ readonly bucketBlocks: number; readonly points: readonly ChartPoint[] }> =>
              loadMetricRange(range, (metric): number => metric.hashrate)}
            value={HyUtilFormatHashRate(explorer.latestMetrics?.hashrate ?? 0)}
            valueFormatter={HyUtilFormatHashRate}
          />
        </HyCounterPair>
      </div>

      <div class="overview-section overview-pending">
        <HyTxList
          mobileExpand={true}
          title={tr("Pending Transactions")}
          onviewall={(): void => {
            void goto("/transactions");
          }}
          transactions={displayTransactions}
          ontxclick={(txid: string): void => {
            void goto("/transaction?hash=" + txid);
          }}
        />
      </div>
    {/snippet}

    {#snippet right()}
      <div class="overview-section desktop-only overview-blocks">
        <HyBlList
          mobileExpand={true}
          blocks={displayBlocks.map((block) => ({
            height: block.blockHeight,
            hash: block.blockHash,
            time: HyUtilFormatFullDate(block.blockTime),
            txCount: block.txCount,
            size: HyUtilFormatBytes(block.blockSize),
          }))}
          onviewall={(): void => {
            void goto("/blocks");
          }}
          onblockclick={(hash: string): void => {
            void goto("/block?hash=" + hash);
          }}
        />
      </div>

      <div class="overview-section desktop-only overview-desktop-secondary">
        <HyCounterPair>
          <HyGraphWidget
            blockTimeSeconds={getWebUiConfig().network.blockTimeSeconds}
            data={processedTransactionPoints}
            icon={IconTxVolume}
            label={tr("Processed Transactions")}
            loadRange={(
              range: TimeRange
            ): Promise<{ readonly bucketBlocks: number; readonly points: readonly ChartPoint[] }> =>
              loadMetricRange(range, (metric): number => metric.txCount)}
            value={formatIntegerMetric(explorer.latestTransactionCount)}
            valueFormatter={formatIntegerMetric}
          />

          <HyCounterStack>
            <HyPlainCounter icon={IconSupply} label={tr("Circulating Supply")} value={circulatingSupplyValue} />
            <HyPlainCounter
              icon={IconDatabase}
              label={tr("Blockchain Size")}
              value={HyUtilFormatBytes(explorer.latestMetrics?.blockchainSize ?? 0)}
            />
          </HyCounterStack>
        </HyCounterPair>
      </div>
    {/snippet}
  </HyDashboardLayout>
</HyPageFrame>

<!-- nosemgrep: contrib.r2c.svelte-routes-no-style-blocks -->
<style lang="scss">
  @use "_views-mixins.scss" as view;

  .mobile-only {
    display: none;
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .desktop-only {
      display: none;
    }

    .mobile-only {
      display: block;
    }

    .overview-mobile-summary {
      order: 1;
    }

    .overview-blocks {
      order: 2;
    }

    .overview-mobile-blocks {
      order: 2;
    }

    .overview-mobile-graphs {
      order: 3;
    }

    .overview-pending {
      order: 4;
    }
  }
</style>
