<script lang="ts">
  /** Loading placeholder for pages, panels, or lists. */
  import { tr } from "@repo/hyphen-i18n";

  interface Props {
    readonly title?: string;
    readonly message?: string;
  }

  const { title = tr("Loading"), message = tr("Fetching the latest explorer data and preparing the view.") }: Props =
    $props();
</script>

<div aria-live="polite" class="feedback-shell" role="status">
  <div aria-hidden="true" class="spinner"></div>
  <div class="copy">
    <h2>{title}</h2>
    <p>{message}</p>
  </div>
</div>

<style>
  .feedback-shell {
    align-items: center;
    background: var(--surface-base);
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

  .spinner {
    animation: spin 1s linear infinite;
    border: 3px solid var(--surface-raised);
    border-radius: var(--radius-full);
    border-top-color: var(--accent-blue);
    height: 40px;
    width: 40px;
  }

  .copy {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    max-width: 42ch;
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

  @keyframes spin {
    from {
      transform: rotate(0deg);
    }

    to {
      transform: rotate(360deg);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .spinner {
      animation: none;
    }
  }
</style>
