<script lang="ts">
  /** Scroll body that keeps panel headers fixed above long content. */
  import { page } from "$app/state";
  import type { Snippet } from "svelte";
  import { VwThrobber } from "@repo/hyphen-views";

  interface Props {
    readonly container?: boolean;
    readonly hasMore?: boolean;
    readonly loadingMore?: boolean;
    readonly onloadmore?: (() => unknown) | undefined;
    readonly scrollMode?: "page" | "self";
    readonly children: Snippet;
  }

  const {
    container = false,
    hasMore = false,
    loadingMore = false,
    onloadmore,
    scrollMode = "self",
    children,
  }: Props = $props();

  let scrollBody = $state<HTMLDivElement | null>(null);
  let loadRequested = $state(false);
  let hasUserScrolled = $state(false);

  const LOAD_AHEAD_PX = 240;

  function getScrollNode(): HTMLDivElement | null {
    if (scrollBody == null) {
      return null;
    }

    if (scrollMode === "page") {
      return scrollBody.closest(".page-frame");
    }

    return scrollBody;
  }

  function shouldLoadMore(node: HTMLDivElement): boolean {
    if (!hasMore || loadingMore || onloadmore == null || loadRequested || !hasUserScrolled) {
      return false;
    }

    const remaining = node.scrollHeight - node.clientHeight - node.scrollTop;
    return remaining <= LOAD_AHEAD_PX;
  }

  function maybeLoadMore(): void {
    const node = getScrollNode();
    if (node == null || !shouldLoadMore(node)) {
      return;
    }

    const loadMore = onloadmore;
    if (loadMore == null) {
      return;
    }

    loadRequested = true;
    void Promise.resolve(loadMore()).finally((): void => {
      loadRequested = false;
      if (shouldLoadMore(node)) {
        globalThis.requestAnimationFrame((): void => {
          maybeLoadMore();
        });
      }
    });
  }

  $effect(() => {
    const node = getScrollNode();
    if (node == null) {
      return;
    }

    const handleScroll = (): void => {
      if (node.scrollTop > 0) {
        hasUserScrolled = true;
      }

      maybeLoadMore();
    };

    node.addEventListener("scroll", handleScroll, { passive: true });

    return (): void => {
      node.removeEventListener("scroll", handleScroll);
    };
  });

  $effect((): void => {
    const node = getScrollNode();
    if (node == null) {
      return;
    }

    void page.url.pathname;
    if (scrollMode === "self") {
      node.scrollTop = 0;
    }

    hasUserScrolled = false;
    loadRequested = false;
  });
</script>

<div bind:this={scrollBody} class:container class:page-mode={scrollMode === "page"} class="scroll-body">
  {@render children()}
  {#if hasMore || loadingMore}
    <div class="load-footer">
      {#if loadingMore}
        <VwThrobber />
      {/if}
    </div>
  {/if}
</div>

<style>
  .scroll-body {
    flex: 1;
    min-height: 0;
    overflow-x: hidden;
    overflow-y: auto;
  }

  .scroll-body.container {
    container-type: inline-size;
  }

  .scroll-body.page-mode {
    overflow: visible;
  }

  .scroll-body::-webkit-scrollbar {
    width: var(--hy-scrollbar-width);
  }

  .scroll-body::-webkit-scrollbar-track {
    background: transparent;
  }

  .scroll-body::-webkit-scrollbar-thumb {
    background: var(--hy-scrollbar-thumb);
    border-radius: var(--hy-scrollbar-thumb-radius);
  }

  .scroll-body::-webkit-scrollbar-thumb:hover {
    background: var(--hy-scrollbar-thumb-hover);
  }

  .load-footer {
    align-items: center;
    display: flex;
    justify-content: center;
    min-height: 48px;
    padding: var(--space-3);
  }
</style>
