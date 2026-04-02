<script lang="ts">
  /** Page container that applies the shared content rhythm. */
  import type { Snippet } from "svelte";

  interface Props {
    readonly eyebrow?: string;
    readonly title: string;
    readonly subtitle?: string;
    readonly actions?: Snippet;
    readonly children: Snippet;
  }

  const { eyebrow, title, subtitle, actions, children }: Props = $props();
</script>

<section class="page">
  <header class="page-header">
    <div class="copy">
      {#if eyebrow}
        <span class="eyebrow">{eyebrow}</span>
      {/if}
      <h1 class="title">{title}</h1>
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
  <div class="content">
    {@render children()}
  </div>
</section>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .page {
    display: flex;
    flex-direction: column;
    gap: var(--space-6);
  }

  .page-header {
    align-items: flex-start;
    display: flex;
    gap: var(--space-5);
    justify-content: space-between;
  }

  .copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    min-width: 0;
  }

  .eyebrow {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .title {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-4xl);
    font-weight: 600;
    line-height: 1.1;
    margin: 0;
  }

  .subtitle {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-md-plus);
    line-height: 1.5;
    margin: 0;
    max-width: 72ch;
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
    gap: var(--space-6);
  }

  @include view.media-down(tablet) {
    .page-header {
      flex-direction: column;
    }

    .actions {
      width: 100%;
    }
  }
</style>
