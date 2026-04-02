<script lang="ts">
  /** Panel header for route titles, back links, and actions. */
  import { VwText } from "@repo/hyphen-views";

  interface Props {
    readonly title: string;
    readonly subtitle?: string;
    readonly showBack?: boolean;
    readonly onback?: (() => void) | undefined;
  }

  const {
    title,
    subtitle,
    showBack = true,
    onback = (): void => {
      globalThis.history.back();
    },
  }: Props = $props();
</script>

<div class="panel-header">
  <div class="header-row">
    {#if showBack}
      <button aria-label="Go back" class="back-btn" onclick={onback} type="button">
        <svg
          aria-hidden="true"
          fill="none"
          height="20"
          stroke="currentColor"
          stroke-width="2"
          viewBox="0 0 24 24"
          width="20"
        >
          <polyline points="15 18 9 12 15 6"></polyline>
        </svg>
      </button>
    {/if}
    <div class="header-text">
      <VwText as="h2" size="md+" weight="extrabold">{title}</VwText>
      {#if subtitle != null}
        <span class="subtitle-wrap">
          <VwText color="muted" size="sm" weight="medium">{subtitle}</VwText>
        </span>
      {/if}
    </div>
  </div>
</div>

<style>
  .panel-header {
    align-items: center;
    border-bottom: 1px solid var(--glass-border);
    display: flex;
    flex-shrink: 0;
    min-height: var(--hy-panel-header-min-height);
    padding: var(--hy-panel-header-padding);
  }

  .header-row {
    align-items: center;
    display: flex;
    gap: var(--hy-browser-gap);
    width: 100%;
  }

  .header-text {
    align-items: center;
    display: flex;
    flex: 1;
    gap: var(--hy-inline-gap);
    min-width: 0;
  }

  .subtitle-wrap {
    margin-left: auto;
    min-width: 0;
    text-align: right;
  }

  .back-btn {
    align-items: center;
    background: var(--overlay-subtle);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-input);
    color: var(--color-fg);
    cursor: pointer;
    display: flex;
    flex-shrink: 0;
    height: var(--hy-nav-icon-button-size);
    justify-content: center;
    transition: background var(--duration-fast) var(--ease-default);
    width: var(--hy-nav-icon-button-size);
  }

  .back-btn:hover {
    background: var(--overlay-hover);
  }
</style>
