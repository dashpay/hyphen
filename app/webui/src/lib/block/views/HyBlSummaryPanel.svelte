<script lang="ts">
  /** Block summary panel for headline metadata and navigation. */
  import { fmt, tr } from "@repo/hyphen-i18n";
  import { VwPanel } from "@repo/hyphen-views";

  import type { ApiBlockDetail, BlockTag } from "$lib";
  import { getBlockTags, getWebUiConfig } from "$lib";
  import { HyUtilFormatBytes, HyUtilFormatFullDate, HyUtilFormatNumber, HyUtilFormatSelectedAmount } from "$lib/HyUtil";
  import { HyCopyButton, HyPanelHeader, HyScrollBody, HyTag } from "$lib/views";

  interface Props {
    readonly block: ApiBlockDetail;
    readonly onback?: () => void;
  }

  const {
    block,
    onback = (): void => {
      globalThis.history.back();
    },
  }: Props = $props();

  let copiedHash: string | null = $state(null);

  const blockTags = $derived.by((): readonly BlockTag[] => getBlockTags(block, getWebUiConfig()));
  const showChainlockDetails = $derived(getWebUiConfig().features.chainlock.enabled);

  function copyToClipboard(value: string): void {
    void globalThis.navigator.clipboard.writeText(value).then((): void => {
      copiedHash = value;
      globalThis.setTimeout((): void => {
        copiedHash = null;
      }, 1_500);
    });
  }

  function formatBlockHref(hashOrHeight: string | number): string {
    if (typeof hashOrHeight === "number") {
      return "/block?height=" + String(hashOrHeight);
    }

    return "/block?hash=" + hashOrHeight;
  }
</script>

