<script lang="ts">
  /** Grid wrapper for metric cards with shared dashboard rhythm. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly children: Snippet;
    readonly columns?: number;
  }

  const { children, columns = 3 }: Props = $props();
</script>

<div class="metric-grid" style:grid-template-columns={`repeat(${String(columns)}, minmax(0, 1fr))`}>
  {@render children()}
</div>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .metric-grid {
    display: grid;
    gap: var(--hy-detail-stat-gap);
  }

  /* --hy-breakpoint-tablet */
  @include view.container-down(tablet) {
    .metric-grid {
      grid-template-columns: 1fr !important;
    }
  }
</style>
