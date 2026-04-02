<script lang="ts">
  /** Shell layout that composes the route header and body regions. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly body: Snippet;
    readonly header: Snippet;
  }

  const { body, header }: Props = $props();
</script>

<div class="page-layout">
  {@render header()}
  {@render body()}
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  :global(.app-shell) {
    height: 100dvh !important;
    overflow: hidden !important;
  }

  :global(.app-content) {
    display: flex !important;
    flex-direction: column !important;
    min-width: 0 !important;
    overflow: hidden !important;
  }

  .page-layout {
    container-type: inline-size;
    display: grid;
    flex: 1;
    gap: var(--hy-shell-page-gap);
    grid-template-rows: auto minmax(0, 1fr);
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  :global(.page-layout > *) {
    min-height: 0;
    min-width: 0;
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    :global(.app-shell) {
      border-radius: 0 !important;
    }
  }
</style>
