<script lang="ts">
  /** Compact dropdown for header-level selection controls. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly label: string;
    readonly children?: Snippet;
  }

  const { label, children }: Props = $props();

  let open = $state(false);
  let rootElement = $state<HTMLDivElement | null>(null);

  function toggle(): void {
    open = !open;
  }

  function handleWindowClick(event: MouseEvent): void {
    const target = event.target;
    if (!(target instanceof Node)) {
      return;
    }

    if (open && rootElement != null && !rootElement.contains(target)) {
      open = false;
    }
  }
</script>

<svelte:window onclick={handleWindowClick} />

<div bind:this={rootElement} class="header-dropdown">
  <button aria-expanded={open} class="dropdown-trigger" onclick={toggle} type="button">
    <span class="dropdown-label">{label}</span>
    <svg aria-hidden="true" height="6" viewBox="0 0 10 6" width="10" xmlns="http://www.w3.org/2000/svg">
      <path d="M1 1L5 5L9 1" fill="none" stroke="currentColor" stroke-linecap="round" stroke-width="1.5" />
    </svg>
  </button>
  {#if open && children}
    <div
      class="dropdown-menu"
      onclick={(): void => {
        open = false;
      }}
      onkeydown={(event: KeyboardEvent): void => {
        if (event.key === "Escape") {
          open = false;
        }
      }}
      role="menu"
      tabindex="-1"
    >
      {@render children()}
    </div>
  {/if}
</div>

<style>
  .header-dropdown {
    max-width: 100%;
    min-width: 0;
    position: relative;
  }

  .dropdown-trigger {
    align-items: center;
    background: var(--accent-blue-15);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-pill);
    color: var(--color-fg);
    cursor: pointer;
    display: inline-flex;
    font-family: var(--font-primary);
    font-size: var(--text-sm-plus);
    font-weight: 500;
    gap: 6px;
    max-width: 100%;
    min-width: 0;
    padding: 8px 16px;
    transition: background var(--duration-fast) var(--ease-default);
  }

  .dropdown-trigger:hover {
    background: rgba(76, 126, 255, 0.25);
  }

  .dropdown-trigger:focus-visible {
    background: rgba(76, 126, 255, 0.25);
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }

  .dropdown-label {
    overflow: hidden;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .dropdown-menu {
    background: var(--surface-base);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-nav);
    box-shadow: var(--shadow-tooltip);
    left: 0;
    max-width: min(240px, calc(100vw - 32px));
    min-width: 160px;
    padding: 8px;
    position: absolute;
    top: calc(100% + 8px);
    z-index: 50;
  }

  .header-dropdown :global(button:focus-visible) {
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }
</style>
