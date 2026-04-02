<script lang="ts">
  /** Two-column page layout for summary and detail compositions. */
  import type { HyUtilBreakpointName } from "$lib/HyUtil";
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
    readonly mode?: "even" | "left-heavy" | "right-heavy";
    readonly stackAt?: HyUtilBreakpointName;
  }

  const { children, mode = "even", stackAt = "tablet" }: Props = $props();
</script>

<div
  class:left-heavy={mode === "left-heavy"}
  class:right-heavy={mode === "right-heavy"}
  class:stack-at-desktop={stackAt === "desktop"}
  class:stack-at-laptop={stackAt === "laptop"}
  class:stack-at-narrow-phone={stackAt === "narrowPhone"}
  class:stack-at-phone={stackAt === "phone"}
  class:stack-at-tablet={stackAt === "tablet"}
  class:stack-at-ultrawide={stackAt === "ultrawide"}
  class="page-columns"
>
  {@render children()}
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .page-columns {
    display: grid;
    gap: var(--hy-browser-gap);
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    height: 100%;
    min-width: 0;
  }

  .page-columns.left-heavy {
    grid-template-columns: minmax(0, 1fr) var(--hy-page-columns-side-width);
  }

  .page-columns.right-heavy {
    grid-template-columns: var(--hy-page-columns-side-width) minmax(0, 1fr);
  }

  /* --hy-breakpoint-desktop */
  @include view.media-up(desktop) {
    .page-columns.left-heavy,
    .page-columns.right-heavy {
      grid-template-columns: minmax(0, 1fr) var(--hy-page-columns-side-width-wide);
    }

    .page-columns.right-heavy {
      grid-template-columns: var(--hy-page-columns-side-width-wide) minmax(0, 1fr);
    }
  }

  /* --hy-breakpoint-ultrawide */
  @include view.media-down(ultrawide) {
    .page-columns.stack-at-ultrawide {
      grid-template-columns: 1fr;
    }
  }

  /* --hy-breakpoint-desktop */
  @include view.media-down(desktop) {
    .page-columns.stack-at-desktop {
      grid-template-columns: 1fr;
    }
  }

  /* --hy-breakpoint-laptop */
  @include view.media-down(laptop) {
    .page-columns.stack-at-laptop {
      grid-template-columns: 1fr;
    }
  }

  /* --hy-breakpoint-tablet */
  @include view.media-down(tablet) {
    .page-columns.stack-at-tablet {
      grid-template-columns: 1fr;
    }
  }

  /* --hy-breakpoint-phone */
  @include view.media-down(phone) {
    .page-columns.stack-at-phone {
      grid-template-columns: 1fr;
    }
  }

  /* --hy-breakpoint-narrow-phone */
  @include view.media-down(narrow-phone) {
    .page-columns.stack-at-narrow-phone {
      grid-template-columns: 1fr;
    }
  }
</style>
