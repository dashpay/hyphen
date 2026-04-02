<script lang="ts">
  /** List panel with shared framing for compact explorer lists. */
  import type { Snippet } from "svelte";

  import { VwPanel } from "@repo/hyphen-views";

  import HyListHeader from "./HyListHeader.svelte";

  interface Props {
    readonly action?: Snippet;
    readonly children: Snippet;
    readonly fullHeight?: boolean;
    readonly subtitle?: string;
    readonly title: string;
  }

  const { action, children, fullHeight = true, subtitle, title }: Props = $props();
</script>

<div class:auto-height={!fullHeight} class="list-panel">
  <VwPanel border={true} padding={0} radius="sm" variant="glass">
    <HyListHeader {action} {subtitle} {title} />
    {@render children()}
  </VwPanel>
</div>

<style>
  .list-panel {
    height: 100%;
    min-width: 0;
  }

  .list-panel.auto-height {
    height: auto;
  }

  .list-panel :global(.panel) {
    display: grid;
    height: 100%;
    grid-template-rows: auto minmax(0, 1fr);
    min-width: 0;
    overflow: hidden;
  }

  .list-panel.auto-height :global(.panel) {
    height: auto;
  }
</style>
