<script lang="ts">
  import type { Snippet } from "svelte";

  /** Table cell wrapper with alignment and width control. */
  interface Props {
    readonly as?: "th" | "td";
    readonly align?: "left" | "right" | "center";
    readonly width?: string;
    readonly children: Snippet;
  }

  const { as = "td", align = "left", width, children }: Props = $props();
  const classes = $derived(`table-cell align-${align}`);
</script>

{#if as === "th"}
  <th class={classes} style:width>
    {@render children()}
  </th>
{:else}
  <td class={classes} style:width>
    {@render children()}
  </td>
{/if}

<style>
  .table-cell {
    border-bottom: 1px solid var(--border-subtle);
    font-family: var(--font-primary);
    padding: 16px 20px;
  }

  th.table-cell {
    color: var(--text-muted);
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0.04em;
    text-transform: uppercase;
    white-space: nowrap;
  }

  td.table-cell {
    color: var(--color-fg);
    font-size: var(--text-base);
    font-weight: 500;
  }

  .align-left {
    text-align: left;
  }

  .align-center {
    text-align: center;
  }

  .align-right {
    text-align: right;
  }
</style>
