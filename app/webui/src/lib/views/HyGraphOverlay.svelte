<script lang="ts">
  /** Large graph overlay window for expanded dashboard charts. */
  import type { Action } from "svelte/action";
  import type { Snippet } from "svelte";
  import { cubicOut } from "svelte/easing";
  import { fade } from "svelte/transition";

  interface Props {
    readonly children: Snippet;
    readonly fullscreen?: boolean;
    readonly onclick?: () => void;
  }

  const { children, fullscreen = false, onclick }: Props = $props();

  const portalToBody: Action = (node) => {
    const target = globalThis.document.body;
    target.appendChild(node);

    return {
      destroy(): void {
        node.remove();
      },
    };
  };
</script>

<!-- svelte-ignore a11y_no_static_element_interactions -->
<div
  class="graph-overlay"
  class:fullscreen
  {onclick}
  role="presentation"
  transition:fade={{ duration: 300, easing: cubicOut }}
  use:portalToBody
>
  <!-- svelte-ignore a11y_click_events_have_key_events -->
  <!-- svelte-ignore a11y_no_static_element_interactions -->
  <div
    class="graph-overlay-inner"
    onclick={(event: MouseEvent): void => {
      event.stopPropagation();
    }}
  >
    {@render children()}
  </div>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .graph-overlay {
    align-items: center;
    backdrop-filter: blur(var(--blur-radius));
    background: var(--surface-overlay);
    display: flex;
    inset: 0;
    justify-content: center;
    position: fixed;
    z-index: 320;
  }

  .graph-overlay-inner {
    height: var(--hy-chart-overlay-height);
    max-height: calc(100dvh - var(--hy-chart-overlay-viewport-gap));
    max-width: calc(100vw - var(--hy-chart-overlay-viewport-gap));
    width: var(--hy-chart-overlay-width);
  }

  .graph-overlay.fullscreen {
    align-items: stretch;
    justify-content: stretch;
  }

  .graph-overlay.fullscreen .graph-overlay-inner {
    height: 100dvh;
    max-height: 100dvh;
    max-width: 100vw;
    width: 100vw;
  }

  /* --hy-breakpoint-phone */
  @include view.media-down(phone) {
    .graph-overlay-inner {
      height: var(--hy-chart-overlay-height-phone);
      width: var(--hy-chart-overlay-width-phone);
    }
  }
</style>
