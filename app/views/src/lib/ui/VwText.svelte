<script lang="ts">
  import type { Snippet } from "svelte";

  import { FONT_WEIGHT, TEXT_SCALE } from "../index";

  /** Text primitive for shared type scale and color tokens. */
  export type TextSize = keyof typeof TEXT_SCALE;
  export type TextWeight = keyof typeof FONT_WEIGHT;
  export type TextColor = "default" | "accent-blue" | "accent-mint" | "muted" | "tertiary" | "faint" | "on-accent";
  export type TextAlign = "left" | "center" | "right";
  export type TextTag = "span" | "p" | "h1" | "h2" | "h3" | "h4" | "h5" | "h6" | "label" | "small";
  export type TextTracking = "normal" | "tight";

  interface Props {
    readonly size?: TextSize;
    readonly weight?: TextWeight;
    readonly align?: TextAlign;
    readonly mono?: boolean;
    readonly muted?: boolean;
    readonly color?: TextColor;
    readonly as?: TextTag;
    readonly htmlFor?: string;
    readonly tracking?: TextTracking;
    readonly lineHeight?: number | string;
    readonly children: Snippet;
  }

  const {
    size = "base",
    weight = "medium",
    align,
    mono = false,
    muted = false,
    color = "default",
    as = "span",
    htmlFor,
    tracking = "normal",
    lineHeight,
    children,
  }: Props = $props();

  const isBlock = $derived(as === "p" || (as !== "span" && as !== "label" && as !== "small" && as.startsWith("h")));

  const computedStyle = $derived.by((): string => {
    const parts: string[] = [];
    parts.push(`font-size: ${String(TEXT_SCALE[size])}px`);
    parts.push(`font-weight: ${String(FONT_WEIGHT[weight])}`);
    if (align != null) {
      parts.push(`text-align: ${align}`);
    }
    if (tracking === "tight") {
      parts.push("letter-spacing: -0.03em");
    }
    if (lineHeight != null) {
      if (typeof lineHeight === "number" && lineHeight >= 5) {
        parts.push(`line-height: ${String(lineHeight)}px`);
      } else {
        parts.push(`line-height: ${String(lineHeight)}`);
      }
    }
    if (isBlock) {
      parts.push("display: block");
    }
    return parts.join("; ");
  });

  const classes = $derived.by((): string => {
    const values: string[] = ["text"];
    if (mono) {
      values.push("mono");
    }
    if (muted || color === "muted") {
      values.push("muted");
    }
    if (color === "accent-blue") {
      values.push("accent-blue");
    }
    if (color === "accent-mint") {
      values.push("accent-mint");
    }
    if (color === "tertiary") {
      values.push("tertiary");
    }
    if (color === "faint") {
      values.push("faint");
    }
    if (color === "on-accent") {
      values.push("on-accent");
    }
    return values.join(" ");
  });
</script>

{#if as === "h1"}
  <h1 class={classes} style={computedStyle}>{@render children()}</h1>
{:else if as === "h2"}
  <h2 class={classes} style={computedStyle}>{@render children()}</h2>
{:else if as === "h3"}
  <h3 class={classes} style={computedStyle}>{@render children()}</h3>
{:else if as === "h4"}
  <h4 class={classes} style={computedStyle}>{@render children()}</h4>
{:else if as === "h5"}
  <h5 class={classes} style={computedStyle}>{@render children()}</h5>
{:else if as === "h6"}
  <h6 class={classes} style={computedStyle}>{@render children()}</h6>
{:else if as === "p"}
  <p class={classes} style={computedStyle}>{@render children()}</p>
{:else if as === "label"}
  <label class={classes} for={htmlFor} style={computedStyle}>{@render children()}</label>
{:else if as === "small"}
  <small class={classes} style={computedStyle}>{@render children()}</small>
{:else}
  <span class={classes} style={computedStyle}>{@render children()}</span>
{/if}

<style>
  .text {
    color: var(--color-fg);
    line-height: inherit;
    font-family: var(--font-primary);
  }

  .mono {
    font-family: var(--font-mono);
  }

  .muted {
    color: var(--text-muted);
  }

  .tertiary {
    color: var(--text-tertiary);
  }

  .faint {
    color: var(--text-faint);
  }

  .on-accent {
    color: var(--color-on-accent, #ffffff);
  }

  .accent-blue {
    color: var(--accent-blue);
  }

  .accent-mint {
    color: var(--accent-mint);
  }
</style>
