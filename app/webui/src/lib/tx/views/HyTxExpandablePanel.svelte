<script lang="ts">
  /** Shared expandable transaction-row panel for block and address detail views. */
  import { VwDivider, VwText, VwThrobber } from "@repo/hyphen-views";
  import IconChevronDown from "virtual:icons/mdi/chevron-down";
  import IconChevronRight from "virtual:icons/mdi/chevron-right";

  import type { ApiTxDetail, TransactionPayloadField, TransactionRenderer } from "$lib";
  import { genericPayload, getTransaction, getTransactionRenderer, getWebUiConfig } from "$lib";
  import { HyAnFadeIn, HyAnReveal } from "$lib/anim";
  import { HyUtilFormatBytes, HyUtilFormatSelectedAmount } from "$lib/HyUtil";
  import type { HyTxExpandableRow } from "$lib/tx/HyTxUtil";
  import { HyAmountText, HyDetailSection, HyListPanel, HyScrollBody } from "$lib/views";

  import HyTxIoColumns from "./HyTxIoColumns.svelte";
  import HyTxPayloadTable from "./HyTxPayloadTable.svelte";
  import HyTxTag from "./HyTxTag.svelte";

  interface Props {
    readonly fullHeight?: boolean;
    readonly hasMore?: boolean;
    readonly loadingMore?: boolean;
    readonly ioAmountLayout?: "inline" | "stacked";
    readonly hidePayloadForTypes?: readonly string[];
    readonly onloadmore?: (() => unknown) | undefined;
    readonly ontxclick: ((txid: string) => void) | undefined;
    readonly rows: readonly HyTxExpandableRow[];
    readonly scrollMode?: "page" | "self";
    readonly subtitle: string;
    readonly title: string;
  }

  interface ExpandedTransactionState {
    readonly detail: ApiTxDetail;
    readonly payloadFields: readonly TransactionPayloadField[];
    readonly renderer: TransactionRenderer;
  }

  const {
    fullHeight = true,
    hasMore = false,
    loadingMore = false,
    hidePayloadForTypes = [],
    ioAmountLayout = "inline",
    onloadmore,
    ontxclick,
    rows,
    scrollMode = "self",
    subtitle,
    title,
  }: Props = $props();

  let expandedTxid = $state<string | null>(null);
  let expandedTransactions = $state<Record<string, ExpandedTransactionState | undefined>>({});
  let loadingTxids = $state<Record<string, boolean | undefined>>({});

  function getExpandedState(txid: string): ExpandedTransactionState | null {
    return expandedTransactions[txid] ?? null;
  }

  function hasExpandedIo(expanded: ExpandedTransactionState): boolean {
    if (expanded.renderer.renderMode === "payload") {
      return false;
    }

    return expanded.detail.vin.length > 0 || expanded.detail.vout.length > 0;
  }

  function hasExpandedPayload(expanded: ExpandedTransactionState): boolean {
    if (expanded.renderer.renderMode === "payment") {
      return false;
    }

    if (hidePayloadForTypes.includes(expanded.renderer.type)) {
      return false;
    }

    return expanded.payloadFields.length > 0;
  }

  async function toggleExpanded(row: HyTxExpandableRow): Promise<void> {
    if (expandedTxid === row.txid) {
      expandedTxid = null;
      return;
    }

    expandedTxid = row.txid;

    if (expandedTransactions[row.txid] != null || loadingTxids[row.txid] === true) {
      return;
    }

    loadingTxids = { ...loadingTxids, [row.txid]: true };

    try {
      const detail = await getTransaction(row.txid);

      if (detail == null) {
        return;
      }

      const renderer = getTransactionRenderer(detail);
      const payloadFields = renderer.parsePayload?.(detail) ?? genericPayload(detail);

      expandedTransactions = {
        ...expandedTransactions,
        [row.txid]: {
          detail,
          payloadFields,
          renderer,
        },
      };
    } finally {
      loadingTxids = { ...loadingTxids, [row.txid]: false };
    }
  }
  function getAmountClass(row: HyTxExpandableRow): string {
    if (row.amountTone === "positive") {
      return "amount-positive";
    }

    if (row.amountTone === "negative") {
      return "amount-negative";
    }

    return "";
  }
</script>

