<script lang="ts">
  /** Older/newer route browser for relative history navigation. */
  import { tr } from "@repo/hyphen-i18n";

  interface Props {
    readonly canGoNewer?: boolean;
    readonly canGoOlder?: boolean;
    readonly loading?: boolean;
    readonly onnewer: () => void;
    readonly onolder: () => void;
    readonly pageLabel: string;
  }

  const { canGoNewer = false, canGoOlder = false, loading = false, onnewer, onolder, pageLabel }: Props = $props();
</script>

<div class="browser-nav">
  <button class="history-btn" disabled={!canGoNewer || loading} onclick={onnewer} type="button">{tr("Newer")}</button>
  <span class="page-label">{pageLabel}</span>
  <button class="history-btn" disabled={!canGoOlder || loading} onclick={onolder} type="button">{tr("Older")}</button>
</div>

<style>
  .browser-nav {
    align-items: center;
    display: flex;
    gap: var(--hy-browser-gap);
    justify-content: center;
    padding: var(--hy-browser-gap);
  }

  .history-btn {
    background: var(--surface-overlay);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-input);
    color: var(--color-fg);
    cursor: pointer;
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    font-weight: 600;
    min-width: 96px;
    padding: var(--space-2) var(--space-4);
    transition:
      background var(--duration-fast) var(--ease-default),
      color var(--duration-fast) var(--ease-default);
  }

  .history-btn:hover:not(:disabled) {
    background: var(--overlay-hover);
  }

  .history-btn:disabled {
    cursor: default;
    opacity: 0.5;
  }

  .page-label {
    color: var(--text-muted);
    font-family: var(--font-primary);
    font-size: var(--text-sm);
    font-weight: 600;
    min-width: 72px;
    text-align: center;
  }
</style>
