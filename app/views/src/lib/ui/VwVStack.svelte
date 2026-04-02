<script lang="ts">
  /** Vertical stack with shared gap and alignment props. */
  import type { Snippet } from "svelte";

  type StackAlign = "stretch" | "start" | "center" | "end";
  type StackJustify = "start" | "center" | "end" | "between";

  interface Props {
    readonly gap?: number;
    readonly padding?: number;
    readonly align?: StackAlign;
    readonly justify?: StackJustify;
    readonly children: Snippet;
  }

  const { gap = 0, padding = 0, align = "stretch", justify = "start", children }: Props = $props();

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
  };

  const computedStyle = $derived(
    `gap: ${String(gap)}px; padding: ${String(padding)}px; align-items: ${alignMap[align]}; justify-content: ${justifyMap[justify]}`
  );
</script>

<div class="vstack" style={computedStyle}>
  {@render children()}
</div>

<style>
  .vstack {
    display: flex;
    flex-direction: column;
  }
</style>
