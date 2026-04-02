<script lang="ts">
  /** Dashboard grid that arranges overview counters and lists. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly children?: Snippet;
    readonly left: Snippet;
    readonly right: Snippet;
  }

  const { left, right }: Props = $props();
</script>

<div class="dashboard">
  <div class="col-left">
    {@render left()}
  </div>
  <div class="col-right">
    {@render right()}
  </div>
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .dashboard {
    container-type: inline-size;
    display: grid;
    gap: var(--hy-dashboard-gap);
    grid-template-columns: minmax(0, 1fr) minmax(0, 1fr);
    grid-template-rows: minmax(0, 1fr);
    isolation: isolate;
    min-height: 0;
  }

  .col-left,
  .col-right {
    display: grid;
    gap: var(--hy-dashboard-gap);
    min-height: 0;
  }

  .col-left {
    grid-template-rows: var(--hy-dashboard-rail-height) minmax(0, 1fr);
  }

  .col-right {
    grid-template-rows: minmax(0, 1fr) var(--hy-dashboard-rail-height);
  }

  /* --hy-breakpoint-tablet */
  @include view.container-down(tablet) {
    .dashboard {
      display: flex;
      flex-direction: column;
      gap: var(--hy-dashboard-gap);
      height: auto;
      min-height: 100%;
      overflow: visible;
      padding-bottom: var(--hy-mobile-dock-clearance);
    }

    .col-left,
    .col-right {
      display: flex;
      flex-direction: column;
      flex-shrink: 0;
      gap: var(--hy-dashboard-gap);
      min-height: auto;
    }
  }
</style>
