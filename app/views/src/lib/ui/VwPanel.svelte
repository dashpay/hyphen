<script lang="ts">
  import type { Snippet } from "svelte";

  /** Panel surface with shared variants and padding controls. */
  type PanelRadius = "sm" | "card" | "marketing" | "panel";
  type PanelVariant = "card" | "glass" | "tinted" | "transparent";

  interface Props {
    readonly padding?: number;
    readonly radius?: PanelRadius;
    readonly variant?: PanelVariant;
    readonly border?: boolean;
    readonly children: Snippet;
  }

  const { padding = 20, radius = "card", variant = "card", border = false, children }: Props = $props();

  const radiusMap: Record<PanelRadius, string> = {
    sm: "var(--radius-input)",
    card: "var(--radius-card)",
    marketing: "var(--radius-card)",
    panel: "var(--radius-header)",
  };

  const computedStyle = $derived(`padding: ${String(padding)}px; border-radius: ${radiusMap[radius]}`);

  const classes = $derived.by((): string => {
    const values: string[] = ["panel", `variant-${variant}`];
    if (border) {
      values.push("bordered");
    }
    return values.join(" ");
  });
</script>

<div class={classes} style={computedStyle}>
  {@render children()}
</div>

<style>
  .panel {
    color: var(--color-fg);
    overflow: hidden;
    position: relative;
  }

  .variant-card {
    background: var(--surface-base);
    box-shadow: var(--shadow-card);
  }

  .variant-glass {
    background: var(--surface-raised);
    -webkit-backdrop-filter: blur(var(--blur-radius));
    backdrop-filter: blur(var(--blur-radius));
  }

  .variant-tinted {
    background: var(--accent-blue-15);
  }

  .variant-transparent {
    background: transparent;
  }

  .bordered {
    border: 1px solid var(--glass-border);
  }
</style>
