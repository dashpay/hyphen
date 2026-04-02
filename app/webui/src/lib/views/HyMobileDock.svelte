<script lang="ts">
  /** Floating mobile dock for primary route navigation. */
  import type { Component } from "svelte";

  interface DockItem {
    readonly active: boolean;
    readonly href: string;
    readonly icon: Component;
    readonly label: string;
  }

  interface Props {
    readonly items: readonly DockItem[];
  }

  const { items }: Props = $props();
</script>

<nav aria-label="Primary navigation" class="mobile-dock">
  {#each items as item (item.label)}
    {@const Icon = item.icon}
    <a
      aria-current={item.active ? "page" : undefined}
      aria-label={item.label}
      class:active={item.active}
      class="dock-item"
      href={item.href}
      title={item.label}
    >
      <span class="icon-slot">
        <Icon aria-hidden="true" height="20" width="20"></Icon>
      </span>
    </a>
  {/each}
</nav>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .mobile-dock {
    -webkit-backdrop-filter: blur(var(--blur-radius));
    align-items: center;
    backdrop-filter: blur(var(--blur-radius));
    background: var(--hy-mobile-dock-bg);
    border: 1px solid var(--hy-mobile-dock-border);
    border-radius: var(--hy-mobile-dock-radius);
    bottom: var(--hy-mobile-dock-bottom);
    box-sizing: border-box;
    box-shadow: var(--shadow-floating);
    display: none;
    gap: var(--space-2);
    justify-content: space-between;
    left: var(--hy-mobile-dock-bottom);
    padding: var(--hy-mobile-dock-padding);
    position: fixed;
    right: var(--hy-mobile-dock-bottom);
    z-index: 120;
  }

  .mobile-dock:has(.dock-item:nth-child(6)) {
    gap: var(--space-1);
    padding: 6px;
  }

  .dock-item {
    align-items: center;
    background: transparent;
    border: none;
    border-radius: var(--radius-identity);
    color: var(--color-fg);
    cursor: pointer;
    display: inline-flex;
    flex: 1 1 0;
    height: auto;
    justify-content: center;
    max-width: var(--hy-mobile-dock-item-size);
    padding: 0;
    text-decoration: none;
    transition:
      background var(--duration-fast) var(--ease-default),
      color var(--duration-fast) var(--ease-default);
    width: auto;
    aspect-ratio: 1 / 1;
  }

  .mobile-dock:has(.dock-item:nth-child(6)) .dock-item {
    max-width: calc(var(--hy-mobile-dock-item-size) - 8px);
  }

  .dock-item:hover {
    background: var(--overlay-hover);
  }

  .dock-item:focus-visible {
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }

  .dock-item.active {
    background: var(--accent-blue);
    color: var(--color-on-accent);
  }

  .icon-slot {
    align-items: center;
    display: inline-flex;
    height: 80%;
    justify-content: center;
    width: 80%;
  }

  .mobile-dock:has(.dock-item:nth-child(6)) .icon-slot {
    height: 80%;
    width: 80%;
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .mobile-dock {
      display: inline-flex;
    }
  }
</style>
