<script lang="ts">
  /** Button primitive for actions, links, and icon triggers. */
  import type { Snippet } from "svelte";

  type ButtonVariant = "primary" | "beveled" | "cta" | "ghost" | "ghost-tinted";

  interface Props {
    readonly variant?: ButtonVariant;
    readonly fullWidth?: boolean;
    readonly disabled?: boolean;
    readonly action?: () => void;
    readonly icon?: Snippet;
    readonly children: Snippet;
  }

  const { variant = "primary", fullWidth = false, disabled = false, action, icon, children }: Props = $props();

  const classes = $derived.by((): string => {
    const values: string[] = ["button", `variant-${variant}`];
    if (fullWidth) {
      values.push("full-width");
    }
    return values.join(" ");
  });
</script>

<button class={classes} {disabled} onclick={action} type="button">
  {#if icon}
    <span class="icon">{@render icon()}</span>
  {/if}
  <span class="label">{@render children()}</span>
</button>

<style>
  .button {
    align-items: center;
    border: none;
    cursor: pointer;
    display: inline-flex;
    font-family: var(--font-primary);
    font-weight: 600;
    gap: var(--space-2);
    justify-content: center;
    transition:
      background var(--duration-fast) var(--ease-default),
      opacity var(--duration-fast) var(--ease-default);
    white-space: nowrap;
  }

  .button:disabled {
    cursor: not-allowed;
    opacity: 0.4;
  }

  .full-width {
    width: 100%;
  }

  .variant-primary,
  .variant-beveled,
  .variant-ghost,
  .variant-ghost-tinted {
    font-size: var(--text-md-plus);
    height: 65px;
    padding: 10px 35px;
  }

  .variant-primary,
  .variant-beveled,
  .variant-cta,
  .variant-ghost,
  .variant-ghost-tinted {
    -webkit-backdrop-filter: blur(var(--blur-radius));
    backdrop-filter: blur(var(--blur-radius));
  }

  .variant-primary {
    background: var(--accent-blue);
    border-radius: var(--radius-pill);
    color: #ffffff;
  }

  .variant-beveled {
    background: var(--accent-blue);
    border-radius: var(--radius-button);
    color: #ffffff;
  }

  .variant-cta {
    background: var(--accent-mint);
    border-radius: var(--radius-input);
    color: var(--color-bg);
    font-size: var(--text-md-plus);
    height: 50px;
    padding: 10px 25px;
  }

  .variant-ghost {
    background: var(--surface-raised);
    border-radius: var(--radius-pill);
    color: var(--color-fg);
  }

  .variant-ghost-tinted {
    background: var(--accent-blue-15);
    border-radius: var(--radius-button);
    color: #ffffff;
  }

  .button:hover:not(:disabled) {
    opacity: 0.85;
  }

  .icon,
  .label {
    align-items: center;
    display: inline-flex;
  }
</style>
