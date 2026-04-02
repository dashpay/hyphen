<script lang="ts">
  import type { Snippet } from "svelte";

  /** Badge for compact status and type labels. */
  interface Props {
    readonly size?: "sm" | "lg";
    readonly variant?: "outline" | "filled" | "accent";
    readonly children: Snippet;
  }

  const { size = "sm", variant = "outline", children }: Props = $props();

  const classes = $derived(`badge size-${size} variant-${variant}`);
</script>

<span class={classes}>
  {@render children()}
</span>

<style>
  .badge {
    align-items: center;
    border-radius: var(--radius-card);
    display: inline-flex;
    font-family: var(--font-primary);
    justify-content: center;
    white-space: nowrap;
  }

  .size-sm {
    font-size: var(--text-sm);
    font-weight: 500;
    padding: 10px 35px;
  }

  .size-lg {
    font-size: var(--text-md-plus);
    font-weight: 600;
    padding: 15px 35px;
  }

  .variant-outline {
    background: transparent;
    border: 0.49px solid var(--color-fg);
    color: var(--color-fg);
  }

  .variant-filled {
    background: var(--color-fg);
    border: 0.49px solid transparent;
    color: var(--color-bg);
  }

  .variant-accent {
    background: var(--accent-blue);
    border: 0.49px solid transparent;
    color: #ffffff;
  }
</style>
