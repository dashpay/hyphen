<script lang="ts">
  /** Horizontal stack with shared gap and alignment props. */
  import type { Snippet } from "svelte";

  type StackAlign = "stretch" | "start" | "center" | "end" | "baseline";
  type StackJustify = "start" | "center" | "end" | "between";

  interface Props {
    readonly gap?: number;
    readonly padding?: number;
    readonly align?: StackAlign;
    readonly justify?: StackJustify;
    readonly wrap?: boolean;
    readonly children: Snippet;
  }

  const { gap = 0, padding = 0, align = "center", justify = "start", wrap = false, children }: Props = $props();

  const justifyMap: Record<StackJustify, string> = {
    start: "flex-start",
    center: "center",
    end: "flex-end",
    between: "space-between",
  };

  const alignMap: Record<StackAlign, string> = {
    stretch: "stretch",
    start: "flex-start",
    center: "center",
    end: "flex-end",
    baseline: "baseline",
  };

  const computedStyle = $derived(
    `gap: ${String(gap)}px; padding: ${String(padding)}px; align-items: ${alignMap[align]}; justify-content: ${justifyMap[justify]}${wrap ? "; flex-wrap: wrap" : ""}`
  );
</script>

<div class="hstack" style={computedStyle}>
  {@render children()}
</div>

<style>
  .hstack {
    display: flex;
    flex-direction: row;
  }
</style>
