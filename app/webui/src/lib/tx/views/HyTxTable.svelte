<script lang="ts">
  /** Transaction table for history browsing and compact summaries. */
  import { fmt, tr } from "@repo/hyphen-i18n";
  import type { ApiTxSummary } from "$lib";
  import { getWebUiConfig } from "$lib";
  import { HyUtilFormatNumber, HyUtilFormatTableAmount, HyUtilGetBreakpointWidth } from "$lib/HyUtil";
  import { getTransactionDisplay } from "$lib/tx/display";
  import {
    HyCopyButton,
    HyPanelAsyncState,
    HyScrollBody,
    HyTableGetHeaderTemplate,
    HyTableGetTemplate,
    HyTableGetVisibleColumns,
    HyTableHasColumn,
    HyTableHeaderRow,
    HyTableMeasureTextWidth,
    HyTableReadFont,
    HyTableObserveWidth,
    type HyTableColumnSpec,
  } from "$lib/views";
  import HyTxTag from "./HyTxTag.svelte";

  interface Props {
    readonly hasMore?: boolean;
    readonly loadingMore?: boolean;
    readonly onloadmore?: (() => unknown) | undefined;
    readonly transactions: readonly ApiTxSummary[];
    readonly ontxclick?: (txHash: string) => void;
  }

  type HyTxColumnId = "size" | "txid" | "type" | "value";

  const { hasMore = false, loadingMore = false, onloadmore, transactions, ontxclick }: Props = $props();

  let copiedHash = $state<string | null>(null);
  let containerWidth = $state(Number.POSITIVE_INFINITY);
  let valueFont = $state<string | null>(null);
  let tableWrap = $state<HTMLDivElement | null>(null);

  const baseColumns = $derived.by((): readonly HyTableColumnSpec<HyTxColumnId>[] => [
    {
      id: "txid",
      label: tr("TxID"),
      flexible: true,
      minWidth: containerWidth <= HyUtilGetBreakpointWidth("phone") ? "180px" : "280px",
    },
    { id: "type", label: tr("Type"), width: "var(--hy-browser-col-type-width)", dropBelow: "phone" },
    {
      id: "value",
      label: tr("Value"),
      width: "var(--hy-browser-col-value-width)",
      align: "end",
    },
    { id: "size", label: tr("Size"), width: "var(--hy-browser-col-size-width)", align: "end", dropBelow: "tablet" },
  ]);
  const visibleColumns = $derived(HyTableGetVisibleColumns(baseColumns, containerWidth));
  const valueFloorWidth = $derived.by((): string => {
    const valueColumn = visibleColumns.find((column): boolean => column.id === "value");

    return valueColumn?.width ?? "0px";
  });
  const valueTexts = $derived.by((): readonly string[] => {
    return transactions.map((transaction: ApiTxSummary): string => {
      return HyUtilFormatTableAmount(transaction.valueOut, getWebUiConfig(), 4);
    });
  });
  const measuredValueWidth = $derived.by((): number | null => {
    if (valueFont == null) {
      return null;
    }

    return HyTableMeasureTextWidth([tr("Value"), ...valueTexts], valueFont, valueFloorWidth);
  });
  const columnSpecs = $derived.by((): readonly HyTableColumnSpec<HyTxColumnId>[] => {
    return visibleColumns.map((column): HyTableColumnSpec<HyTxColumnId> => {
      if (column.id === "value" && measuredValueWidth != null) {
        return {
          ...column,
          headerWidth: `${String(measuredValueWidth)}px`,
          width: `${String(measuredValueWidth)}px`,
        };
      }

      return column;
    });
  });
  const tableTemplate = $derived(HyTableGetTemplate(columnSpecs, containerWidth));
  const tableHeaderTemplate = $derived(HyTableGetHeaderTemplate(columnSpecs, containerWidth));

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

    valueFont = HyTableReadFont(tableWrap, ".col-value");
  });

  function copyHash(txid: string, event: MouseEvent): void {
    event.stopPropagation();
    void globalThis.navigator.clipboard.writeText(txid).then((): void => {
      copiedHash = txid;
      globalThis.setTimeout((): void => {
        copiedHash = null;
      }, 1_500);
    });
  }
