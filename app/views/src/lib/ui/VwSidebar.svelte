<script lang="ts">
  /** Sidebar rail for navigation or secondary content. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly collapsed?: boolean;
    readonly children: Snippet;
  }

  const { collapsed = false, children }: Props = $props();
</script>

<aside class:collapsed class="sidebar">
  {@render children()}
</aside>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .sidebar {
    background: var(--sidebar-bg);
    border-bottom-right-radius: var(--radius-sidebar);
    border-right: 1px solid var(--sidebar-border);
    border-top-right-radius: var(--radius-sidebar);
    box-shadow: var(--shadow-sidebar);
    display: flex;
    flex-direction: column;
    flex-shrink: 0;
    gap: var(--space-2);
    overflow-y: auto;
    padding: 24px;
    transition:
      padding var(--duration-fast) var(--ease-default),
      width var(--duration-fast) var(--ease-default);
    width: 258px;
  }

  .sidebar.collapsed {
    align-items: center;
    padding: 20px 12px;
    width: 84px;
  }

  @include view.media-down(tablet) {
    .sidebar {
      display: none;
    }
  }
</style>
