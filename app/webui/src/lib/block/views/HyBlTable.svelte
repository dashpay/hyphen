<script lang="ts">
  /** Block table for history browsing and compact summaries. */
  import { tr } from "@repo/hyphen-i18n";
  import type { ApiBlockSummary } from "$lib";
  import { HyUtilFormatBytes, HyUtilFormatFullDate, HyUtilFormatNumber, HyUtilGetBreakpointWidth } from "$lib/HyUtil";
  import {
    HyCopyButton,
    HyPanelAsyncState,
    HyScrollBody,
    HyTableGetTemplate,
    HyTableGetVisibleColumns,
    HyTableHasColumn,
    HyTableHeaderRow,
    HyTableObserveWidth,
    type HyTableColumnSpec,
  } from "$lib/views";

  interface Props {
    readonly blocks: readonly ApiBlockSummary[];
    readonly hasMore?: boolean;
    readonly loadingMore?: boolean;
    readonly onblockclick?: (hash: string) => void;
    readonly onloadmore?: (() => unknown) | undefined;
    readonly scrollMode?: "page" | "self";
  }

  type HyBlColumnId = "bits" | "hash" | "height" | "size" | "time" | "txns";

  const {
    blocks,
    hasMore = false,
    loadingMore = false,
    onblockclick,
    onloadmore,
    scrollMode = "self",
  }: Props = $props();

  let copiedHash = $state<string | null>(null);
  let containerWidth = $state(Number.POSITIVE_INFINITY);
  let tableWrap = $state<HTMLDivElement | null>(null);

  const baseColumns = $derived.by((): readonly HyTableColumnSpec<HyBlColumnId>[] => [
    { id: "height", label: tr("Height"), width: "var(--hy-browser-col-height-width)" },
    {
      id: "hash",
      label: tr("Hash"),
      flexible: true,
      minWidth: containerWidth <= HyUtilGetBreakpointWidth("phone") ? "188px" : "280px",
    },
    { id: "time", label: tr("Time"), width: "var(--hy-browser-col-time-width)", dropBelow: "phone" },
    { id: "txns", label: tr("Txns"), width: "var(--hy-browser-col-txns-width)", align: "end", dropBelow: "phone" },
    { id: "size", label: tr("Size"), width: "var(--hy-browser-col-size-width)", align: "end", dropBelow: "tablet" },
    { id: "bits", label: "Bits", width: "var(--hy-browser-col-bits-width)", align: "end", dropBelow: "tablet" },
  ]);
  const visibleColumns = $derived(HyTableGetVisibleColumns(baseColumns, containerWidth));
  const tableTemplate = $derived(HyTableGetTemplate(visibleColumns));

  $effect(() => {
    if (tableWrap == null) {
      return;
    }

    return HyTableObserveWidth(tableWrap, (width: number): void => {
      containerWidth = width;
    });
  });

  function copyHash(hash: string, event: MouseEvent): void {
    event.stopPropagation();
    void globalThis.navigator.clipboard.writeText(hash).then((): void => {
      copiedHash = hash;
      globalThis.setTimeout((): void => {
        copiedHash = null;
      }, 1_500);
    });
  }
</script>

<div bind:this={tableWrap} class:page-mode={scrollMode === "page"} class="table-wrap">
  {#if blocks.length === 0}
    <HyPanelAsyncState empty emptyMessage={tr("No data.")} emptyTitle={tr("No entries")} />
  {:else}
    <HyTableHeaderRow columns={tableTemplate}>
      {#if HyTableHasColumn(visibleColumns, "height")}
        <span class="col col-height">{tr("Height")}</span>
      {/if}
      {#if HyTableHasColumn(visibleColumns, "hash")}
        <span class="col col-hash">{tr("Hash")}</span>
      {/if}
      {#if HyTableHasColumn(visibleColumns, "time")}
        <span class="col col-time">{tr("Time")}</span>
      {/if}
      {#if HyTableHasColumn(visibleColumns, "txns")}
        <span class="col col-txns">{tr("Txns")}</span>
      {/if}
      {#if HyTableHasColumn(visibleColumns, "size")}
        <span class="col col-size">{tr("Size")}</span>
      {/if}
      {#if HyTableHasColumn(visibleColumns, "bits")}
        <span class="col col-bits">Bits</span>
      {/if}
    </HyTableHeaderRow>

    <HyScrollBody {hasMore} {loadingMore} {onloadmore} {scrollMode}>
      <div class="table-body" style:--hy-table-columns={tableTemplate}>
        {#each blocks as block (block.blockHeight)}
          <div
            class="block-row"
            onclick={(): void => {
              onblockclick?.(block.blockHash);
            }}
            onkeydown={(event: KeyboardEvent): void => {
              if (event.key === "Enter") {
                onblockclick?.(block.blockHash);
              }
            }}
            role="button"
            tabindex="0"
          >
            {#if HyTableHasColumn(visibleColumns, "height")}
              <span class="col col-height mono accent">{HyUtilFormatNumber(block.blockHeight)}</span>
            {/if}
            {#if HyTableHasColumn(visibleColumns, "hash")}
              <span class="col col-hash">
                <span class="hash-text mono muted">{block.blockHash}</span>
                <HyCopyButton
                  copied={copiedHash === block.blockHash}
                  copiedTitle={tr("Copied to clipboard")}
                  label={tr("Copy block hash")}
                  onpress={(event: MouseEvent): void => {
                    copyHash(block.blockHash, event);
                  }}
                  title={tr("Copy hash")}
                />
              </span>
            {/if}
            {#if HyTableHasColumn(visibleColumns, "time")}
              <span class="col col-time">{HyUtilFormatFullDate(block.blockTime)}</span>
            {/if}
            {#if HyTableHasColumn(visibleColumns, "txns")}
              <span class="col col-txns">{HyUtilFormatNumber(block.txCount)}</span>
            {/if}
            {#if HyTableHasColumn(visibleColumns, "size")}
              <span class="col col-size">{HyUtilFormatBytes(block.blockSize)}</span>
            {/if}
            {#if HyTableHasColumn(visibleColumns, "bits")}
              <span class="col col-bits mono">{block.bits}</span>
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

  .table-wrap.page-mode {
    flex: 0 0 auto;
    min-height: auto;
    overflow: visible;
  }

  .table-body {
    align-content: start;
    column-gap: var(--hy-browser-column-gap);
    display: grid;
    grid-template-columns: var(--hy-table-columns);
    min-height: 0;
    min-width: 0;
    width: 100%;
  }

  .block-row {
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

  .block-row:hover,
  .block-row:focus-visible {
    background: var(--overlay-subtle);
  }

  .block-row:focus-visible {
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

  .col-hash {
    gap: var(--hy-tight-gap);
    min-width: 0;
    overflow: hidden;
  }

  .col-txns,
  .col-size,
  .col-bits {
    justify-content: flex-end;
    white-space: nowrap;
  }

  .hash-text {
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
  }

  .muted {
    color: var(--text-muted);
  }
</style>
