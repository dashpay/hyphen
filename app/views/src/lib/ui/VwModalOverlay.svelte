<script lang="ts" module>
  /** Modal overlay with focus trapping and backdrop handling. */
  let modalTitleIdSequence = 0;
</script>

<script lang="ts">
  import { tick } from "svelte";
  import type { Snippet } from "svelte";

  interface Props {
    readonly open?: boolean;
    readonly title?: string;
    readonly onclose?: () => void;
    readonly children: Snippet;
  }

  const { open = false, title, onclose, children }: Props = $props();
  const titleId = `modal-title-${String(++modalTitleIdSequence)}`;
  let panelElement = $state<HTMLDivElement | null>(null);
  let closeButtonElement = $state<HTMLButtonElement | null>(null);
  let restoreFocusTo: HTMLElement | null = null;

  const FOCUSABLE_SELECTOR =
    'a[href], area[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex]:not([tabindex="-1"])';

  function getFocusableElements(): readonly HTMLElement[] {
    if (panelElement == null) {
      return [];
    }

    return Array.from(panelElement.querySelectorAll<HTMLElement>(FOCUSABLE_SELECTOR)).filter((element) => {
      return !element.hasAttribute("disabled") && element.getAttribute("aria-hidden") !== "true";
    });
  }

  function handlePanelKeydown(event: KeyboardEvent): void {
    if (!open) {
      return;
    }

    if (event.key === "Escape") {
      onclose?.();
      return;
    }

    if (event.key !== "Tab") {
      return;
    }

    const focusableElements = getFocusableElements();
    if (focusableElements.length === 0) {
      event.preventDefault();
      panelElement?.focus();
      return;
    }

    const firstElement = focusableElements[0];
    const lastElement = focusableElements[focusableElements.length - 1];
    const activeElement = globalThis.document.activeElement;

    if (event.shiftKey && activeElement === firstElement) {
      event.preventDefault();
      lastElement?.focus();
      return;
    }

    if (!event.shiftKey && activeElement === lastElement) {
      event.preventDefault();
      firstElement?.focus();
    }
  }

  $effect((): (() => void) | undefined => {
    if (!open) {
      return;
    }

    restoreFocusTo =
      globalThis.document.activeElement instanceof HTMLElement ? globalThis.document.activeElement : null;

    void tick().then(() => {
      (closeButtonElement ?? getFocusableElements()[0] ?? panelElement)?.focus();
    });

    return () => {
      restoreFocusTo?.focus();
      restoreFocusTo = null;
    };
  });
</script>

{#if open}
  <div class="backdrop">
    <button aria-label="Close overlay" class="dismiss-surface" onclick={onclose} type="button"></button>
    <div
      aria-labelledby={title != null ? titleId : undefined}
      aria-modal="true"
      bind:this={panelElement}
      class="panel"
      onkeydown={handlePanelKeydown}
      role="dialog"
      tabindex="-1"
    >
      <header class="header">
        {#if title}
          <h2 id={titleId}>{title}</h2>
        {/if}
        <button aria-label="Close overlay" bind:this={closeButtonElement} class="close" onclick={onclose} type="button">
          Close
        </button>
      </header>
      <div class="content">
        {@render children()}
      </div>
    </div>
  </div>
{/if}

<style>
  .backdrop {
    -webkit-backdrop-filter: blur(var(--blur-radius));
    align-items: center;
    backdrop-filter: blur(var(--blur-radius));
    background: rgba(12, 28, 51, 0.48);
    display: flex;
    inset: 0;
    justify-content: center;
    padding: 24px;
    position: fixed;
    z-index: 200;
  }

  .dismiss-surface {
    appearance: none;
    background: transparent;
    border: none;
    inset: 0;
    padding: 0;
    position: absolute;
  }

  .panel {
    background: var(--surface-base);
    border: 1px solid var(--glass-border);
    border-radius: var(--radius-card);
    box-shadow: var(--shadow-card);
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    max-height: min(90dvh, 880px);
    max-width: min(960px, 100%);
    overflow: hidden;
    position: relative;
    width: min(720px, 100%);
  }

  .header {
    align-items: center;
    border-bottom: 1px solid var(--border-subtle);
    display: flex;
    justify-content: space-between;
    padding: 20px 24px 0;
  }

  .content {
    overflow: auto;
    padding: 0 24px 24px;
  }

  h2 {
    color: var(--color-fg);
    font-family: var(--font-primary);
    font-size: var(--text-xl);
    font-weight: 700;
    margin: 0;
  }

  .close {
    background: transparent;
    border: none;
    color: var(--text-muted);
    cursor: pointer;
    font-family: var(--font-primary);
    font-size: var(--text-sm-plus);
    font-weight: 600;
  }
</style>
