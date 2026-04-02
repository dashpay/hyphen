<script lang="ts">
  /** Concentration panel for rich-list share and donut composition. */
  import { fmt, tr } from "@repo/hyphen-i18n";
  import { VwDonutChart } from "@repo/hyphen-views";

  import { HyListPanel } from "$lib/views";

  interface PercentileRow {
    readonly percentile: string;
    readonly pct: number;
    readonly colorVar: string;
  }

  interface DonutSegment {
    readonly color: string;
    readonly label: string;
    readonly value: number;
  }

  interface Props {
    readonly donutSegments: readonly DonutSegment[];
    readonly hoveredSegment: number | null;
    readonly onhover: (index: number | null) => void;
    readonly percentileData: readonly PercentileRow[];
    readonly topConcentration: string;
  }

  const { donutSegments, hoveredSegment, onhover, percentileData, topConcentration }: Props = $props();

  let donutWidth = $state(0);

  const donutSize = $derived.by((): number => {
    if (donutWidth <= 0) {
      return 180;
    }

    return Math.max(132, Math.min(180, donutWidth - 32));
  });

  const donutStrokeWidth = $derived(donutSize <= 160 ? 24 : 28);
</script>

<div class="concentration-panel">
  <HyListPanel title={tr("Concentration")}>
    <div class="rcol-body">
      <div bind:clientWidth={donutWidth} class="donut-center">
        <VwDonutChart
          fill={true}
          centerLabel={tr("Top 1")}
          centerValue={topConcentration}
          gapDeg={2}
          hoveredIndex={hoveredSegment}
          {onhover}
          segments={donutSegments}
          size={donutSize}
          strokeWidth={donutStrokeWidth}
        />
      </div>

      <div class="concentration-table">
        <div class="concentration-header">
          <span class="concentration-heading">{tr("Percentile")}</span>
          <span class="concentration-heading concentration-heading-value">{tr("% Supply")}</span>
        </div>
        {#each percentileData as row (row.percentile)}
          <div class="concentration-row">
            <span class="swatch" style:background={row.colorVar}></span>
            <span class="percentile-label">{row.percentile}</span>
            <span class="percentile-value">{fmt("{}%", row.pct.toFixed(1))}</span>
          </div>
        {/each}
      </div>
    </div>
  </HyListPanel>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .concentration-panel {
    height: 100%;
    min-height: 0;
    min-width: 0;
  }

  .rcol-body {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    height: 100%;
    min-height: 0;
  }

  .donut-center {
    align-items: center;
    display: flex;
    flex: 1;
    justify-content: center;
    min-height: 0;
    padding: var(--hy-donut-center-padding);
  }

  .donut-center :global(.donut) {
    aspect-ratio: 1;
    height: auto;
    max-width: min(100%, var(--hy-donut-size));
    width: min(100%, var(--hy-donut-size));
  }

  .concentration-table {
    display: flex;
    flex-direction: column;
    padding: 0 var(--space-3) var(--space-4);
  }

  .concentration-header {
    align-items: center;
    border-bottom: 1px solid var(--glass-border);
    border-top: 1px solid var(--glass-border);
    display: grid;
    gap: var(--space-3);
    grid-template-columns: minmax(0, 1fr) max-content;
    padding: var(--hy-donut-legend-padding);
  }

  .concentration-heading {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-xs);
    font-weight: 700;
    letter-spacing: 0.05em;
    text-transform: uppercase;
  }

  .concentration-heading-value {
    text-align: right;
  }

  .concentration-row {
    align-items: center;
    border-bottom: 1px solid var(--overlay-subtle);
    display: grid;
    gap: var(--space-3);
    grid-template-columns: var(--hy-donut-swatch-size) minmax(0, 1fr) max-content;
    padding: var(--hy-donut-row-padding);
  }

  .swatch {
    border-radius: 999px;
    display: block;
    height: var(--hy-donut-swatch-size);
    width: var(--hy-donut-swatch-size);
  }

  .percentile-label {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .percentile-value {
    color: var(--color-fg);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-weight: 700;
  }

  /* --hy-breakpoint-laptop */
  @include view.media-down(laptop) {
    .donut-center :global(.donut) {
      height: var(--hy-donut-size);
      max-width: var(--hy-donut-size);
    }
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .concentration-panel :global(.list-panel),
    .concentration-panel :global(.panel) {
      height: auto;
    }

    .rcol-body {
      align-items: stretch;
      display: grid;
      gap: var(--space-3);
      grid-template-columns: minmax(132px, 42%) minmax(0, 1fr);
      height: auto;
      padding: var(--space-3);
    }

    .donut-center {
      align-self: start;
      padding: 0;
    }

    .donut-center :global(.donut) {
      width: min(100%, var(--hy-donut-size-mobile));
    }

    .concentration-table {
      padding: 0;
    }

    .concentration-header {
      display: none;
    }

    .donut-center :global(.center-label) {
      font-size: var(--text-2xs);
    }

    .donut-center :global(.center-value) {
      font-size: var(--text-md);
    }

    .concentration-header {
      padding-top: 0;
    }
  }

  /* --hy-breakpoint-phone */
  @include view.media-down(phone) {
    .rcol-body {
      grid-template-columns: minmax(120px, 44%) minmax(0, 1fr);
    }
  }
</style>
