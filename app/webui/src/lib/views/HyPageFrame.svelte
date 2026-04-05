<script lang="ts">
  /** Page frame that applies the shared route padding and width. */
  import { page } from "$app/state";
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
  }

  const { children }: Props = $props();
  let frame = $state<HTMLDivElement | null>(null);

  $effect((): void => {
    if (frame == null) {
      return;
    }

    void page.url.pathname;
    frame.scrollTop = 0;
  });
</script>

<div bind:this={frame} class="page-frame">
  {@render children()}
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .page-frame {
    container-type: inline-size;
    display: flex;
    flex-direction: column;
    gap: var(--hy-browser-gap);
    height: 100%;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  :global(.page-frame > *) {
    min-height: 0;
    min-width: 0;
  }

  :global(.page-frame > :first-child) {
    flex: 1 1 auto;
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .page-frame {
      overflow: auto;
      padding-bottom: var(--hy-mobile-dock-clearance);
      scroll-padding-bottom: var(--hy-mobile-dock-clearance);
    }

    :global(.page-frame > :first-child) {
      flex: 0 0 auto;
    }
  }
</style>
