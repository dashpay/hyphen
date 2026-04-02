<script lang="ts">
  import type { Snippet } from "svelte";

  /** Scrollable list surface with keyboard reachability. */
  interface Props {
    readonly maxHeight?: string;
    readonly label?: string;
    readonly children: Snippet;
  }

  const { maxHeight = "420px", label = "Scrollable content", children }: Props = $props();

  let scrollListEl = $state<HTMLDivElement | null>(null);

  function handleFocusKeydown(event: KeyboardEvent): void {
    if (scrollListEl == null) {
      return;
    }

    const pageDelta = scrollListEl.clientHeight;
    const lineDelta = 40;

    if (event.key === "ArrowDown") {
      event.preventDefault();
      scrollListEl.scrollBy({ top: lineDelta, behavior: "smooth" });
      return;
    }

    if (event.key === "ArrowUp") {
      event.preventDefault();
      scrollListEl.scrollBy({ top: -lineDelta, behavior: "smooth" });
      return;
    }

    if (event.key === "PageDown" || event.key === " ") {
      event.preventDefault();
      scrollListEl.scrollBy({ top: pageDelta, behavior: "smooth" });
      return;
    }

    if (event.key === "PageUp") {
      event.preventDefault();
      scrollListEl.scrollBy({ top: -pageDelta, behavior: "smooth" });
      return;
    }

    if (event.key === "Home") {
      event.preventDefault();
      scrollListEl.scrollTo({ top: 0, behavior: "smooth" });
      return;
    }

    if (event.key === "End") {
      event.preventDefault();
      scrollListEl.scrollTo({ top: scrollListEl.scrollHeight, behavior: "smooth" });
    }
  }
</script>

<div class="scroll-list-shell">
  <button aria-label={label} class="scroll-focus" onkeydown={handleFocusKeydown} type="button"></button>
  <div bind:this={scrollListEl} class="scroll-list" style:max-height={maxHeight}>
    {@render children()}
  </div>
</div>

<style>
  .scroll-list-shell {
    position: relative;
  }

  .scroll-focus {
    background: transparent;
    border: none;
    inset: 0;
    padding: 0;
    pointer-events: none;
    position: absolute;
    z-index: 1;
  }

  .scroll-focus:focus-visible {
    outline: none;
  }

  .scroll-focus:focus-visible + .scroll-list {
    outline: 2px solid var(--accent-blue);
    outline-offset: 2px;
  }

  .scroll-list {
    display: flex;
    flex-direction: column;
    gap: var(--space-3);
    overflow-y: auto;
    overscroll-behavior: contain;
    padding-right: var(--space-1);
  }

  .scroll-list::-webkit-scrollbar {
    width: 8px;
  }

  .scroll-list::-webkit-scrollbar-thumb {
    background: var(--border-strong);
    border-radius: var(--radius-full);
  }
</style>