</script>

<div bind:this={tableWrap} class="table-wrap">
  {#if transactions.length === 0}
    <HyPanelAsyncState empty emptyMessage={tr("No data.")} emptyTitle={tr("No entries")} />
  {:else}
    <HyTableHeaderRow columns={tableHeaderTemplate}>
      {#if HyTableHasColumn(visibleColumns, "txid")}
        <span class="col col-txid">{tr("TxID")}</span>
      {/if}
      {#if HyTableHasColumn(visibleColumns, "type")}
        <span class="col col-type">{tr("Type")}</span>
      {/if}
      {#if HyTableHasColumn(visibleColumns, "value")}
        <span class="col col-value">{tr("Value")}</span>
      {/if}
      {#if HyTableHasColumn(visibleColumns, "size")}
        <span class="col col-size">{tr("Size")}</span>
      {/if}
    </HyTableHeaderRow>

    <HyScrollBody {hasMore} {loadingMore} {onloadmore}>
      <div class="tx-table" style:--hy-table-columns={tableTemplate}>
        {#each transactions as transaction (transaction.txHash)}
          {@const txDisplay = getTransactionDisplay(transaction.txType, transaction.blockPos)}
          <div
            class="tx-row"
            onclick={(): void => {
              ontxclick?.(transaction.txHash);
            }}
            onkeydown={(event: KeyboardEvent): void => {
              if (event.key === "Enter") {
                ontxclick?.(transaction.txHash);
              }
            }}
            role="button"
            tabindex="0"
          >
            {#if HyTableHasColumn(visibleColumns, "txid")}
              <span class="col col-txid">
                <span class="hash-text mono accent">{transaction.txHash}</span>
                <HyCopyButton
                  copied={copiedHash === transaction.txHash}
                  copiedTitle={tr("Copied to clipboard")}
                  label={tr("Copy transaction ID")}
                  onpress={(event: MouseEvent): void => {
                    copyHash(transaction.txHash, event);
                  }}
                  title={tr("Copy txid")}
                />
              </span>
            {/if}
            {#if HyTableHasColumn(visibleColumns, "type")}
              <span class="col col-type">
                <HyTxTag label={txDisplay.label} tone={txDisplay.tone} />
              </span>
            {/if}
            {#if HyTableHasColumn(visibleColumns, "value")}
              <span class="col col-value">{HyUtilFormatTableAmount(transaction.valueOut, getWebUiConfig(), 4)}</span>
            {/if}
            {#if HyTableHasColumn(visibleColumns, "size")}
              <span class="col col-size muted">{fmt("{} B", HyUtilFormatNumber(transaction.txSize))}</span>
            {/if}
          </div>
        {/each}
      </div>
    </HyScrollBody>
  {/if}
</div>

<style>
  .table-wrap {
    display: flex;
    flex: 1;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  .tx-table {
    align-content: start;
    column-gap: var(--hy-browser-column-gap);
    display: grid;
    grid-template-columns: var(--hy-table-columns);
    min-height: 0;
    min-width: 0;
    width: 100%;
  }

  .tx-row {
    align-items: center;
    border-bottom: 1px solid var(--overlay-subtle);
    column-gap: var(--hy-browser-column-gap);
    cursor: pointer;
    display: grid;
    grid-column: 1 / -1;
    grid-template-columns: subgrid;
    padding: var(--hy-list-row-padding);
    transition: background var(--duration-fast) var(--ease-default);
  }

  .tx-row:hover,
  .tx-row:focus-visible {
    background: var(--overlay-subtle);
  }

  .tx-row:focus-visible {
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

  .col-txid {
    gap: var(--hy-tight-gap);
    min-width: 0;
    overflow: hidden;
  }

  .col-type,
  .col-value,
  .col-size {
    white-space: nowrap;
  }

  .col-value,
  .col-size {
    justify-content: flex-end;
    font-family: var(--font-mono);
  }

  .hash-text {
    display: block;
    flex: 1;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .accent {
    color: var(--accent-blue);
    font-weight: 700;
    text-decoration: none;
  }

  .muted {
    color: var(--text-muted);
  }
</style>
