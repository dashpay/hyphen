<script lang="ts">
  /** Transaction detail panel for inputs, outputs, and payload data. */
  import { fmt, tr } from "@repo/hyphen-i18n";
  import { VwPanel } from "@repo/hyphen-views";

  import type { ApiTxDetail, TransactionPayloadField, TransactionRenderer } from "$lib";
  import { HyUtilFormatBytes, HyUtilFormatFullDate, HyUtilFormatNumber } from "$lib/HyUtil";
  import { HyPanelHeader, HyScrollBody } from "$lib/views";
  import HyTxIoColumns from "./HyTxIoColumns.svelte";
  import HyTxPayloadTable from "./HyTxPayloadTable.svelte";

  interface Props {
    readonly blockTime: string | null;
    readonly fullHeight?: boolean;
    readonly transaction: ApiTxDetail;
    readonly payloadFields: readonly TransactionPayloadField[];
    readonly scrollMode?: "page" | "self";
    readonly transactionRenderer: TransactionRenderer;
  }

  const {
    blockTime,
    fullHeight = true,
    transaction,
    payloadFields,
    scrollMode = "self",
    transactionRenderer,
  }: Props = $props();

  function formatTransactionSize(size: number): string {
    return size >= 1_000 ? HyUtilFormatBytes(size) : fmt("{} B", size);
  }

  function formatLockTime(lockTime: number): string {
    return lockTime === 0 ? tr("Unlocked") : HyUtilFormatNumber(lockTime);
  }

  function formatBlockHref(blockHeight: number): string {
    return "/block?height=" + String(blockHeight);
  }

  function hasIoColumns(): boolean {
    if (transactionRenderer.renderMode === "payload") {
      return false;
    }

    return transaction.vin.length > 0 || transaction.vout.length > 0;
  }

  function hasPayloadFields(): boolean {
    if (transactionRenderer.renderMode === "payment") {
      return false;
    }

    return payloadFields.length > 0;
  }
</script>

<div class:auto-height={!fullHeight} class="tx-detail-panel">
  <VwPanel border={true} padding={0} radius="sm" variant="glass">
    <HyPanelHeader showBack={false} title={tr("Details")} />

    <HyScrollBody container={true} {scrollMode}>
      <div class="tx-detail-stack">
        <div class="tx-detail-grid">
          <div class="hy-detail-kv-column">
            <span class="hy-detail-kv-label">{tr("Size")}</span>
            <span class="hy-detail-kv-value">{formatTransactionSize(transaction.size)}</span>

            <span class="hy-detail-kv-label">{tr("Version")}</span>
            <span class="hy-detail-kv-value">{HyUtilFormatNumber(transaction.version)}</span>

            <span class="hy-detail-kv-label">Locktime</span>
            <span class="hy-detail-kv-value mono truncate">{formatLockTime(transaction.lockTime)}</span>
          </div>

          <div class="hy-detail-kv-column">
            {#if blockTime != null}
              <span class="hy-detail-kv-label">{tr("Block Time")}</span>
              <span class="hy-detail-kv-value">{HyUtilFormatFullDate(blockTime)}</span>
            {:else}
              <span class="hy-detail-kv-label">{tr("Block")}</span>
              {#if transaction.blockHeight != null}
                <a class="hy-detail-kv-value block-link" href={formatBlockHref(transaction.blockHeight)}>
                  {fmt("#{}", HyUtilFormatNumber(transaction.blockHeight))}
                </a>
              {:else}
                <span class="hy-detail-kv-value">Mempool</span>
              {/if}
            {/if}

            <span class="hy-detail-kv-label">{tr("Inputs")}</span>
            <span class="hy-detail-kv-value">{transaction.vin.length}</span>

            <span class="hy-detail-kv-label">{tr("Outputs")}</span>
            <span class="hy-detail-kv-value">{transaction.vout.length}</span>
          </div>
        </div>

        {#if hasIoColumns()}
          <HyTxIoColumns amountLayout="stacked" {transaction} {transactionRenderer} />
        {/if}

        {#if hasPayloadFields()}
          <div>
            <div class="section-title">{tr("Payload")}</div>
            <HyTxPayloadTable {payloadFields} {transactionRenderer} />
          </div>
        {/if}
      </div>
    </HyScrollBody>
  </VwPanel>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .tx-detail-panel {
    height: 100%;
    min-width: 0;
  }

  .tx-detail-panel.auto-height {
    height: auto;
  }

  .tx-detail-panel :global(.panel) {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    height: 100%;
    min-width: 0;
    overflow: hidden;
  }

  .tx-detail-panel.auto-height :global(.panel) {
    height: auto;
  }

  .tx-detail-stack {
    display: grid;
    gap: var(--hy-detail-flow-gap);
    padding: var(--hy-detail-section-padding);
  }

  .tx-detail-grid {
    display: grid;
    gap: var(--hy-micro-gap) var(--hy-detail-column-gap);
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  }

  .block-link {
    color: var(--accent-blue);
    text-decoration: none;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .truncate {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .section-title {
    color: var(--accent-blue);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.04em;
    margin-bottom: var(--hy-detail-tag-gap);
    text-transform: uppercase;
  }

  @include view.container-down(narrow-phone) {
    .tx-detail-grid {
      grid-template-columns: 1fr;
    }
  }
</style>
