<script lang="ts">
  /** Metric counter card with embedded history graphics and action. */
  import type { Component } from "svelte";

  import { VwPanel, VwText, VwVStack } from "@repo/hyphen-views";

  import { HyAnHoverGloss } from "../anim";
  import { chartToSmoothPath, type ChartPoint } from "../data/chart";
  import HyAmountText from "./HyAmountText.svelte";

  const POINT_SPACING = 8;

  interface Props {
    readonly label: string;
    readonly value: string;
    readonly icon: Component;
    readonly data: readonly ChartPoint[];
    readonly onExpand?: () => void;
  }

  const { label, value, icon: Icon, data, onExpand }: Props = $props();

  let chartWidth = $state(300);
  let chartHeight = $state(65);

  const visibleData = $derived.by((): readonly ChartPoint[] => {
    const maxPoints = Math.max(Math.floor(chartWidth / POINT_SPACING), 2);
    return data.length > maxPoints ? data.slice(-maxPoints) : data;
  });

  const path = $derived(chartToSmoothPath(visibleData, chartWidth, chartHeight));
</script>

<button
  aria-label={onExpand != null ? "Expand " + label + " chart" : undefined}
  class:clickable={onExpand != null}
  class="counter-wrap"
  onclick={(): void => {
    onExpand?.();
  }}
  type="button"
  use:HyAnHoverGloss
>
  <VwPanel border={true} padding={16} radius="sm" variant="glass">
    <div class="bg-icon">
      <Icon color="var(--accent-blue)" height="140" width="140" />
    </div>
    <div bind:clientHeight={chartHeight} bind:clientWidth={chartWidth} class="chart-bg">
      <svg aria-hidden="true" height={chartHeight} viewBox="0 0 {chartWidth} {chartHeight}" width={chartWidth}>
        <path d={path} fill="none" stroke="var(--accent-blue)" stroke-width="2" />
      </svg>
    </div>
    <VwVStack gap={8}>
      <VwText color="muted" size="sm" weight="medium">{label}</VwText>
      <VwText size="xl" weight="extrabold">
        <HyAmountText {value} />
      </VwText>
    </VwVStack>
    <div aria-hidden="true" class="detail-button">
      <svg
        aria-hidden="true"
        fill="none"
        height="16"
        stroke="white"
        stroke-linecap="round"
        stroke-linejoin="round"
        stroke-width="2"
        viewBox="0 0 24 24"
        width="16"
      >
        <polyline points="9 18 15 12 9 6"></polyline>
      </svg>
    </div>
  </VwPanel>
</button>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .counter-wrap {
    background: none;
    border: none;
    display: block;
    height: 100%;
    min-width: 0;
    padding: 0;
    text-align: left;
    width: 100%;
  }

  .counter-wrap.clickable {
    cursor: pointer;
  }

  .counter-wrap :global(.text) {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .counter-wrap :global(.panel) {
    height: 100%;
    overflow: hidden;
    position: relative;
  }

  .bg-icon {
    -webkit-mask-image: linear-gradient(to bottom, black 70%, transparent);
    mask-image: linear-gradient(to bottom, black 70%, transparent);
    opacity: 0.2;
    pointer-events: none;
    position: absolute;
    right: var(--hy-graph-counter-icon-offset-x);
    top: var(--hy-graph-counter-icon-offset-y);
  }

  .bg-icon > :global(svg) {
    paint-order: stroke;
    stroke: var(--color-card-surface);
    stroke-width: var(--hy-graph-counter-icon-stroke-width);
  }

  .chart-bg {
    bottom: 0;
    height: 60%;
    left: 0;
    opacity: 0.2;
    pointer-events: none;
    position: absolute;
    width: 100%;
  }

  .detail-button {
    align-items: center;
    background: var(--accent-blue);
    border-radius: 50%;
    bottom: var(--hy-graph-counter-action-inset);
    display: flex;
    height: var(--hy-graph-counter-action-size);
    justify-content: center;
    pointer-events: none;
    position: absolute;
    right: var(--hy-graph-counter-action-inset);
    width: var(--hy-graph-counter-action-size);
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .counter-wrap :global(.panel) {
      min-height: 200px;
    }
  }
</style>
