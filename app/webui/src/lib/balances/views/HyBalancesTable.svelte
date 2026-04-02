<script lang="ts">
  /** Balances table for ranked rich-list address rows. */
  import { fmt, tr } from "@repo/hyphen-i18n";
  import type { ApiRichListEntry } from "$lib";

  import { getWebUiConfig } from "$lib";
  import { HyUtilFormatNumber, HyUtilFormatTableAmount } from "$lib/HyUtil";

  import {
    HyCopyButton,
    HyTableGetHeaderTemplate,
    HyListPanel,
    HyPanelAsyncState,
    HyScrollBody,
    HyTableGetTemplate,
    HyTableGetVisibleColumns,
    HyTableHasColumn,
    HyTableHeaderRow,
    HyTableMeasureTextWidth,
    HyTableReadFont,
    HyTableObserveWidth,
    type HyTableColumnSpec,
  } from "$lib/views";

  interface Props {
    readonly addresses: readonly ApiRichListEntry[];
    readonly copiedAddress: string | null;
    readonly formatSupplyShare: (entry: ApiRichListEntry) => string;
    readonly hasMore?: boolean;
    readonly loadingMore?: boolean;
    readonly oncopyaddress: (address: string, event: MouseEvent) => void;
    readonly onloadmore?: (() => unknown) | undefined;
    readonly scrollMode?: "page" | "self";
    readonly showsupply?: boolean;
    readonly totalCount?: number | null;
  }

  type HyBalancesColumnId = "address" | "balance" | "rank" | "supply" | "txcount";

  const {
    addresses,
    copiedAddress,
    formatSupplyShare,
    hasMore = false,
    loadingMore = false,
    oncopyaddress,
    onloadmore,
    scrollMode = "self",
    showsupply = true,
    totalCount = null,
  }: Props = $props();

  let containerWidth = $state(Number.POSITIVE_INFINITY);
  let balanceFont = $state<string | null>(null);
  let supplyFont = $state<string | null>(null);
  let tableWrap = $state<HTMLDivElement | null>(null);
  const allColumns = $derived.by((): readonly HyTableColumnSpec<HyBalancesColumnId>[] => [
    { id: "rank", label: tr("#"), width: "var(--hy-browser-bal-rank-width)", align: "end", dropBelow: "phone" },
    { id: "address", label: tr("Address"), flexible: true, minWidth: "0px" },
    { id: "txcount", label: tr("Txns"), width: "var(--hy-browser-bal-txns-width)", align: "end", dropBelow: "phone" },
    {
      id: "supply",
      label: tr("% Supply"),
      width: "var(--hy-browser-bal-supply-width)",
      headerWidth: "var(--hy-browser-bal-supply-header-width)",
      align: "end",
      dropBelow: "tablet",
    },
    {
      id: "balance",
      label: tr("Balance"),
      width: "var(--hy-browser-bal-balance-width)",
      headerWidth: "var(--hy-browser-bal-balance-header-width)",
      align: "end",
    },
  ]);
  const baseColumns = $derived.by((): readonly HyTableColumnSpec<HyBalancesColumnId>[] =>
    showsupply ? allColumns : allColumns.filter((column) => column.id !== "supply")
  );
  const visibleColumns = $derived(HyTableGetVisibleColumns(baseColumns, containerWidth));
  const balanceFloorWidth = $derived.by((): string => {
    const balanceColumn = visibleColumns.find((column): boolean => column.id === "balance");

    return balanceColumn?.width ?? "0px";
  });
  const supplyFloorWidth = $derived.by((): string => {
    const supplyColumn = visibleColumns.find((column): boolean => column.id === "supply");

    return supplyColumn?.width ?? "0px";
  });
  const balanceTexts = $derived.by((): readonly string[] => {
    return addresses.map((row: ApiRichListEntry): string => {
      return HyUtilFormatTableAmount(row.balance, getWebUiConfig());
    });
  });
  const supplyTexts = $derived.by((): readonly string[] => {
    return addresses.map((row: ApiRichListEntry): string => formatSupplyShare(row));
  });
  const hasSupplyColumn = $derived(HyTableHasColumn(visibleColumns, "supply"));
  const measuredBalanceWidth = $derived.by((): number | null => {
    if (balanceFont == null) {
      return null;
    }

    return HyTableMeasureTextWidth([tr("Balance"), ...balanceTexts], balanceFont, balanceFloorWidth);
  });
  const measuredSupplyWidth = $derived.by((): number | null => {
    if (supplyFont == null) {
      return null;
    }

    return HyTableMeasureTextWidth([tr("% Supply"), ...supplyTexts], supplyFont, supplyFloorWidth);
  });
  const columnSpecs = $derived.by((): readonly HyTableColumnSpec<HyBalancesColumnId>[] => {
    return visibleColumns.map((column): HyTableColumnSpec<HyBalancesColumnId> => {
      if (column.id === "balance" && measuredBalanceWidth != null) {
        return {
          ...column,
          elastic: !hasSupplyColumn,
          headerWidth: `${String(measuredBalanceWidth)}px`,
          width: `${String(measuredBalanceWidth)}px`,
        };
      }

      if (column.id === "supply" && measuredSupplyWidth != null) {
        return {
          ...column,
          headerWidth: `${String(measuredSupplyWidth)}px`,
          width: `${String(measuredSupplyWidth)}px`,
        };
      }

      return column;
    });
  });
  const tableHeaderTemplate = $derived(HyTableGetHeaderTemplate(columnSpecs, containerWidth));
  const tableTemplate = $derived(HyTableGetTemplate(columnSpecs, containerWidth));
  $effect(() => {
    if (tableWrap == null) {
      return;
    }

    return HyTableObserveWidth(tableWrap, (width: number): void => {
      containerWidth = width;
    });
  });

  $effect(() => {
    if (tableWrap == null) {
      return;
    }

    balanceFont = HyTableReadFont(tableWrap, ".col-balance");
    supplyFont = HyTableReadFont(tableWrap, ".col-supply");
  });
