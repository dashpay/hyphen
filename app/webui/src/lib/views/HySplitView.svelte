<script lang="ts">
  /** Split panel layout for paired summary and detail surfaces. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly children?: Snippet;
    readonly columns?: string;
  }

  const { children, columns = "minmax(0, 1fr) minmax(0, 1fr)" }: Props = $props();
</script>

<div class="split-view" style:--hy-split-columns={columns}>
  {@render children?.()}
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .split-view {
    container-type: inline-size;
    display: grid;
    gap: var(--hy-split-gap);
    grid-template-columns: var(--hy-split-columns);
    grid-template-rows: minmax(0, 1fr);
    height: 100%;
    min-height: 0;
    overflow: hidden;
  }

  :global(.split-view > .panel) {
    display: grid;
    height: 100%;
    grid-template-rows: auto minmax(0, 1fr);
    min-height: 0;
    overflow: hidden;
  }

  :global(.split-view > .panel > :first-child) {
    min-height: 0;
    min-width: 0;
  }

  :global(.split-view > .panel > .scroll-body) {
    min-height: 0;
    min-width: 0;
  }

  /* --hy-breakpoint-tablet */
  @include view.container-down(tablet) {
    .split-view {
      align-content: start;
      display: flex;
      flex-direction: column;
      height: auto;
      overflow: visible;
    }

    :global(.split-view > *) {
      flex: none;
      height: auto !important;
      min-height: 0;
      overflow: visible;
    }

    :global(.split-view > .panel) {
      display: block;
      height: auto;
      overflow: visible;
    }

    :global(.split-view > .panel > .scroll-body) {
      flex: none;
      overflow: visible;
    }
  }
</style>
