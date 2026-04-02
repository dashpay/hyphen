<script lang="ts">
  /** Disclosure surface for expandable blocks of content. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly title: string;
    readonly open?: boolean;
    readonly summary?: string;
    readonly children: Snippet;
  }

  const { title, open = false, summary, children }: Props = $props();
</script>

<details class="disclosure" {open}>
  <summary class="summary">
    <span class="copy">
      <span class="title">{title}</span>
      {#if summary}
        <span class="meta">{summary}</span>
      {/if}
    </span>
    <span aria-hidden="true" class="indicator">›</span>
  </summary>
  <div class="content">
    {@render children()}
  </div>
</details>

<style>
  .disclosure {
    background: var(--surface-base);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-card);
    overflow: hidden;
  }

  .summary {
    align-items: center;
    cursor: pointer;
    display: flex;
    gap: var(--space-3);
    justify-content: space-between;
    list-style: none;
    padding: 20px 24px;
  }

  .summary::-webkit-details-marker {
    display: none;
  }

  .copy {
    align-items: baseline;
    display: flex;
    gap: var(--space-3);
    min-width: 0;
  }

  .title {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-base-plus);
    font-weight: 600;
  }

  .meta {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-sm-plus);
  }

  .indicator {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-lg);
    line-height: 1;
    transition: transform var(--duration-fast) var(--ease-default);
  }

  .disclosure[open] .indicator {
    transform: rotate(90deg);
  }

  .content {
    border-top: 1px solid var(--border-subtle);
    padding: 20px 24px 24px;
  }
</style>
