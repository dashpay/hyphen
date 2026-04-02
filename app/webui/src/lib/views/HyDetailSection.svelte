<script lang="ts">
  /** Detail section wrapper for titled content blocks. */
  import type { Snippet } from "svelte";

  import { VwText } from "@repo/hyphen-views";

  interface Props {
    readonly children: Snippet;
    readonly title?: string;
    readonly titleSize?: "sm" | "xs";
  }

  const { children, title, titleSize = "sm" }: Props = $props();
</script>

<section class="detail-section">
  {#if title != null}
    <div class="section-title">
      <VwText color="accent-blue" size={titleSize} weight="bold">{title}</VwText>
    </div>
  {/if}
  {@render children()}
</section>

<style>
  .detail-section {
    padding: var(--hy-detail-section-padding);
  }

  :global(.detail-section + .detail-section) {
    border-top: 1px solid var(--glass-border);
  }

  .section-title {
    margin-bottom: var(--hy-detail-copy-margin);
  }
</style>
