<script lang="ts">
  /** Validator table for history browsing and filtered summaries. */
  import { fmt, tr } from "@repo/hyphen-i18n";

  import type { ApiValidatorSummary } from "$lib";
  import { getValidatorStatusTag, getValidatorTypeTag, getValidators } from "$lib";
  import {
    HyUtilGetBreakpointWidth,
    HyUtilFormatNumber,
    HyUtilFormatRelativeDate,
    HyUtilReadErrorMessage,
    HyUtilReadErrorStatus,
  } from "$lib/HyUtil";
  import { HyVlFormatEndpoint } from "$lib/validator/HyVlUtil";
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
    readonly globalEnabledCount?: number | null;
    readonly onvalidatorclick?: (proTxHash: string) => void;
    readonly onsummarychange?: (summary: string) => void;
    readonly scrollMode?: "page" | "self";
  }

  type HyVlColumnId = "address" | "protx" | "status" | "type" | "paid";

  const { globalEnabledCount = null, onvalidatorclick, onsummarychange, scrollMode = "self" }: Props = $props();

  let validators = $state<readonly ApiValidatorSummary[]>([]);
  let totalCount = $state(0);
  let loading = $state(true);
  let loadingMore = $state(false);
  let refreshing = $state(false);
  let errorMessage = $state<string | null>(null);
  let errorStatus = $state<number | null>(null);
  let disabledMessage = $state<string | null>(null);
  let copiedHash = $state<string | null>(null);
  let containerWidth = $state(Number.POSITIVE_INFINITY);
  let tableWrap = $state<HTMLDivElement | null>(null);
  let nextCursor = $state<string | null>(null);

  const baseColumns = $derived.by((): readonly HyTableColumnSpec<HyVlColumnId>[] => [
    {
      id: "protx",
      label: "ProTxHash",
      flexible: true,
      minWidth: containerWidth <= HyUtilGetBreakpointWidth("phone") ? "196px" : "280px",
    },
    { id: "type", label: tr("Type"), width: "var(--hy-browser-vl-type-width)", dropBelow: "tablet" },
    { id: "address", label: tr("IP"), width: "var(--hy-browser-vl-address-width)", dropBelow: "tablet" },
    { id: "status", label: tr("Status"), width: "var(--hy-browser-vl-status-width)" },
    {
      id: "paid",
      label: tr("Last Paid"),
      width: "var(--hy-browser-vl-reg-width)",
      align: "end",
      dropBelow: "tablet",
    },
  ]);
  const visibleColumns = $derived(HyTableGetVisibleColumns(baseColumns, containerWidth));
  const tableTemplate = $derived(HyTableGetTemplate(visibleColumns));

  function mergeValidators(
    currentRows: readonly ApiValidatorSummary[],
    nextRows: readonly ApiValidatorSummary[]
  ): readonly ApiValidatorSummary[] {
    if (currentRows.length === 0) {
      return nextRows;
    }

    const nextByHash = new Map(nextRows.map((row): readonly [string, ApiValidatorSummary] => [row.proTxHash, row]));
    const mergedRows = currentRows.map((row): ApiValidatorSummary => nextByHash.get(row.proTxHash) ?? row);
    const knownHashes = new Set(mergedRows.map((row): string => row.proTxHash));
    const appendedRows = nextRows.filter((row): boolean => !knownHashes.has(row.proTxHash));
    return [...mergedRows, ...appendedRows];
  }

  function getValidatorEnabled(validator: ApiValidatorSummary): boolean {
    const liveStatus = validator.live.status?.toLowerCase();
    return liveStatus === "valid";
  }

  function formatLastPaid(validator: ApiValidatorSummary): string {
    if ((validator.live.lastPaidHeight ?? 0) <= 0) {
      return tr("Unavailable");
    }

    if (validator.live.lastPaidHeight != null && validator.live.lastPaidAt != null) {
      return fmt(
        "#{} ({})",
        HyUtilFormatNumber(validator.live.lastPaidHeight),
        HyUtilFormatRelativeDate(validator.live.lastPaidAt)
      );
    }

    if (validator.live.lastPaidHeight != null) {
      return fmt("#{}", HyUtilFormatNumber(validator.live.lastPaidHeight));
    }

    if (validator.live.lastPaidAt != null) {
      return HyUtilFormatRelativeDate(validator.live.lastPaidAt);
    }

    return tr("Unavailable");
  }

  const validatorsSummary = $derived.by((): string => {
    const enabledCount =
      globalEnabledCount ?? validators.filter((validator): boolean => getValidatorEnabled(validator)).length;
    return tr("{totalCount, plural, one {# validator} other {# validators}} ({enabledCount} active)", {
      totalCount,
      enabledCount: HyUtilFormatNumber(enabledCount),
    });
  });

  $effect(() => {
    if (tableWrap == null) {
      return;
    }

    return HyTableObserveWidth(tableWrap, (width: number): void => {
      containerWidth = width;
    });
  });

  function copyToClipboard(value: string, event: MouseEvent): void {
    event.stopPropagation();
    void globalThis.navigator.clipboard.writeText(value).then((): void => {
      copiedHash = value;
      globalThis.setTimeout((): void => {
        copiedHash = null;
      }, 1_500);
    });
  }

  $effect((): void => {
    onsummarychange?.(validatorsSummary);
  });

  $effect(() => {
    const readValidators = async (background: boolean): Promise<void> => {
      if (!background) {
        validators = [];
        totalCount = 0;
        errorMessage = null;
        errorStatus = null;
        disabledMessage = null;
        loading = true;
      } else {
        refreshing = true;
      }

      try {
        const response = await getValidators(100);
        validators = background ? mergeValidators(validators, response.value) : response.value;
        totalCount = response.totalCount ?? validators.length;
        nextCursor = response.nextCursor ?? null;
        errorMessage = null;
        errorStatus = null;
      } catch (caught: unknown) {
        if (!background) {
          errorMessage = HyUtilReadErrorMessage(caught);
          errorStatus = HyUtilReadErrorStatus(caught);
          nextCursor = null;
        }
      } finally {
        loading = false;
        refreshing = false;
      }
    };

    void readValidators(false);

    const refreshHandle = globalThis.setInterval((): void => {
      if (loading || loadingMore || refreshing) {
        return;
      }

      void readValidators(true);
    }, 30_000);

    return (): void => {
      globalThis.clearInterval(refreshHandle);
    };
  });

  async function loadMore(): Promise<void> {
    if (loadingMore || nextCursor == null) {
      return;
    }

    loadingMore = true;

    try {
      const response = await getValidators(100, nextCursor);
      validators = mergeValidators(validators, response.value);
      totalCount = response.totalCount ?? validators.length;
      nextCursor = response.nextCursor ?? null;
    } catch {
      nextCursor = null;
    } finally {
      loadingMore = false;
    }
  }
