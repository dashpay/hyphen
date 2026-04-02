<script lang="ts">
  /** Shell control that toggles the sidebar rail between full and compact modes. */
  import type { Component } from "svelte";

  interface Props {
    readonly icon: Component;
    readonly label: string;
    readonly onclick?: (() => void) | undefined;
  }

  const { icon: Icon, label, onclick }: Props = $props();
</script>

<button aria-label={label} class="toggle" {onclick} title={label} type="button">
  <Icon aria-hidden="true" height="18" width="18"></Icon>
</button>

<style>
  .toggle {
    align-items: center;
    background: var(--overlay-subtle);
    border: none;
    border-radius: var(--radius-identity);
    color: var(--color-fg);
    cursor: pointer;
    display: inline-flex;
    height: var(--hy-shell-control-height);
    justify-content: center;
    min-width: var(--hy-shell-control-min-width);
    padding: var(--hy-shell-control-inline-padding);
    transition:
      background var(--duration-fast) var(--ease-default),
      color var(--duration-fast) var(--ease-default);
    width: min(100%, var(--hy-shell-control-max-width));
  }

  .toggle:hover {
    background: var(--overlay-hover);
  }

  .toggle:focus-visible {
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }
</style>
