<script lang="ts">
  /** Horizontal bar chart for ranked comparisons. */
  interface BarDatum {
    readonly label: string;
    readonly value: number;
    readonly displayValue?: string;
    readonly color?: string;
  }

  interface Props {
    readonly items: readonly BarDatum[];
    readonly maxValue?: number;
  }

  const { items, maxValue }: Props = $props();

  const ceiling = $derived.by((): number => {
    const explicitMax = maxValue ?? Math.max(...items.map((item) => item.value), 1);
    return explicitMax > 0 ? explicitMax : 1;
  });

  function formatFillWidth(value: number): string {
    const ratio = Math.max(0, Math.min(1, value / ceiling));
    return String(ratio * 100) + "%";
  }
</script>

<div class="bar-chart">
  {#each items as item, index (`${item.label}:${String(index)}`)}
    <div class="row">
      <div class="meta">
        <span class="label">{item.label}</span>
        <span class="value">{item.displayValue ?? String(item.value)}</span>
      </div>
      <div class="track">
        <div
          class="fill"
          style:width={formatFillWidth(item.value)}
          style:background={item.color ?? "var(--accent-blue)"}
        ></div>
      </div>
    </div>
  {/each}
</div>

<style>
  .bar-chart {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .row {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
  }

  .meta {
    align-items: baseline;
    display: flex;
    gap: var(--space-3);
    justify-content: space-between;
  }

  .label {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-base);
    font-weight: 500;
  }

  .value {
    color: var(--text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .track {
    background: var(--surface-raised);
    border-radius: var(--radius-full);
    height: 10px;
    overflow: hidden;
  }

  .fill {
    border-radius: inherit;
    height: 100%;
    min-width: 6px;
  }
</style>
