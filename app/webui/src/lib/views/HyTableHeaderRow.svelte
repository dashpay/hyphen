<script lang="ts">
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
    readonly columns?: string;
    readonly gap?: string;
    readonly mode?: "flex" | "grid";
  }

  const { children, columns, gap, mode = "grid" }: Props = $props();
</script>

<div
  class:flex-layout={mode === "flex"}
  class="table-header"
  style:gap={`var(--hy-table-header-gap, ${gap ?? "var(--hy-inline-gap)"})`}
  style:grid-template-columns={mode === "grid" ? `var(--hy-table-header-columns, ${columns ?? "subgrid"})` : undefined}
>
  {@render children()}
</div>

<style>
  .table-header {
    backdrop-filter: blur(var(--blur-radius));
    background: var(--color-card-surface);
    border-bottom: 1px solid var(--glass-border);
    display: grid;
    grid-column: 1 / -1;
    padding: var(--hy-list-header-padding);
    position: sticky;
    top: 0;
    z-index: 1;
  }

  .table-header.flex-layout {
    display: flex;
    flex-shrink: 0;
  }

  .table-header :global(.col) {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-xs);
    font-weight: 700;
    justify-content: flex-start;
    letter-spacing: 0.05em;
    min-width: 0;
    overflow: hidden;
    text-overflow: ellipsis;
    text-transform: uppercase;
    white-space: nowrap;
  }
</style>
