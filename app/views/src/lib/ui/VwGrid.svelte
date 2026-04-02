<script lang="ts">
  /** Grid layout with token-based gap and column control. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly cols?: 1 | 2 | 3 | 4 | "auto";
    readonly gap?: number;
    readonly minWidth?: number;
    readonly children: Snippet;
  }

  const { cols = "auto", gap = 20, minWidth = 280, children }: Props = $props();

  const computedStyle = $derived.by((): string => {
    if (cols === "auto") {
      return `grid-template-columns: repeat(auto-fit, minmax(${String(minWidth)}px, 1fr)); gap: ${String(gap)}px`;
    }

    return `grid-template-columns: repeat(${String(cols)}, 1fr); gap: ${String(gap)}px`;
  });
</script>

<div class="grid" style={computedStyle}>
  {@render children()}
</div>

<style>
  .grid {
    display: grid;
  }
</style>
