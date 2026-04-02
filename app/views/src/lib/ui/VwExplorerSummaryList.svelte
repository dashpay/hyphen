<script lang="ts">
  /** Summary list used by explorer-style overview panels. */
  interface SummaryItem {
    readonly id: string;
    readonly title: string;
    readonly subtitle: string;
    readonly meta?: string;
    readonly trailing?: string;
  }

  interface Props {
    readonly items: readonly SummaryItem[];
  }

  const { items }: Props = $props();
</script>

<div class="summary-list">
  {#each items as item (item.id)}
    <div class="item">
      <div class="copy">
        <span class="title">{item.title}</span>
        <span class="subtitle">{item.subtitle}</span>
        {#if item.meta}
          <span class="meta">{item.meta}</span>
        {/if}
      </div>
      {#if item.trailing}
        <span class="trailing">{item.trailing}</span>
      {/if}
    </div>
  {/each}
</div>

<style>
  .summary-list {
    display: flex;
    flex-direction: column;
  }

  .item {
    align-items: center;
    display: flex;
    gap: var(--space-4);
    justify-content: space-between;
    min-height: 58px;
    padding: 0 16px;
    transition: background var(--duration-fast) var(--ease-default);
  }

  .item:hover {
    background: var(--surface-raised);
  }

  .copy {
    display: flex;
    flex-direction: column;
    gap: 2px;
    min-width: 0;
    overflow: hidden;
  }

  .title {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    font-weight: 600;
  }

  .subtitle {
    color: var(--text-muted);
    font-family: var(--font-mono);
    font-size: var(--text-xs);
    font-weight: 300;
    max-width: 260px;
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .meta {
    color: var(--text-tertiary);
    font-family: var(--font-primary);
    font-size: var(--text-xs);
    font-weight: 400;
  }

  .trailing {
    color: var(--color-fg);
    flex-shrink: 0;
    font-family: var(--font-primary);
    font-size: var(--text-md);
    font-weight: 600;
  }
</style>
