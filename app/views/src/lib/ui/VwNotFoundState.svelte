<script lang="ts">
  /** Not-found surface for missing records or routes. */
  import { tr } from "@repo/hyphen-i18n";

  interface Props {
    readonly title?: string;
    readonly message?: string;
  }

  const { title = tr("Page not found"), message = tr("This page could not be found.") }: Props = $props();
</script>

<div class="feedback-shell">
  <div aria-hidden="true" class="glyph">?</div>
  <div class="copy">
    <h2>{title}</h2>
    <p>{message}</p>
  </div>
</div>

<style>
  .feedback-shell {
    align-items: center;
    background: var(--surface-base);
    border: 1px dashed var(--border-strong);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-card);
    display: flex;
    flex-direction: column;
    gap: var(--space-4);
    justify-content: center;
    min-height: 280px;
    padding: 32px;
    text-align: center;
  }

  .glyph {
    align-items: center;
    background: var(--surface-raised);
    border-radius: var(--radius-full);
    color: var(--text-muted);
    display: inline-flex;
    font-family: var(--font-mono);
    font-size: var(--text-2xl);
    font-weight: 700;
    height: 48px;
    justify-content: center;
    width: 48px;
  }

  .copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 44ch;
  }

  h2,
  p {
    margin: 0;
  }

  h2 {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-xl);
    font-weight: 700;
  }

  p {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-base-plus);
    line-height: 1.5;
  }
</style>
