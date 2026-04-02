<script lang="ts">
  import type { Snippet } from "svelte";

  /** Key-value list for compact metadata groups. */
  interface KeyValueItem {
    readonly key: string;
    readonly label: string;
    readonly value: string | Snippet;
  }

  interface Props {
    readonly items: readonly KeyValueItem[];
  }

  const { items }: Props = $props();

  function isSnippet(value: KeyValueItem["value"]): value is Snippet {
    return typeof value === "function";
  }
</script>

<dl class="key-value-list">
  {#each items as item (item.key)}
    <div class="row">
      <dt>{item.label}</dt>
      <dd>
        {#if isSnippet(item.value)}
          {@render item.value()}
        {:else}
          {item.value}
        {/if}
      </dd>
    </div>
  {/each}
</dl>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .key-value-list {
    display: grid;
    gap: var(--space-4);
    margin: 0;
  }

  .row {
    align-items: start;
    display: grid;
    gap: var(--space-3);
    grid-template-columns: minmax(120px, 180px) minmax(0, 1fr);
  }

  dt,
  dd {
    margin: 0;
  }

  dt {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-sm-plus);
    font-weight: 600;
  }

  dd {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-base-plus);
    font-weight: 500;
    min-width: 0;
  }

  @include view.media-down(tablet) {
    .row {
      grid-template-columns: 1fr;
    }
  }
</style>
