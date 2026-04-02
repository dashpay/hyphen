<script lang="ts">
  /** Counter stack layout for vertical dashboard metric groups. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
  }

  const { children }: Props = $props();
</script>

<div class="counter-stack">
  {@render children()}
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .counter-stack {
    display: grid;
    gap: var(--hy-detail-stat-gap);
    grid-template-rows: minmax(0, 1fr) minmax(0, 1fr);
    min-width: 0;
  }

  /* --hy-breakpoint-tablet */
  @include view.container-down(tablet) {
    .counter-stack {
      display: flex;
      flex-direction: column;
      gap: var(--hy-detail-stat-gap);
    }
  }
</style>
