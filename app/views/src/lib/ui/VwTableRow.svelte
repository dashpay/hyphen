<script lang="ts">
  import type { Snippet } from "svelte";

  /** Table row wrapper for cell compositions. */
  interface Props {
    readonly children: Snippet;
  }

  const { children }: Props = $props();
</script>

<tr class="table-row">
  {@render children()}
</tr>

<style>
  .table-row {
    background: transparent;
    transition: background var(--duration-fast) var(--ease-default);
  }

  .table-row:hover {
    background: var(--surface-raised);
  }
</style>
