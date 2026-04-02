<script lang="ts">
  /** Validator history panel for recent activity and tx links. */
  import { fmt, tr } from "@repo/hyphen-i18n";
  import { VwPanel } from "@repo/hyphen-views";

  import type { ApiTxDetail, ApiValidatorDetail, TransactionPayloadField, TransactionRenderer } from "$lib";
  import { genericPayload, getTransactionRenderer } from "$lib";
  import { HyUtilFormatNumber } from "$lib/HyUtil";
  import { getTransactionDisplay } from "$lib/tx/display";
  import { HyVlFormatHistoryCount } from "$lib/validator/HyVlUtil";
  import { HyCopyButton, HyPanelHeader, HyScrollBody } from "$lib/views";

  interface Props {
    readonly fullHeight?: boolean;
    readonly scrollMode?: "page" | "self";
    readonly validator: ApiValidatorDetail;
  }

  const { fullHeight = true, scrollMode = "self", validator }: Props = $props();

  let copiedHash = $state<string | null>(null);
  let expandedTxids = $state<ReadonlySet<string>>(new Set<string>());

  function copyToClipboard(value: string, event?: MouseEvent): void {
    event?.stopPropagation();
    void globalThis.navigator.clipboard.writeText(value).then((): void => {
      copiedHash = value;
      globalThis.setTimeout((): void => {
        copiedHash = null;
      }, 1_500);
    });
  }

  function toggleExpanded(txid: string, event: MouseEvent): void {
    event.stopPropagation();
    const nextExpanded = new Set(expandedTxids);
    if (nextExpanded.has(txid)) {
      nextExpanded.delete(txid);
    } else {
      nextExpanded.add(txid);
    }

    expandedTxids = nextExpanded;
  }

  function normalizeHistoryType(txType: string): string {
    switch (txType) {
      case "pro_reg_tx":
        return "provider_register";
      case "pro_up_serv_tx":
        return "provider_update_service";
      case "pro_up_reg_tx":
        return "provider_update_registrar";
      case "pro_up_rev_tx":
        return "provider_update_revoke";
      default:
        return txType;
    }
  }

  function getHistoryRenderer(txType: string, payload: unknown): TransactionRenderer {
    return getTransactionRenderer({
      blockHeight: null,
      blockPos: null,
      fee: null,
      inputCount: 0,
      lockTime: 0,
      outputCount: 0,
      payload,
      size: 0,
      live: { tags: null },
      tags: { mined: true, coinbase: false },
      txType: normalizeHistoryType(txType),
      txid: "",
      valueOut: 0,
      version: 0,
      vin: [],
      vout: [],
    } satisfies ApiTxDetail);
  }

  function getHistoryPayloadFields(txType: string, payload: unknown): readonly TransactionPayloadField[] {
    const renderer = getHistoryRenderer(txType, payload);
    const syntheticDetail = {
      blockHeight: null,
      blockPos: null,
      fee: null,
      inputCount: 0,
      lockTime: 0,
      outputCount: 0,
      payload,
      size: 0,
      live: { tags: null },
      tags: { mined: true, coinbase: false },
      txType: renderer.type,
      txid: "",
      valueOut: 0,
      version: 0,
      vin: [],
      vout: [],
    } satisfies ApiTxDetail;

    return renderer.parsePayload?.(syntheticDetail) ?? genericPayload(syntheticDetail);
  }
</script>

