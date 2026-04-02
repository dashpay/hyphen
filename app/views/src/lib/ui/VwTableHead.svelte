<script lang="ts">
  import type { Snippet } from "svelte";

  /** Table head wrapper for labels and header controls. */
  interface Props {
    readonly children: Snippet;
  }

  const { children }: Props = $props();
</script>

<thead class="table-head">
  {@render children()}
</thead>

<style>
  .table-head {
    position: sticky;
    top: 0;
    z-index: 1;
  }
</style>
