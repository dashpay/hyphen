<script lang="ts">
  /** State block for panel-local loading, empty, and error content. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
  }

  const { children }: Props = $props();
</script>

<div class="panel-state">
  {@render children()}
</div>

<style>
  .panel-state {
    align-items: center;
    display: flex;
    flex: 1;
    justify-content: center;
    padding: var(--hy-panel-state-padding);
  }
</style>
