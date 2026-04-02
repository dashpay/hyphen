<script lang="ts">
  /** Sidebar navigation item with Hyphen shell styling. */
  import type { Component } from "svelte";

  import { VwHStack, VwText } from "@repo/hyphen-views";

  interface Props {
    readonly active?: boolean;
    readonly collapsed?: boolean;
    readonly href: string;
    readonly icon: Component;
    readonly label: string;
  }

  const { active = false, collapsed = false, href, icon, label }: Props = $props();
</script>

<a
  aria-current={active ? "page" : undefined}
  aria-label={label}
  class:active
  class:collapsed
  class="nav-item"
  {href}
  title={label}
>
  <VwHStack align="center" gap={12}>
    {@const Icon = icon}
    <span class="icon-slot">
      <Icon aria-hidden="true" height="18" width="18"></Icon>
    </span>
    {#if !collapsed}
      <VwText color={active ? "on-accent" : "default"} size="sm" weight={active ? "extrabold" : "medium"}>
        {label}
      </VwText>
    {/if}
  </VwHStack>
</a>

<style>
  .nav-item {
    align-items: center;
    background: var(--overlay-subtle);
    border: none;
    border-radius: var(--radius-identity);
    color: var(--color-fg);
    cursor: pointer;
    display: flex;
    min-height: var(--hy-shell-nav-item-min-height);
    padding: var(--hy-shell-nav-item-padding);
    text-decoration: none;
    transition:
      background var(--duration-fast) var(--ease-default),
      font-weight var(--duration-fast) var(--ease-default);
    width: 100%;
  }

  .nav-item:hover {
    background: var(--overlay-hover);
  }

  .nav-item:focus-visible {
    outline: 2px solid var(--accent-blue);
    outline-offset: -2px;
  }

  .nav-item.active {
    background: var(--accent-blue);
    color: var(--color-on-accent);
  }

  .nav-item.collapsed {
    height: var(--hy-shell-control-height);
    justify-content: center;
    min-width: var(--hy-shell-control-min-width);
    padding: var(--hy-shell-control-inline-padding);
    width: var(--hy-shell-control-min-width);
  }

  .nav-item :global(.hstack) {
    align-items: center;
    width: 100%;
  }

  .nav-item.collapsed :global(.hstack) {
    width: auto;
    justify-content: center;
  }

  .icon-slot {
    align-items: center;
    display: inline-flex;
    flex-shrink: 0;
    height: var(--hy-shell-icon-size);
    justify-content: center;
    width: var(--hy-shell-icon-size);
  }
</style>
