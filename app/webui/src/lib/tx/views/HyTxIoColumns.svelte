<script lang="ts">
  /** IO columns for transaction-style From and To sections. */
  import { tr } from "@repo/hyphen-i18n";
  import { VwText } from "@repo/hyphen-views";

  import type { ApiTxDetail, ApiTxInput, ApiTxOutput, TransactionRenderer } from "$lib";
  import { getWebUiConfig } from "$lib";
  import { HyUtilFormatSelectedAmount } from "$lib/HyUtil";
  import { HyAmountText } from "$lib/views";

  import HyTxTag from "./HyTxTag.svelte";

  interface Props {
    readonly amountLayout?: "inline" | "stacked";
    readonly transaction: ApiTxDetail;
    readonly transactionRenderer: TransactionRenderer;
  }

  const { amountLayout = "inline", transaction, transactionRenderer }: Props = $props();

  function formatInputLabel(input: ApiTxInput): string {
    if (input.coinbase != null) {
      return tr("Reward");
    }

    if (input.address != null) {
      return input.address;
    }

    if (input.txid != null && input.vout != null) {
      return input.txid + ":" + String(input.vout);
    }

    return tr("Unknown input");
  }

  function formatOutputLabel(output: ApiTxOutput): string {
    return output.address ?? tr("Unknown output");
  }

  function formatAddressHref(address: string | null): string | undefined {
    if (address == null) {
      return undefined;
    }

    return "/address?id=" + encodeURIComponent(address);
  }
</script>

{#if transactionRenderer.renderMode !== "payload"}
  <div class="hy-detail-io-columns">
    {#if transaction.vin.length > 0}
      <div class="io-panel">
        <div class="io-title">
          <VwText color="accent-blue" size="xs" weight="bold">{tr("From ({})", transaction.vin.length)}</VwText>
        </div>
        <div class="io-list">
          {#each transaction.vin as input, index (index)}
            <div class="io-item">
              <span class="io-idx">{input.n}</span>
              <div class:io-body-stacked={amountLayout === "stacked"} class="io-body">
                {#if input.coinbase != null}
                  <HyTxTag label={tr("Reward")} tone="coinbase" />
                {:else if formatAddressHref(input.address) != null}
                  <a class="mono io-addr hy-truncate-text" href={formatAddressHref(input.address)}>
                    {formatInputLabel(input)}
                  </a>
                {:else}
                  <span class="mono io-addr hy-truncate-text">{formatInputLabel(input)}</span>
                {/if}
                {#if input.value != null}
                  <span class="io-value">
                    <HyAmountText value={HyUtilFormatSelectedAmount(input.value, getWebUiConfig())} />
                  </span>
                {/if}
              </div>
            </div>
          {/each}
        </div>
      </div>
    {/if}

    {#if transaction.vout.length > 0}
      <div class="io-panel">
        <div class="io-title">
          <VwText color="accent-blue" size="xs" weight="bold">{tr("To ({})", transaction.vout.length)}</VwText>
        </div>
        <div class="io-list">
          {#each transaction.vout as output (output.n)}
            <div class="io-item">
              <span class="io-idx">{output.n}</span>
              <div class:io-body-stacked={amountLayout === "stacked"} class="io-body">
                {#if formatAddressHref(output.address) != null}
                  <a class="mono io-addr hy-truncate-text" href={formatAddressHref(output.address)}>
                    {formatOutputLabel(output)}
                  </a>
                {:else}
                  <span class="mono io-addr hy-truncate-text">{formatOutputLabel(output)}</span>
                {/if}
                <span class="io-value">
                  <HyAmountText value={HyUtilFormatSelectedAmount(output.value, getWebUiConfig())} />
                </span>
              </div>
            </div>
          {/each}
        </div>
      </div>
    {/if}
  </div>
{/if}

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .hy-detail-io-columns {
    align-items: start;
    display: grid;
    gap: var(--space-4);
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
  }

  .io-panel {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    min-width: 0;
  }

  .io-title {
    margin-bottom: var(--hy-detail-tag-gap);
  }

  .io-title :global(.text) {
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .io-list {
    display: flex;
    flex-direction: column;
  }

  .io-item {
    align-items: flex-start;
    border-bottom: 1px solid var(--overlay-subtle);
    display: flex;
    gap: var(--hy-detail-flow-row-gap);
    padding: var(--hy-detail-inline-padding);
  }

  .io-item:last-child {
    border-bottom: none;
  }

  .io-idx {
    color: var(--text-muted);
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    text-align: right;
    width: var(--hy-detail-io-index-width);
  }

  .io-body {
    align-items: flex-start;
    display: flex;
    flex: 1;
    gap: var(--hy-detail-flow-row-gap);
    min-width: 0;
  }

  .io-body-stacked {
    align-items: start;
    display: grid;
    gap: 4px;
    grid-template-columns: minmax(0, 1fr);
  }

  .io-addr {
    color: var(--accent-blue);
    flex: 1;
    font-size: var(--text-sm);
    line-height: 1.4;
    min-width: 0;
    text-decoration: none;
  }

  .io-value {
    color: var(--color-fg);
    display: inline-flex;
    flex-shrink: 0;
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-weight: 600;
    max-width: 100%;
    white-space: nowrap;
  }

  .io-body-stacked .io-value {
    justify-self: start;
  }

  .io-body-stacked .io-addr {
    display: block;
    max-width: 100%;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .mono {
    font-family: var(--font-mono);
  }

  @include view.container-down(narrow-phone) {
    .hy-detail-io-columns {
      gap: var(--hy-detail-grid-gap);
    }

    .io-item {
      gap: var(--hy-detail-tag-gap);
    }

    .io-body {
      gap: var(--hy-detail-tag-gap);
    }
  }
</style>