</script>

{#if disabledMessage != null}
  <HyPanelAsyncState errorMessage={disabledMessage} />
{:else}
  <HyPanelAsyncState
    empty={validators.length === 0}
    {errorMessage}
    {errorStatus}
    {loading}
    emptyTitle={tr("No entries")}
    emptyMessage={tr("No data.")}
  >
    <div bind:this={tableWrap} class:page-mode={scrollMode === "page"} class="table-wrap">
      <HyTableHeaderRow columns={tableTemplate}>
        {#if HyTableHasColumn(visibleColumns, "protx")}
          <span class="col col-protx">ProTxHash</span>
        {/if}
        {#if HyTableHasColumn(visibleColumns, "type")}
          <span class="col col-type">{tr("Type")}</span>
        {/if}
        {#if HyTableHasColumn(visibleColumns, "address")}
          <span class="col col-address">{tr("IP")}</span>
        {/if}
        {#if HyTableHasColumn(visibleColumns, "status")}
          <span class="col col-status">{tr("Status")}</span>
        {/if}
        {#if HyTableHasColumn(visibleColumns, "paid")}
          <span class="col col-reg">{tr("Last Paid")}</span>
        {/if}
      </HyTableHeaderRow>

      <HyScrollBody hasMore={nextCursor != null} {loadingMore} onloadmore={loadMore} {scrollMode}>
        <div class="scroll-body" style:--hy-table-columns={tableTemplate}>
          {#each validators as validator (validator.proTxHash)}
            {@const typeTag = getValidatorTypeTag(validator.type)}
            {@const statusTag = getValidatorStatusTag(validator.status, validator.live.status)}
            <div
              class="table-row"
              onclick={(): void => {
                onvalidatorclick?.(validator.proTxHash);
              }}
              onkeydown={(event: KeyboardEvent): void => {
                if (event.key === "Enter") {
                  onvalidatorclick?.(validator.proTxHash);
                }
              }}
              role="button"
              tabindex="0"
            >
              {#if HyTableHasColumn(visibleColumns, "protx")}
                <span class="col col-protx hy-truncate-lane">
                  <span class="hash-text mono accent hy-truncate-text">{validator.proTxHash}</span>
                  <HyCopyButton
                    copied={copiedHash === validator.proTxHash}
                    copiedTitle={tr("Copied to clipboard")}
                    label={tr("Copy proTxHash")}
                    onpress={(event: MouseEvent): void => {
                      copyToClipboard(validator.proTxHash, event);
                    }}
                    title={tr("Copy proTxHash")}
                  />
                </span>
              {/if}
              {#if HyTableHasColumn(visibleColumns, "type")}
                <span class="col col-type">
                  <span class:mint-type={typeTag.tone === "mint"} class="type-badge">{typeTag.label}</span>
                </span>
              {/if}
              {#if HyTableHasColumn(visibleColumns, "address")}
                <span class="col col-address mono hy-truncate-text">
                  {HyVlFormatEndpoint(validator.services.coreP2p[0])}
                </span>
              {/if}
              {#if HyTableHasColumn(visibleColumns, "status")}
                <span class="col col-status">
                  <span
                    class:active-dot={statusTag.tone === "green"}
                    class:banned-dot={statusTag.tone === "red"}
                    class:registered-dot={statusTag.tone === "yellow"}
                    class="status-dot"
                  ></span>
                  <span
                    class:active-text={statusTag.tone === "green"}
                    class:banned-text={statusTag.tone === "red"}
                    class:registered-text={statusTag.tone === "yellow"}
                  >
                    {statusTag.label}
                  </span>
                </span>
              {/if}
              {#if HyTableHasColumn(visibleColumns, "paid")}
                <span class="col col-reg muted">
                  <span class="paid-text hy-truncate-text">{formatLastPaid(validator)}</span>
                </span>
              {/if}
            </div>
          {/each}
        </div>
      </HyScrollBody>
    </div>
  </HyPanelAsyncState>
{/if}

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

  .scroll-body {
    align-content: start;
    column-gap: var(--hy-browser-validator-column-gap);
    display: grid;
    grid-template-columns: var(--hy-table-columns);
    min-width: 0;
    width: 100%;
  }

  .table-row {
    align-items: center;
    border-bottom: 1px solid var(--overlay-subtle);
    cursor: pointer;
    display: grid;
    grid-column: 1 / -1;
    grid-template-columns: subgrid;
    padding: var(--hy-browser-validator-row-padding);
  }

  .table-row:hover {
    background: var(--overlay-subtle);
  }

  .table-row:focus-visible {
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

  .col-protx {
    gap: var(--hy-detail-tag-gap);
    min-width: 0;
    overflow: hidden;
  }

  .col-type,
  .col-address,
  .col-status,
  .col-reg {
    white-space: nowrap;
  }

  .col-status {
    gap: var(--hy-detail-tag-gap);
  }

  .col-reg {
    justify-content: flex-end;
    min-width: 0;
    overflow: hidden;
  }

  .paid-text {
    max-width: 100%;
    text-align: right;
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

  .status-dot {
    background: var(--status-negative);
    border-radius: 50%;
    flex-shrink: 0;
    height: var(--hy-status-dot-size);
    width: var(--hy-status-dot-size);
  }

  .status-dot.active-dot {
    background: var(--status-positive);
  }

  .status-dot.registered-dot {
    background: var(--status-warning);
  }

  .active-text {
    color: var(--status-positive);
  }

  .banned-text {
    color: var(--status-negative);
  }

  .registered-text {
    color: var(--status-warning);
  }

  .type-badge {
    background: var(--surface-accent-strong);
    border-radius: var(--hy-badge-radius);
    color: var(--accent-blue);
    font-size: var(--text-xs);
    font-weight: 700;
    padding: var(--hy-badge-inline-padding);
    white-space: nowrap;
  }

  .type-badge.mint-type {
    background: var(--surface-mint-subtle);
    color: var(--accent-mint);
  }
</style>
