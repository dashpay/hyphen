<script lang="ts">
  import type { Snippet } from "svelte";

  /** Stat card with headline value and optional action. */
  interface Props {
    readonly label: string;
    readonly value: string;
    readonly meta?: string;
    readonly action?: Snippet;
  }

  const { label, value, meta, action }: Props = $props();
</script>

<article class="stat-card">
  <div class="header">
    <span class="label">{label}</span>
    {#if action}
      <div class="action">
        {@render action()}
      </div>
    {/if}
  </div>
  <strong class="value">{value}</strong>
  {#if meta}
    <span class="meta">{meta}</span>
  {/if}
</article>

<style>
  .stat-card {
    background: var(--surface-base);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-card);
    color: var(--color-fg);
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    padding: 24px;
  }

  .header {
    align-items: center;
    display: flex;
    gap: var(--space-3);
    justify-content: space-between;
  }

  .label {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-sm-plus);
    font-weight: 600;
  }

  .action {
    align-items: center;
    display: inline-flex;
    flex-shrink: 0;
  }

  .value {
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
</style>
