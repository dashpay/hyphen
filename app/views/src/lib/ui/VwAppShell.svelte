<script lang="ts">
  /** App shell with sidebar and scrollable content regions. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly sidebar?: Snippet;
    readonly children: Snippet;
  }

  const { sidebar, children }: Props = $props();
</script>

<div class="app-shell">
  {#if sidebar}
    {@render sidebar()}
  {/if}
  <main class="app-content">
    {@render children()}
  </main>
</div>

<style>
  .app-shell {
    background: var(--color-bg);
    border-radius: var(--radius-frame);
    display: flex;
    min-height: 100dvh;
    overflow: hidden;
    position: relative;
  }

  .app-content {
    flex: 1;
    overflow-y: auto;
    padding: 20px;
  }
</style>
