<script lang="ts">
  import { tr } from "@repo/hyphen-i18n";
  import { onMount } from "svelte";

  import { HyBalancesExplorer, HyBalancesTable } from "$lib/balances/views";
  import { HyPageFrame, HyPageState } from "$lib/views";

  import type { ApiRichListEntry } from "$lib";
  import { getRichList, getWebUiConfig } from "$lib";
  import { HyUtilReadErrorMessage, HyUtilReadErrorStatus, HyUtilWebUiName } from "$lib/HyUtil";

  interface PercentileRow {
    readonly percentile: string;
    readonly pct: number;
    readonly colorVar: string;
  }

  let addresses = $state<readonly ApiRichListEntry[]>([]);
  let loading = $state(true);
  let loadingMore = $state(false);
  let errorMessage = $state<string | null>(null);
  let errorStatus = $state<number | null>(null);
  let copiedAddress = $state<string | null>(null);
  let hoveredSegment = $state<number | null>(null);
  let includedSupply = $state<number | null>(null);
  let nextCursor = $state<string | null>(null);
  let totalCount = $state<number | null>(null);

  const totalSupply = $derived.by((): number | null => {
    if (includedSupply == null || includedSupply <= 0) {
      return null;
    }

    return includedSupply;
  });

  const showsSupplySurfaces = $derived(getWebUiConfig().balances.enableConcentration);

  const percentileData = $derived.by((): readonly PercentileRow[] => {
    if (addresses.length === 0 || totalSupply == null || totalSupply <= 0) {
      return [];
    }

    const topHundred = addresses.slice(0, 100);
    const toPercentage = (value: number): number => (value / totalSupply) * 100;
    const topOne = topHundred
      .slice(0, 1)
      .reduce((sum: number, entry: ApiRichListEntry): number => sum + entry.balance, 0);
    const topTwoToFive = topHundred
      .slice(1, 5)
      .reduce((sum: number, entry: ApiRichListEntry): number => sum + entry.balance, 0);
    const topSixToTen = topHundred
      .slice(5, 10)
      .reduce((sum: number, entry: ApiRichListEntry): number => sum + entry.balance, 0);
    const topElevenToTwentyFive = topHundred
      .slice(10, 25)
      .reduce((sum: number, entry: ApiRichListEntry): number => sum + entry.balance, 0);
    const topTwentySixToFifty = topHundred
      .slice(25, 50)
      .reduce((sum: number, entry: ApiRichListEntry): number => sum + entry.balance, 0);
    const topFiftyOneToHundred = topHundred
      .slice(50)
      .reduce((sum: number, entry: ApiRichListEntry): number => sum + entry.balance, 0);

    return [
      { percentile: tr("Top 1"), pct: toPercentage(topOne), colorVar: "var(--hy-donut-top1)" },
      { percentile: tr("Top 2-5"), pct: toPercentage(topTwoToFive), colorVar: "var(--hy-donut-top2to5)" },
      { percentile: tr("Top 6-10"), pct: toPercentage(topSixToTen), colorVar: "var(--hy-donut-top6to10)" },
      { percentile: tr("Top 11-25"), pct: toPercentage(topElevenToTwentyFive), colorVar: "var(--hy-donut-top11to25)" },
      { percentile: tr("Top 26-50"), pct: toPercentage(topTwentySixToFifty), colorVar: "var(--hy-donut-top26to50)" },
      { percentile: tr("Top 51-100"), pct: toPercentage(topFiftyOneToHundred), colorVar: "var(--hy-donut-top51to100)" },
    ];
  });

  const donutSegments = $derived(
    percentileData.map((row: PercentileRow) => ({
      label: row.percentile,
      value: row.pct,
      color: row.colorVar,
    }))
  );

  const topConcentration = $derived(
    percentileData[0] != null ? percentileData[0].pct.toFixed(1) + "%" : tr("Unavailable")
  );

  function formatSupplyShare(entry: ApiRichListEntry): string {
    if (entry.pctSupply < 0) {
      return tr("Unavailable");
    }

    return (entry.pctSupply * 100).toFixed(4) + "%";
  }

  function copyAddress(address: string, event: MouseEvent): void {
    event.stopPropagation();

    void globalThis.navigator.clipboard.writeText(address).then((): void => {
      copiedAddress = address;
      globalThis.setTimeout((): void => {
        copiedAddress = null;
      }, 1_500);
    });
  }

  async function loadMore(): Promise<void> {
    if (loadingMore || nextCursor == null) {
      return;
    }

    loadingMore = true;

    try {
      const response = await getRichList(100, nextCursor);
      addresses = [...addresses, ...response.value];
      totalCount = response.totalCount;
      nextCursor = response.nextCursor ?? null;
    } catch {
      nextCursor = null;
    } finally {
      loadingMore = false;
    }
  }

  onMount((): void => {
    void getRichList(100)
      .then((response): void => {
        addresses = response.value;
        includedSupply = response.includedSupply;
        totalCount = response.totalCount;
        nextCursor = response.nextCursor ?? null;
      })
      .catch((caught: unknown): void => {
        errorMessage = HyUtilReadErrorMessage(caught);
        errorStatus = HyUtilReadErrorStatus(caught);
        nextCursor = null;
      })
      .finally((): void => {
        loading = false;
      });
  });
</script>

<svelte:head>
  <title>{tr("{} | {}", HyUtilWebUiName, tr("Balances"))}</title>
</svelte:head>

<HyPageFrame>
  <HyPageState empty={false} emptyMessage="" emptyTitle="" {errorMessage} {errorStatus} {loading}>
    {#if showsSupplySurfaces}
      <HyBalancesExplorer
        {addresses}
        {copiedAddress}
        {donutSegments}
        {formatSupplyShare}
        hasMore={nextCursor != null}
        {hoveredSegment}
        {loadingMore}
        oncopyaddress={copyAddress}
        onhover={(index: number | null): void => {
          hoveredSegment = index;
        }}
        onloadmore={loadMore}
        {percentileData}
        showsupply={true}
        {topConcentration}
        {totalCount}
      />
    {:else}
      <HyBalancesTable
        {addresses}
        {copiedAddress}
        {formatSupplyShare}
        hasMore={nextCursor != null}
        {loadingMore}
        oncopyaddress={copyAddress}
        onloadmore={loadMore}
        showsupply={false}
        {totalCount}
      />
    {/if}
  </HyPageState>
</HyPageFrame>
