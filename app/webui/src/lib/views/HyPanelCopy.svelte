<script lang="ts">
  /** Copy block for short panel descriptions and helper text. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
  }

  const { children }: Props = $props();
</script>

<section class="panel-copy-wrap">
  <p class="panel-copy">
    {@render children()}
  </p>
</section>

<style>
  .panel-copy-wrap {
    align-items: center;
    display: flex;
    min-height: 0;
  }

  .panel-copy {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    margin: 0;
    padding: var(--space-5);
  }
</style>
