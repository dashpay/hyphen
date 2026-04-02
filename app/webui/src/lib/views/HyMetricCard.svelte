<script lang="ts">
  /** Metric card for compact headline values and supporting copy. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
    readonly tone?: "accent" | "default";
  }

  const { children, tone = "default" }: Props = $props();
</script>

<div class:accent-card={tone === "accent"} class="metric-card">
  {@render children()}
</div>

<style>
  .metric-card {
    background: var(--overlay-subtle);
    border: 1px solid var(--overlay-subtle);
    border-radius: var(--radius-input);
    display: flex;
    flex-direction: column;
    gap: var(--hy-detail-card-gap);
    min-width: 0;
    padding: var(--hy-detail-chip-padding);
  }

  .metric-card.accent-card {
    background: var(--surface-accent-subtle);
    border-color: var(--border-accent-subtle);
    padding: var(--hy-detail-balance-card-padding);
  }
</style>
