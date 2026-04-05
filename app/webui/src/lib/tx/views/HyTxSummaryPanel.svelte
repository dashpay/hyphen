<script lang="ts">
  /** Transaction summary panel for left-column headline metadata. */
  import { tr } from "@repo/hyphen-i18n";
  import { VwPanel } from "@repo/hyphen-views";

  import type { ApiTxDetail, TransactionRenderer } from "$lib";
  import { getWebUiConfig } from "$lib";
  import { HyUtilFormatFullDate, HyUtilFormatNumber, HyUtilFormatSelectedAmount } from "$lib/HyUtil";
  import { getTransactionDisplay } from "$lib/tx/display";
  import { HyAmountText, HyCopyButton, HyPanelHeader, HyScrollBody } from "$lib/views";
  import HyTxTag from "./HyTxTag.svelte";

  interface Props {
    readonly transaction: ApiTxDetail;
    readonly blockTime: string | null;
    readonly transactionHash: string;
    readonly transactionLabel: string;
    readonly transactionRenderer: TransactionRenderer;
    readonly onback?: () => void;
  }

  const {
    transaction,
    blockTime,
    transactionHash,
    transactionLabel,
    transactionRenderer,
    onback = (): void => {
      globalThis.history.back();
    },
  }: Props = $props();

  let copiedHash: string | null = $state(null);

  const txDisplay = $derived.by(() => getTransactionDisplay(transactionRenderer.type, transaction.blockPos));
  const blockSubtitle = $derived.by((): string => {
    if (blockTime != null) {
      return tr("Mined on {}", HyUtilFormatFullDate(blockTime));
    }

    return transaction.blockHeight != null ? tr("Block #{}", HyUtilFormatNumber(transaction.blockHeight)) : "Mempool";
  });

  function copyToClipboard(value: string): void {
    void globalThis.navigator.clipboard.writeText(value).then((): void => {
      copiedHash = value;
      globalThis.setTimeout((): void => {
        copiedHash = null;
      }, 1_500);
    });
  }
</script>

<div class="summary-panel">
  <VwPanel border={true} padding={0} radius="sm" variant="glass">
    <HyPanelHeader {onback} subtitle={blockSubtitle} title={tr("Transaction")} />

    <HyScrollBody container={true}>
      <div class="hy-reference-stack tx-summary-stack">
      <div>
        <div class="section-title">{tr("Hash")}</div>
        <div class="hy-reference-hash-row">
          <span class="mono hash-full hy-wrap-anywhere">{transactionHash}</span>
          <HyCopyButton
            copied={copiedHash === transactionHash}
            inline={true}
            label={tr("Copy transaction ID")}
            onpress={(): void => {
              copyToClipboard(transactionHash);
            }}
            title={tr("Copy txid")}
          />
        </div>
      </div>

      <div>
        <div class="section-title">{tr("Tags")}</div>
        <div class="hy-reference-tags">
          <HyTxTag label={transactionLabel} tone={txDisplay.tone} />
          {#if transaction.live.tags?.islock === true}
            <HyTxTag label="ISLock" tone="proupserv" />
          {/if}
        </div>
      </div>

      <div class="hy-reference-stat-grid">
        <div class="hy-reference-stat-card">
          <span class="hy-reference-stat-label">{tr("Amount")}</span>
          <span class="hy-reference-stat-value">
            <HyAmountText value={HyUtilFormatSelectedAmount(transaction.valueOut, getWebUiConfig())} />
          </span>
        </div>
        <div class="hy-reference-stat-card">
          <span class="hy-reference-stat-label">{tr("Fee")}</span>
          <span class="hy-reference-stat-value">
            <HyAmountText value={HyUtilFormatSelectedAmount(transaction.fee ?? 0, getWebUiConfig())} />
          </span>
        </div>
      </div>
      </div>
    </HyScrollBody>
  </VwPanel>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .summary-panel {
    height: 100%;
    min-width: 0;
  }

  .summary-panel :global(.panel) {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    height: 100%;
    min-width: 0;
    overflow: hidden;
  }

  .tx-summary-stack {
    padding: var(--hy-detail-section-padding);
  }

  .hash-full {
    color: var(--color-fg);
    font-size: var(--text-sm);
    line-height: 1.4;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .section-title {
    color: var(--accent-blue);
    font-size: var(--text-sm);
    font-weight: 700;
    margin-bottom: var(--hy-detail-tag-gap);
  }

  @include view.media-down(tablet) {
    .summary-panel {
      height: auto;
    }

    .summary-panel :global(.panel) {
      height: auto;
    }
  }
</style>
