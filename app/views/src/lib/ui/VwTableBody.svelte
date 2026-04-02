<script lang="ts">
  import type { Snippet } from "svelte";

  /** Table body wrapper for row content. */
  interface Props {
    readonly children: Snippet;
  }

  const { children }: Props = $props();
</script>

<tbody class="table-body">
  {@render children()}
</tbody>

<style>
  .table-body {
    background: transparent;
  }
</style>
