<script lang="ts">
  /** Centered state container for loading, empty, and error content. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
  }

  const { children }: Props = $props();
</script>

<div class="center-state">
  {@render children()}
</div>

<style>
  .center-state {
    align-items: center;
    display: flex;
    height: 100%;
    justify-content: center;
  }
</style>
