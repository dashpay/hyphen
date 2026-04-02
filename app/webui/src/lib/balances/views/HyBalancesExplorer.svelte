<script lang="ts">
  /** Responsive balances explorer layout with mobile-first concentration summary. */
  import { onMount } from "svelte";
  import type { ApiRichListEntry } from "$lib";
  import { HyUtilGetBreakpointWidth } from "$lib/HyUtil";

  import HyBalancesTable from "./HyBalancesTable.svelte";
  import HyConcentrationPanel from "./HyConcentrationPanel.svelte";

  interface PercentileRow {
    readonly percentile: string;
    readonly pct: number;
    readonly colorVar: string;
  }

  interface DonutSegment {
    readonly color: string;
    readonly label: string;
    readonly value: number;
  }

  interface Props {
    readonly addresses: readonly ApiRichListEntry[];
    readonly copiedAddress: string | null;
    readonly donutSegments: readonly DonutSegment[];
    readonly formatSupplyShare: (entry: ApiRichListEntry) => string;
    readonly hasMore?: boolean;
    readonly hoveredSegment: number | null;
    readonly loadingMore?: boolean;
    readonly oncopyaddress: (address: string, event: MouseEvent) => void;
    readonly onhover: (index: number | null) => void;
    readonly onloadmore?: (() => unknown) | undefined;
    readonly percentileData: readonly PercentileRow[];
    readonly showsupply?: boolean;
    readonly topConcentration: string;
    readonly totalCount?: number | null;
  }

  const {
    addresses,
    copiedAddress,
    donutSegments,
    formatSupplyShare,
    hasMore = false,
    hoveredSegment,
    loadingMore = false,
    oncopyaddress,
    onhover,
    onloadmore,
    percentileData,
    showsupply = true,
    topConcentration,
    totalCount = null,
  }: Props = $props();

  let stackedForDock = $state(false);

  onMount((): (() => void) => {
    const query = globalThis.matchMedia(`(max-width: ${String(HyUtilGetBreakpointWidth("tablet"))}px)`);
    const sync = (): void => {
      stackedForDock = query.matches;
    };

    sync();
    query.addEventListener("change", sync);

    return (): void => {
      query.removeEventListener("change", sync);
    };
  });
</script>

<div class:dock-mode={stackedForDock} class="balances-explorer">
  <div class="balances-list">
    <HyBalancesTable
      {addresses}
      {copiedAddress}
      {formatSupplyShare}
      {hasMore}
      {loadingMore}
      {oncopyaddress}
      {onloadmore}
      scrollMode={stackedForDock ? "page" : "self"}
      {showsupply}
      {totalCount}
    />
  </div>

  <div class="balances-summary">
    <HyConcentrationPanel {donutSegments} {hoveredSegment} {onhover} {percentileData} {topConcentration} />
  </div>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .balances-explorer {
    display: grid;
    gap: var(--hy-browser-gap);
    grid-template-columns: minmax(0, 1fr) minmax(0, var(--hy-page-columns-side-width));
    height: 100%;
    min-width: 0;
  }

  .balances-summary,
  .balances-list {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  /* --hy-breakpoint-desktop */
  @include view.media-up(desktop) {
    .balances-explorer {
      grid-template-columns: minmax(0, 1fr) minmax(0, var(--hy-page-columns-side-width-wide));
    }
  }

  /* --hy-breakpoint-tablet */
  @include view.media-up(tablet) {
    .balances-summary :global(.list-panel),
    .balances-summary :global(.panel),
    .balances-list :global(.list-panel),
    .balances-list :global(.panel) {
      height: 100%;
    }
  }

  .balances-explorer.dock-mode {
    display: flex;
    flex-direction: column;
    height: auto;
    overflow: visible;
  }

  .balances-explorer.dock-mode .balances-summary,
  .balances-explorer.dock-mode .balances-list {
    display: block;
    min-height: auto;
    overflow: visible;
  }

  .balances-explorer.dock-mode .balances-summary {
    flex: 0 0 auto;
    order: -1;
  }

  .balances-explorer.dock-mode .balances-list {
    flex: 0 0 auto;
  }
</style>
