<script lang="ts">
  /** Counter pair layout for adjacent headline metric surfaces. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
  }

  const { children }: Props = $props();
</script>

<div class="counter-pair">
  {@render children()}
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .counter-pair {
    display: grid;
    gap: var(--hy-dashboard-gap);
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    min-width: 0;
  }

  /* --hy-breakpoint-tablet */
  @include view.container-down(tablet) {
    .counter-pair {
      display: flex;
      flex-direction: column;
      flex-shrink: 0;
      gap: var(--hy-dashboard-gap);
    }
  }
</style>