<div class="tx-panel">
  <HyListPanel {fullHeight} {subtitle} {title}>
    <HyScrollBody {hasMore} {loadingMore} {onloadmore} {scrollMode}>
      {#each rows as row, index (row.txid)}
        <div class="tx-entry">
          <div
            class="tx-row row-animate"
            onclick={(): void => {
              void toggleExpanded(row);
            }}
            onkeydown={(event: KeyboardEvent): void => {
              if (event.key === "Enter" || event.key === " ") {
                event.preventDefault();
                void toggleExpanded(row);
              }
            }}
            role="button"
            tabindex="0"
          >
            <div class="hy-compact-row-shell">
              <div class="hy-compact-row-expand-col">
                {#if expandedTxid === row.txid}
                  <IconChevronDown aria-hidden="true" class="icon" />
                {:else}
                  <IconChevronRight aria-hidden="true" class="icon" />
                {/if}
              </div>

              <div class="hy-compact-row-content">
                <div class="hy-compact-row-top">
                  <div class="hy-compact-row-main">
                    <div class="hy-compact-row-detail detail-row-top">
                      <HyTxTag label={row.label} tone={row.tone} />
                      <a
                        class="hy-compact-row-link"
                        href={"/transaction?hash=" + encodeURIComponent(row.txid)}
                        onclick={(event: MouseEvent): void => {
                          event.stopPropagation();
                          ontxclick?.(row.txid);
                        }}
                      >
                        <VwText color="accent-blue" mono={true} size="sm" weight="medium">{row.txid}</VwText>
                      </a>
                    </div>
                  </div>
                  <div class="hy-compact-row-side">
                    <VwText mono={true} size="base" weight="bold">
                      <span class={getAmountClass(row)}>
                        {#if row.amountPrefix != null}
                          <span>{row.amountPrefix}</span>
                        {/if}
                        <HyAmountText value={HyUtilFormatSelectedAmount(row.valueOut, getWebUiConfig())} />
                      </span>
                    </VwText>
                  </div>
                </div>

                <div class="hy-compact-row-bottom">
                  <div class="hy-compact-row-main">
                    <div class="hy-compact-row-meta">
                      {#if row.metaLabel != null}
                        <VwText color="muted" size="xs" weight="medium">{row.metaLabel}</VwText>
                      {/if}
                      {#if row.metaLabel != null}
                        <span aria-hidden="true" class="hy-compact-row-meta-dot"></span>
                      {/if}
                      <VwText color="muted" mono={true} size="xs" weight="medium">{row.metaValue}</VwText>
                    </div>
                  </div>
                  <div class="hy-compact-row-side">
                    <VwText color="muted" mono={true} size="xs" weight="medium">
                      {HyUtilFormatBytes(row.size)}
                    </VwText>
                  </div>
                </div>
              </div>
            </div>
          </div>

          {#if expandedTxid === row.txid}
            <div class="expanded-body" transition:HyAnReveal>
              {#if loadingTxids[row.txid] === true}
                <div class="expanded-loading" transition:HyAnFadeIn>
                  <VwThrobber />
                </div>
              {:else}
                {@const expanded = getExpandedState(row.txid)}
                {#if expanded != null}
                  {#if hasExpandedIo(expanded)}
                    <HyDetailSection>
                      <HyTxIoColumns
                        amountLayout={ioAmountLayout}
                        transaction={expanded.detail}
                        transactionRenderer={expanded.renderer}
                      />
                    </HyDetailSection>
                  {/if}
                  {#if hasExpandedPayload(expanded)}
                    <HyDetailSection title="Payload" titleSize="xs">
                      <HyTxPayloadTable
                        payloadFields={expanded.payloadFields}
                        transactionRenderer={expanded.renderer}
                      />
                    </HyDetailSection>
                  {/if}
                {/if}
              {/if}
            </div>
          {/if}

          {#if index < rows.length - 1}
            <VwDivider opacity={0.08} />
          {/if}
        </div>
      {/each}
    </HyScrollBody>
  </HyListPanel>
</div>

<style>
  .tx-panel {
    height: 100%;
    min-width: 0;
  }

  .tx-entry {
    display: flex;
    flex-direction: column;
    min-width: 0;
  }

  .tx-row {
    cursor: pointer;
    padding: var(--hy-card-list-row-padding);
    transition: background var(--duration-fast) var(--ease-default);
  }

  .tx-row:hover {
    background: var(--hy-list-row-hover);
  }

  .tx-row:focus-visible {
    background: var(--hy-list-row-hover);
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }

  .detail-row-top {
    min-width: 0;
  }

  .hy-compact-row-bottom {
    margin-top: 4px;
  }

  .amount-positive {
    color: var(--status-positive);
  }

  .amount-negative {
    color: var(--status-negative);
  }

  .expanded-body {
    border-top: 1px solid var(--glass-border);
    flex: 0 0 auto;
  }

  .expanded-loading {
    align-items: center;
    display: flex;
    justify-content: center;
    min-height: 120px;
  }
</style>
