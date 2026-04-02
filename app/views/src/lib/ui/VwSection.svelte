<script lang="ts">
  /** Section wrapper with heading and body spacing. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly title?: string;
    readonly subtitle?: string;
    readonly actions?: Snippet;
    readonly children: Snippet;
  }

  const { title, subtitle, actions, children }: Props = $props();
  const hasHeader = $derived(title !== undefined || subtitle !== undefined || actions !== undefined);
</script>

<section class="section">
  {#if hasHeader}
    <header class="section-header">
      <div class="copy">
        {#if title}
          <h2 class="title">{title}</h2>
        {/if}
        {#if subtitle}
          <p class="subtitle">{subtitle}</p>
        {/if}
      </div>
      {#if actions}
        <div class="actions">
          {@render actions()}
        </div>
      {/if}
    </header>
  {/if}
  <div class="content">
    {@render children()}
  </div>
</section>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .section {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  .section-header {
    align-items: flex-start;
    display: flex;
    gap: var(--space-4);
    justify-content: space-between;
  }

  .copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
  }

  .title {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-xl);
    font-weight: 600;
    margin: 0;
  }

  .subtitle {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-base-plus);
    line-height: 1.5;
    margin: 0;
  }

  .actions {
    align-items: center;
    display: inline-flex;
    flex-shrink: 0;
    gap: var(--space-3);
  }

  .content {
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
  }

  @include view.media-down(tablet) {
    .section-header {
      flex-direction: column;
    }

    .actions {
      width: 100%;
    }
  }
</style>
