<script lang="ts">
  /** Header row for list panels with title and trailing actions. */
  import type { Snippet } from "svelte";

  import { VwText } from "@repo/hyphen-views";

  interface Props {
    readonly action?: Snippet | undefined;
    readonly subtitle?: string | undefined;
    readonly title: string;
  }

  const { action, subtitle, title }: Props = $props();
</script>

<div class="list-header">
  <div class="title-row">
    <VwText as="h2" size="md+" weight="extrabold">{title}</VwText>
    {#if subtitle != null}
      <span class="subtitle-wrap">
        <VwText color="muted" size="sm" weight="medium">{subtitle}</VwText>
      </span>
    {/if}
  </div>
  {#if action != null}
    <div class="action-slot">
      {@render action()}
    </div>
  {/if}
</div>

<style>
  .list-header {
    align-items: center;
    border-bottom: 1px solid var(--glass-border);
    box-sizing: border-box;
    display: flex;
    flex-shrink: 0;
    gap: var(--hy-inline-gap);
    justify-content: space-between;
    min-height: var(--hy-panel-header-min-height);
    padding: var(--hy-list-panel-header-padding);
  }

  .title-row {
    align-items: center;
    display: flex;
    flex: 1;
    gap: var(--hy-inline-gap);
    min-width: 0;
  }

  .subtitle-wrap {
    margin-left: auto;
    text-align: right;
  }

  .action-slot {
    flex-shrink: 0;
  }
</style>
