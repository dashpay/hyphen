<script lang="ts">
  /** Compact transaction list for dashboard and block contexts. */
  import { tr } from "@repo/hyphen-i18n";
  import { VwDivider, VwText } from "@repo/hyphen-views";
  import type { TransactionTone } from "$lib/tx/display";

  import HyTxTag from "./HyTxTag.svelte";
  import { HyListPanel, HyScrollBody } from "$lib/views";

  interface TransactionRow {
    readonly hash: string;
    readonly txid: string;
    readonly detail: string;
    readonly detailTone: TransactionTone;
    readonly amount: string;
    readonly metaLabel: string | null;
    readonly flow: string | null;
    readonly blockLabel: string | null;
    readonly sizeLabel: string;
  }

  interface Props {
    readonly mobileExpand?: boolean;
    readonly title?: string;
    readonly transactions: readonly TransactionRow[];
    readonly onviewall?: () => void;
    readonly ontxclick?: (txid: string) => void;
  }

  const {
    mobileExpand = false,
    title = tr("Latest Transactions"),
    transactions,
    onviewall,
    ontxclick,
  }: Props = $props();
</script>

<div class:mobile-expand={mobileExpand} class="tx-panel">
  <HyListPanel {title}>
    {#snippet action()}
      {#if onviewall != null}
        <button class="view-all-btn" onclick={onviewall} type="button">
          <VwText color="accent-blue" size="sm" weight="semibold">{tr("View All")}</VwText>
        </button>
      {/if}
    {/snippet}
    <HyScrollBody>
      {#each transactions as transaction, index (transaction.hash)}
        <div
          class="tx-row row-animate"
          onclick={(): void => {
            ontxclick?.(transaction.txid);
          }}
          onkeydown={(event: KeyboardEvent): void => {
            if (event.key === "Enter") {
              ontxclick?.(transaction.txid);
            }
          }}
          role="button"
          tabindex="0"
        >
          <div class="hy-compact-row-top">
            <div class="hy-compact-row-main">
              <div class="hy-compact-row-detail detail-row-top">
                <HyTxTag label={transaction.detail} tone={transaction.detailTone} />
                <VwText color="accent-blue" mono={true} size="sm" weight="medium">{transaction.hash}</VwText>
              </div>
            </div>
            <div class="hy-compact-row-side">
              <VwText mono={true} size="base" weight="bold">{transaction.amount}</VwText>
            </div>
          </div>
          <div class="hy-compact-row-bottom">
            <div class="hy-compact-row-main">
              <div class="hy-compact-row-meta">
                {#if transaction.metaLabel != null}
                  <VwText color="muted" size="xs" weight="medium">{transaction.metaLabel}</VwText>
                {/if}
                {#if transaction.metaLabel != null && transaction.blockLabel != null}
                  <span aria-hidden="true" class="hy-compact-row-meta-dot"></span>
                {/if}
                {#if transaction.blockLabel != null}
                  <VwText color="muted" mono={true} size="xs" weight="medium">{transaction.blockLabel}</VwText>
                {/if}
              </div>
            </div>
            <div class="hy-compact-row-side">
              <VwText color="muted" mono={true} size="xs" weight="medium">{transaction.sizeLabel}</VwText>
            </div>
          </div>
          {#if transaction.flow != null}
            <div class="hy-compact-row-flow">
              <div class="hy-compact-row-main flow-line">
                <VwText color="tertiary" mono={true} size="xs">{transaction.flow}</VwText>
              </div>
              <div aria-hidden="true" class="hy-compact-row-side hy-compact-row-spacer"></div>
            </div>
          {/if}
        </div>
        {#if index < transactions.length - 1}
          <VwDivider opacity={0.08} />
        {/if}
      {/each}
    </HyScrollBody>
  </HyListPanel>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .tx-panel {
    height: 100%;
    min-width: 0;
  }
  .view-all-btn {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
  }

  .tx-row {
    cursor: pointer;
    padding: var(--hy-card-list-row-padding);
    transition: background var(--duration-fast) var(--ease-default);
  }

  .hy-compact-row-bottom {
    margin-top: 4px;
  }

  .hy-compact-row-flow {
    margin-top: 8px;
  }

  .tx-row:hover {
    background: var(--hy-list-row-hover);
  }

  .tx-row:focus-visible {
    background: var(--hy-list-row-hover);
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }

  .flow-line {
    overflow: hidden;
  }

  .flow-line :global(.text) {
    display: block;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  /* --hy-breakpoint-tablet */
  @include view.container-down(tablet) {
    .tx-panel.mobile-expand,
    .tx-panel.mobile-expand :global(.panel) {
      height: auto;
    }

    .tx-panel:not(.mobile-expand),
    .tx-panel:not(.mobile-expand) :global(.panel) {
      height: 60vh;
    }
  }
</style>