</script>

<div class:page-mode={scrollMode === "page"} class="balances-table">
  <HyListPanel
    subtitle={tr("{count, plural, one {# address} other {# addresses}}", { count: totalCount ?? addresses.length })}
    title={tr("Balances")}
  >
    <div bind:this={tableWrap} class="table-wrap">
      {#if addresses.length === 0}
        <HyPanelAsyncState empty emptyMessage={tr("No data.")} emptyTitle={tr("No entries")} />
      {:else}
        <HyTableHeaderRow columns={tableHeaderTemplate}>
          {#if HyTableHasColumn(visibleColumns, "rank")}
            <span class="col col-rank">{tr("#")}</span>
          {/if}
          {#if HyTableHasColumn(visibleColumns, "address")}
            <span class="col col-address">{tr("Address")}</span>
          {/if}
          {#if HyTableHasColumn(visibleColumns, "txcount")}
            <span class="col col-txcount">{tr("Txns")}</span>
          {/if}
          {#if HyTableHasColumn(visibleColumns, "supply")}
            <span class="col col-supply">{tr("% Supply")}</span>
          {/if}
          {#if HyTableHasColumn(visibleColumns, "balance")}
            <span class="col col-balance">{tr("Balance")}</span>
          {/if}
        </HyTableHeaderRow>

        <HyScrollBody {hasMore} {loadingMore} {onloadmore} {scrollMode}>
          <div class="table-grid" style:--hy-table-columns={tableTemplate}>
            {#each addresses as row (row.address)}
              <a class="addr-row row-animate" href={"/address?id=" + encodeURIComponent(row.address)}>
                {#if HyTableHasColumn(visibleColumns, "rank")}
                  <span class="col col-rank muted">{fmt("{}", HyUtilFormatNumber(row.rank))}</span>
                {/if}
                {#if HyTableHasColumn(visibleColumns, "address")}
                  <span class="col col-address hy-truncate-lane">
                    <span class="hash-text mono accent hy-truncate-text">{row.address}</span>
                    <HyCopyButton
                      copied={copiedAddress === row.address}
                      copiedTitle={tr("Copied to clipboard")}
                      label={tr("Copy address")}
                      onpress={(event: MouseEvent): void => {
                        oncopyaddress(row.address, event);
                      }}
                      title={tr("Copy address")}
                    />
                  </span>
                {/if}
                {#if HyTableHasColumn(visibleColumns, "txcount")}
                  <span class="col col-txcount muted">{HyUtilFormatNumber(row.txCount)}</span>
                {/if}
                {#if HyTableHasColumn(visibleColumns, "supply")}
                  <span class="col col-supply muted hy-nowrap">{formatSupplyShare(row)}</span>
                {/if}
                {#if HyTableHasColumn(visibleColumns, "balance")}
                  <span class="col col-balance hy-nowrap">
                    {HyUtilFormatTableAmount(row.balance, getWebUiConfig())}
                  </span>
                {/if}
              </a>
            {/each}
          </div>
        </HyScrollBody>
      {/if}
    </div>
  </HyListPanel>
</div>

<style>
  .balances-table {
    min-height: 0;
    min-width: 0;
  }

  .balances-table.page-mode {
    min-height: auto;
  }

  .balances-table.page-mode :global(.list-panel),
  .balances-table.page-mode :global(.panel) {
    height: auto;
  }

  .table-wrap {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  .balances-table.page-mode .table-wrap {
    flex: 0 0 auto;
    min-height: auto;
    overflow: visible;
  }

  .table-grid {
    align-content: start;
    column-gap: var(--hy-browser-column-gap);
    display: grid;
    grid-template-columns: var(--hy-table-columns);
    min-height: 0;
    min-width: 0;
    width: 100%;
  }

  .addr-row {
    align-items: center;
    border-bottom: 1px solid var(--overlay-subtle);
    color: inherit;
    cursor: pointer;
    display: grid;
    grid-column: 1 / -1;
    grid-template-columns: subgrid;
    padding: var(--hy-list-row-padding);
    text-decoration: none;
    transition: background var(--duration-fast) var(--ease-default);
  }

  .addr-row:hover {
    background: var(--overlay-subtle);
  }

  .addr-row:focus-visible {
    background: var(--overlay-subtle);
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }

  .col {
    align-items: center;
    display: flex;
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    min-width: 0;
  }

  .col-rank,
  .col-txcount,
  .col-balance,
  .col-supply {
    justify-content: flex-end;
    white-space: nowrap;
  }

  .col-rank {
    padding-right: var(--hy-tight-gap);
  }

  .col-address {
    gap: var(--hy-tight-gap);
    min-width: 0;
    overflow: hidden;
  }

  .col-balance {
    font-family: var(--font-mono);
    font-weight: 600;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .hash-text {
    flex: 1;
  }

  .mono {
    font-family: var(--font-mono, monospace);
  }

  .accent {
    color: var(--accent-blue);
    font-weight: 700;
  }

  .muted {
    color: var(--text-muted);
  }
</style>
