<script lang="ts">
  /** Header row for compact page actions and status. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly left?: Snippet;
    readonly center?: Snippet;
    readonly right?: Snippet;
  }

  const { left, center, right }: Props = $props();
</script>

<header class="header">
  <div class="inner">
    <div class="left">
      {#if left}
        {@render left()}
      {/if}
    </div>
    {#if center}
      <nav class="center">
        {@render center()}
      </nav>
    {/if}
    <div class="right">
      {#if right}
        {@render right()}
      {/if}
    </div>
  </div>
</header>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .header {
    -webkit-backdrop-filter: blur(var(--blur-radius));
    align-items: center;
    background: var(--header-bg);
    backdrop-filter: blur(var(--blur-radius));
    border: 1px solid var(--header-border);
    border-radius: var(--radius-header);
    display: flex;
    height: 80px;
    padding: 0 15px;
    position: sticky;
    top: 0;
    z-index: 100;
  }

  .inner {
    align-items: center;
    display: flex;
    gap: var(--space-5);
    width: 100%;
  }

  .left {
    flex: 1;
    min-width: 0;
  }

  .left:empty,
  .right:empty {
    display: none;
  }

  .center {
    align-items: center;
    display: flex;
    flex: 1;
    gap: 60px;
    justify-content: center;
  }

  .right {
    align-items: center;
    display: flex;
    flex-shrink: 0;
    gap: var(--space-3);
    margin-left: auto;
  }

  @include view.media-down(tablet) {
    .header {
      border-radius: var(--radius-input);
      height: auto;
      padding: 12px 16px;
    }

    .inner {
      flex-wrap: nowrap;
      gap: 12px;
    }

    .left {
      flex: 1 1 0;
      min-width: 0;
    }

    .center {
      display: none;
    }

    .right {
      justify-content: flex-end;
      width: auto;
    }
  }
</style>
