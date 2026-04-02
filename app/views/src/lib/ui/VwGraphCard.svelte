<script lang="ts">
  import VwSparkline from "./VwSparkline.svelte";

  /** Graph card with value, sparkline, and optional meta. */
  interface Props {
    readonly label: string;
    readonly value: string;
    readonly points: readonly number[];
    readonly stroke?: string;
    readonly meta?: string;
  }

  const { label, value, points, stroke = "var(--accent-blue)", meta }: Props = $props();
</script>

<article class="graph-card">
  <div class="copy">
    <span class="label">{label}</span>
    <strong class="value">{value}</strong>
    {#if meta}
      <span class="meta">{meta}</span>
    {/if}
  </div>
  <div class="chart">
    <VwSparkline fill fillOpacity={0.14} {points} {stroke} />
  </div>
</article>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .graph-card {
    align-items: end;
    background: var(--surface-base);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-card);
    display: grid;
    gap: var(--space-5);
    grid-template-columns: minmax(0, 1fr) minmax(120px, 180px);
    padding: 24px;
  }

  .copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .label {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-sm-plus);
    font-weight: 600;
  }

  .value {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-2xl);
    font-weight: 700;
    line-height: 1.1;
  }

  .meta {
    color: var(--text-tertiary);
    font-family: var(--font-primary);
    font-size: var(--text-base);
  }

  .chart {
    align-self: stretch;
    display: flex;
    min-height: 72px;
  }

  @include view.media-down(tablet) {
    .graph-card {
      grid-template-columns: 1fr;
    }
  }
</style>