<div class:auto-height={!fullHeight} class="history-panel">
  <VwPanel border={true} padding={0} radius="sm" variant="glass">
    <HyPanelHeader showBack={false} subtitle={HyVlFormatHistoryCount(validator.history.length)} title={tr("History")} />

    <HyScrollBody container={true} {scrollMode}>
      <div class="hy-reference-history-list">
        {#each validator.history as entry, index (entry.txid)}
          {@const expanded = expandedTxids.has(entry.txid)}
          {@const renderer = getHistoryRenderer(entry.txType, entry.payload)}
          {@const txDisplay = getTransactionDisplay(renderer.type)}
          {@const payloadFields = getHistoryPayloadFields(entry.txType, entry.payload)}
          <div class:expanded class="tx-entry">
            <div
              class="tx-row"
              onclick={(event: MouseEvent): void => {
                toggleExpanded(entry.txid, event);
              }}
              onkeydown={(event: KeyboardEvent): void => {
                if (event.key === "Enter") {
                  toggleExpanded(entry.txid, event as unknown as MouseEvent);
                }
              }}
              role="button"
              tabindex="0"
            >
              <svg
                class="chevron"
                fill="none"
                height="12"
                stroke="currentColor"
                stroke-width="2"
                viewBox="0 0 24 24"
                width="12"
              >
                <polyline points="9 6 15 12 9 18"></polyline>
              </svg>
              <span class="tx-idx muted">{index + 1}</span>
              <div class="tx-hash-col">
                <a
                  class="mono tx-hash tx-link"
                  href={"/transaction?hash=" + encodeURIComponent(entry.txid)}
                  onclick={(event: MouseEvent): void => {
                    event.stopPropagation();
                  }}
                >
                  {entry.txid}
                </a>
                <HyCopyButton
                  copied={copiedHash === entry.txid}
                  inline={true}
                  label={tr("Copy transaction ID")}
                  onpress={(event: MouseEvent): void => {
                    copyToClipboard(entry.txid, event);
                  }}
                  title={tr("Copy txid")}
                />
              </div>
              <span class={"tx-type-badge tone-" + txDisplay.tone}>{renderer.label}</span>
              <span class="mono tx-block">{fmt("#{}", HyUtilFormatNumber(entry.blockHeight))}</span>
            </div>
            {#if expanded}
              <div class="tx-detail-body">
                {#each payloadFields as field (field.label)}
                  <div class="kv-row">
                    <span class="kv-label">{field.label}</span>
                    <span class:mono={field.mono ?? false} class="kv-value">{field.value}</span>
                  </div>
                {/each}
                <a class="open-tx-btn" href={"/transaction?hash=" + encodeURIComponent(entry.txid)}>
                  {tr("View Transaction")}
                </a>
              </div>
            {/if}
          </div>
        {/each}
      </div>
    </HyScrollBody>
  </VwPanel>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .history-panel {
    height: 100%;
    min-width: 0;
  }

  .history-panel.auto-height {
    height: auto;
  }

  .history-panel :global(.panel) {
    display: grid;
    grid-template-rows: auto minmax(0, 1fr);
    height: 100%;
    min-width: 0;
    overflow: hidden;
  }

  .history-panel.auto-height :global(.panel) {
    height: auto;
  }

  .tx-row {
    align-items: center;
    cursor: pointer;
    display: grid;
    gap: var(--hy-detail-tag-gap);
    grid-template-columns: auto auto minmax(0, 1fr) auto auto;
    padding: var(--hy-browser-validator-row-padding);
  }

  .tx-row:hover,
  .tx-row:focus-visible {
    background: var(--overlay-subtle);
  }

  .tx-row:focus-visible {
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }

  .chevron {
    color: var(--text-muted);
    flex-shrink: 0;
    transition: transform var(--duration-fast) var(--ease-default);
  }

  .expanded .chevron {
    transform: rotate(90deg);
  }

  .tx-idx,
  .tx-block {
    white-space: nowrap;
  }

  .tx-hash-col {
    align-items: center;
    display: flex;
    gap: var(--hy-detail-tag-gap);
    min-width: 0;
  }

  .tx-hash-col :global(.copy-button.inline) {
    color: var(--text-muted);
    opacity: 0.7;
  }

  .tx-hash {
    color: var(--accent-blue);
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .tx-link {
    text-decoration: none;
  }

  .tx-type-badge {
    border-radius: var(--hy-badge-radius);
    font-size: var(--text-xs);
    font-weight: 700;
    padding: var(--hy-badge-inline-padding);
    white-space: nowrap;
  }

  .tone-default {
    background: var(--surface-accent-strong);
    color: var(--accent-blue);
  }

  .tone-proreg {
    background: var(--hy-tag-proreg-bg);
    color: var(--hy-tag-proreg-fg);
  }

  .tone-proupserv {
    background: var(--hy-tag-proupserv-bg);
    color: var(--hy-tag-proupserv-fg);
  }

  .tone-proupreg {
    background: var(--hy-tag-proupreg-bg);
    color: var(--hy-tag-proupreg-fg);
  }

  .tone-prouprev {
    background: var(--hy-tag-prouprev-bg);
    color: var(--hy-tag-prouprev-fg);
  }

  .tx-detail-body {
    display: grid;
    gap: var(--hy-detail-flow-gap);
    padding: var(--hy-browser-validator-row-padding);
    padding-top: var(--hy-detail-tag-gap);
  }

  .kv-row {
    align-items: start;
    display: grid;
    gap: var(--hy-detail-flow-gap);
    grid-template-columns: var(--hy-detail-kv-label-width-validator) minmax(0, 1fr);
    min-width: 0;
  }

  .kv-label {
    color: var(--text-muted);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .kv-value,
  .open-tx-btn,
  .mono,
  .muted {
    font-size: var(--text-sm);
  }

  .kv-value,
  .open-tx-btn {
    color: var(--color-fg);
  }

  .kv-value {
    min-width: 0;
    overflow-wrap: anywhere;
    word-break: break-word;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .muted {
    color: var(--text-muted);
  }

  .open-tx-btn {
    align-items: center;
    background: var(--surface-raised);
    border: 1px solid var(--overlay-strong);
    border-radius: var(--radius-pill);
    color: var(--accent-blue);
    display: inline-flex;
    font-size: var(--text-xs);
    font-weight: 600;
    justify-content: center;
    min-height: 28px;
    padding: 0 12px;
    text-decoration: none;
    width: fit-content;
  }

  /* --hy-breakpoint-laptop */
  @include view.container-down(laptop) {
    .tx-row {
      grid-template-columns: auto minmax(0, 1fr) auto auto;
    }

    .tx-idx {
      display: none;
    }
  }
</style>
