<script lang="ts">
  /** Compact block list for dashboard and history contexts. */
  import { tr } from "@repo/hyphen-i18n";
  import { VwDivider, VwText } from "@repo/hyphen-views";

  import { HyBlFormatHeightLabel, HyBlFormatTransactionCount } from "$lib/block/HyBlUtil";
  import { HyListPanel, HyScrollBody } from "$lib/views";

  interface BlockRow {
    readonly height: number;
    readonly hash: string;
    readonly time: string;
    readonly txCount: number;
    readonly size: string;
  }

  interface Props {
    readonly blocks: readonly BlockRow[];
    readonly mobileExpand?: boolean;
    readonly onviewall?: () => void;
    readonly onblockclick?: (hash: string) => void;
  }

  const { blocks, mobileExpand = false, onviewall, onblockclick }: Props = $props();
</script>

<div class:mobile-expand={mobileExpand} class="block-panel">
  <HyListPanel title={tr("Latest Blocks")}>
    {#snippet action()}
      {#if onviewall != null}
        <button class="view-all-btn" onclick={onviewall} type="button">
          <VwText color="accent-blue" size="sm" weight="semibold">{tr("View All")}</VwText>
        </button>
      {/if}
    {/snippet}
    <HyScrollBody>
      {#each blocks as block, index (block.height)}
        <div
          class="block-row row-animate"
          onclick={(): void => {
            onblockclick?.(block.hash);
          }}
          onkeydown={(event: KeyboardEvent): void => {
            if (event.key === "Enter") {
              onblockclick?.(block.hash);
            }
          }}
          role="button"
          tabindex="0"
        >
          <div class="row-top">
            <span class="hash-cell">
              <VwText color="accent-blue" mono={true} size="sm" weight="medium">{block.hash}</VwText>
            </span>
          </div>
          <div class="hy-compact-row-bottom">
            <div class="hy-compact-row-main hy-compact-row-meta">
              <VwText color="muted" size="xs" weight="medium">{block.time}</VwText>
              <span aria-hidden="true" class="hy-compact-row-meta-dot"></span>
              <VwText color="muted" mono={true} size="xs" weight="medium">{HyBlFormatHeightLabel(block.height)}</VwText>
            </div>
            <div class="hy-compact-row-side">
              <VwText mono={true} size="xs" weight="bold">{HyBlFormatTransactionCount(block.txCount)}</VwText>
            </div>
          </div>
        </div>
        {#if index < blocks.length - 1}
          <VwDivider opacity={0.08} />
        {/if}
      {/each}
    </HyScrollBody>
  </HyListPanel>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .block-panel {
    height: 100%;
    min-width: 0;
  }

  .view-all-btn {
    background: none;
    border: none;
    cursor: pointer;
    padding: 0;
  }

  .block-row {
    cursor: pointer;
    padding: var(--hy-card-list-row-padding);
    transition: background var(--duration-fast) var(--ease-default);
  }

  .block-row:hover {
    background: var(--hy-list-row-hover);
  }

  .block-row:focus-visible {
    background: var(--hy-list-row-hover);
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }

  .row-top {
    align-items: center;
    display: flex;
    gap: var(--hy-inline-gap);
    justify-content: space-between;
  }

  .hash-cell {
    align-items: center;
    display: flex;
    flex: 1;
    gap: var(--hy-inline-gap);
    min-width: 0;
  }

  .hash-cell :global(.text) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .hy-compact-row-bottom {
    margin-top: 4px;
  }

  /* --hy-breakpoint-tablet */
  @include view.container-down(tablet) {
    .block-panel.mobile-expand,
    .block-panel.mobile-expand :global(.panel) {
      height: auto;
    }

    .block-panel:not(.mobile-expand),
    .block-panel:not(.mobile-expand) :global(.panel) {
      height: 60vh;
    }
  }
</style>