<div class="summary-panel">
  <VwPanel border={true} padding={0} radius="sm" variant="glass">
    <HyPanelHeader {onback} subtitle={tr("Mined on {}", HyUtilFormatFullDate(block.blockTime))} title={tr("Block")} />

    <HyScrollBody container={true}>
      <div class="summary-stack">
        <div>
          <div class="section-title">{tr("Hash")}</div>
          <div class="hy-reference-hash-row">
            <div class="hash-inline">
              <a class="block-link mono hash-full hy-wrap-anywhere" href={formatBlockHref(block.blockHash)}>
                {block.blockHash}
              </a>
              <HyCopyButton
                copied={copiedHash === block.blockHash}
                inline={true}
                label={tr("Copy hash")}
                onpress={(): void => {
                  copyToClipboard(block.blockHash);
                }}
                title={tr("Copy hash")}
              />
            </div>
          </div>
        </div>

        <div>
          <div class="section-title">{tr("Tags")}</div>
          <div class="hy-reference-tags">
            {#if blockTags.length > 0}
              {#each blockTags as tag (tag.label)}
                <HyTag
                  background={tag.tone === "warning"
                    ? "var(--hy-tag-quorum-bg)"
                    : tag.tone === "success"
                      ? "var(--hy-tag-assetlock-bg)"
                      : "var(--hy-tag-default-bg)"}
                  foreground={tag.tone === "warning"
                    ? "var(--hy-tag-quorum-fg)"
                    : tag.tone === "success"
                      ? "var(--hy-tag-assetlock-fg)"
                      : "var(--hy-tag-default-fg)"}
                  label={tag.label}
                />
              {/each}
            {:else}
              <HyTag background="var(--hy-tag-default-bg)" foreground="var(--hy-tag-default-fg)" label={tr("Block")} />
            {/if}
          </div>
        </div>

        <div>
          <div class="section-title">{tr("Details")}</div>
          <div class="detail-table">
            <div class="detail-row detail-row-wide">
              <span class="kv-label">{tr("Prev Block")}</span>
              <span class="kv-value">
                <a class="block-link mono wrap" href={formatBlockHref(block.prevBlockHash)}>
                  {block.prevBlockHash}
                </a>
                <HyCopyButton
                  copied={copiedHash === block.prevBlockHash}
                  inline={true}
                  label={tr("Copy block hash")}
                  onpress={(): void => {
                    copyToClipboard(block.prevBlockHash);
                  }}
                  title={tr("Copy block hash")}
                />
              </span>
            </div>

            <div class="detail-row detail-row-wide">
              <span class="kv-label">Merkle Root</span>
              <span class="kv-value mono wrap">{block.merkle}</span>
            </div>

            {#if block.chainwork != null}
              <div class="detail-row detail-row-wide">
                <span class="kv-label">Chainwork</span>
                <span class="kv-value mono wrap">{block.chainwork}</span>
              </div>
            {/if}

            <div class="detail-row">
              <span class="kv-label">{tr("Block")}</span>
              <span class="kv-value">{fmt("#{}", HyUtilFormatNumber(block.blockHeight))}</span>
              <span class="kv-label">{tr("Time")}</span>
              <span class="kv-value">{HyUtilFormatFullDate(block.blockTime)}</span>
            </div>

            <div class="detail-row">
              <span class="kv-label">{tr("Size")}</span>
              <span class="kv-value">{HyUtilFormatBytes(block.blockSize)}</span>
            </div>

            <div class="detail-row">
              <span class="kv-label">{tr("Difficulty")}</span>
              <span class="kv-value mono">{block.bits}</span>
              <span class="kv-label">Nonce</span>
              <span class="kv-value mono">{HyUtilFormatNumber(block.nonce)}</span>
            </div>

            <div class="detail-row">
              <span class="kv-label">{tr("Version")}</span>
              <span class="kv-value mono">{HyUtilFormatNumber(block.blockVersion)}</span>
              <span class="kv-label">Bits</span>
              <span class="kv-value mono">{block.bits}</span>
            </div>
          </div>
        </div>

        {#if block.cbTx != null}
          <div>
            <div class="section-title">{tr("Payload")}</div>
            <div class="kv-grid">
              <span class="kv-label">{fmt("{} {}", "cbTx", tr("Height"))}</span>
              <span class="kv-value">{HyUtilFormatNumber(block.cbTx.height)}</span>

              {#if block.cbTx.merkleRootMnList != null}
                <span class="kv-label">MN List Merkle</span>
                <span class="kv-value mono wrap">{block.cbTx.merkleRootMnList}</span>
              {/if}

              {#if block.cbTx.merkleFlags != null}
                <span class="kv-label">Merkle Flags</span>
                <span class="kv-value">{HyUtilFormatNumber(block.cbTx.merkleFlags)}</span>
              {/if}

              {#if showChainlockDetails}
                {#if block.cbTx.bestClHeight != null}
                  <span class="kv-label">{tr("Best CL Height")}</span>
                  <span class="kv-value">{HyUtilFormatNumber(block.cbTx.bestClHeight)}</span>
                {/if}

                {#if block.cbTx.bestClSignature != null}
                  <span class="kv-label">{tr("Best CL Signature")}</span>
                  <span class="kv-value mono wrap">{block.cbTx.bestClSignature}</span>
                {/if}
              {/if}

              {#if block.cbTx.creditPoolBalance != null}
                <span class="kv-label">{tr("Credit Pool")}</span>
                <span class="kv-value">
                  {HyUtilFormatSelectedAmount(block.cbTx.creditPoolBalance, getWebUiConfig())}
                </span>
              {/if}
            </div>
          </div>
        {/if}
      </div>
    </HyScrollBody>
  </VwPanel>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .summary-stack {
    display: grid;
    gap: var(--hy-detail-flow-gap);
    padding: var(--hy-detail-section-padding);
  }

  .hash-full {
    color: var(--color-fg);
    font-size: var(--text-sm);
    line-height: 1.4;
  }

  .hash-inline {
    align-items: flex-start;
    display: inline-flex;
    gap: var(--hy-detail-copy-gap);
    min-width: 0;
  }

  .mono {
    font-family: var(--font-mono);
  }

  .kv-grid {
    align-items: baseline;
    display: grid;
    gap: var(--hy-micro-gap) var(--hy-detail-label-gap);
    grid-template-columns: var(--hy-detail-kv-label-width-wide) minmax(0, 1fr);
  }

  .detail-table {
    display: grid;
    gap: var(--hy-micro-gap);
  }

  .detail-row {
    align-items: baseline;
    display: grid;
    gap: var(--hy-micro-gap) var(--hy-detail-label-gap);
    grid-template-columns:
      var(--hy-detail-kv-column-width, var(--hy-detail-kv-label-width-wide))
      minmax(0, 1fr)
      var(--hy-detail-kv-column-width, var(--hy-detail-kv-label-width-wide))
      minmax(0, 1fr);
    min-width: 0;
  }

  .detail-row-wide {
    grid-template-columns: var(--hy-detail-kv-label-width-wide) minmax(0, 1fr);
  }

  .kv-label {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0.03em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  .kv-value {
    align-items: center;
    color: var(--color-fg);
    display: flex;
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    gap: 4px;
    min-width: 0;
    word-break: break-all;
  }

  .wrap {
    overflow-wrap: anywhere;
  }

  .block-link {
    color: var(--accent-blue);
    text-decoration: none;
  }

  .section-title {
    color: var(--accent-blue);
    font-size: var(--text-sm);
    font-weight: 700;
    margin-bottom: var(--hy-detail-tag-gap);
  }

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

  @include view.media-down(tablet) {
    .summary-panel {
      height: auto;
    }

    .summary-panel :global(.panel) {
      height: auto;
    }

    .detail-row {
      grid-template-columns: var(--hy-detail-kv-column-width, var(--hy-detail-kv-label-width-wide)) minmax(0, 1fr);
    }

    .detail-row-wide {
      grid-template-columns: minmax(0, 1fr);
    }

    .kv-grid {
      gap: var(--hy-detail-flow-row-gap);
      grid-template-columns: minmax(0, 1fr);
    }
  }
</style>
