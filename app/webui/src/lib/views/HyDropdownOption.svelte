<script lang="ts">
  /** Dropdown option row for denomination and shell selectors. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
    readonly onclick?: () => void;
  }

  const { children, onclick }: Props = $props();
</script>

<button class="dropdown-option" {onclick} type="button">
  {@render children()}
</button>

<style>
  .dropdown-option {
    background: transparent;
    border: none;
    border-radius: var(--radius-input);
    color: var(--color-fg);
    cursor: pointer;
    display: block;
    font-family: var(--font-primary);
    padding: var(--hy-shell-dropdown-padding);
    text-align: left;
    transition: background var(--duration-fast) var(--ease-default);
    width: 100%;
  }

  .dropdown-option:hover {
    background: var(--accent-blue-15);
  }

  .dropdown-option:focus-visible {
    background: var(--accent-blue-15);
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }
</style>
