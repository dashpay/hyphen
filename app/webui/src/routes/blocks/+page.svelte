<script lang="ts">
  import { goto } from "$app/navigation";
  import { tr } from "@repo/hyphen-i18n";
  import { onMount } from "svelte";

  import type { ApiBlockSummary } from "$lib";
  import { observeDockMode } from "$lib/config/layout.svelte";
  import { explorer, getBlocks } from "$lib";
  import { HyUtilReadErrorMessage, HyUtilReadErrorStatus, HyUtilWebUiName } from "$lib/HyUtil";
  import { HyBlTable } from "$lib/block/views";
  import { HyListBrowserPage } from "$lib/views";

  const PAGE_SIZE = 25;

  let blocks = $state<readonly ApiBlockSummary[]>([]);
  let loading = $state(true);
  let loadingMore = $state(false);
  let errorMessage = $state<string | null>(null);
  let errorStatus = $state<number | null>(null);
  let totalBlocks = $state(0);
  let nextCursor = $state<string | null>(null);
  let dockMode = $state(false);

  const latestHeight = $derived.by((): number => {
    return explorer.blocks[0]?.blockHeight ?? explorer.blockHeight;
  });

  async function readBlocks(): Promise<void> {
    if (latestHeight <= 0) {
      return;
    }

    loading = true;
    errorMessage = null;
    errorStatus = null;

    try {
      const response = await getBlocks(PAGE_SIZE);
      blocks = response.value;
      totalBlocks = explorer.blockHeight;
      nextCursor = response.nextCursor ?? null;
    } catch (caught: unknown) {
      blocks = [];
      totalBlocks = 0;
      nextCursor = null;
      errorMessage = HyUtilReadErrorMessage(caught);
      errorStatus = HyUtilReadErrorStatus(caught);
    } finally {
      loading = false;
    }
  }

  async function loadMore(): Promise<void> {
    if (loadingMore || nextCursor == null) {
      return;
    }

    loadingMore = true;

    try {
      const response = await getBlocks(PAGE_SIZE, nextCursor);
      blocks = [...blocks, ...response.value];
      nextCursor = response.nextCursor ?? null;
    } catch {
      nextCursor = null;
    } finally {
      loadingMore = false;
    }
  }

  onMount((): (() => void) => {
    const stopObservingDockMode = observeDockMode((nextValue: boolean): void => {
      dockMode = nextValue;
    });

    void readBlocks();

    return (): void => {
      stopObservingDockMode();
    };
  });
</script>

<svelte:head>
  <title>{tr("{} | {}", HyUtilWebUiName, tr("Blocks"))}</title>
</svelte:head>

<HyListBrowserPage
  {errorMessage}
  {errorStatus}
  fullHeight={!dockMode}
  {loading}
  subtitle={tr("{count, plural, one {# block} other {# blocks}}", { count: totalBlocks })}
  title={tr("Blocks")}
>
  <HyBlTable
    {blocks}
    hasMore={nextCursor != null}
    {loadingMore}
    onblockclick={(hash: string): void => {
      void goto("/block?hash=" + hash);
    }}
    onloadmore={loadMore}
    scrollMode={dockMode ? "page" : "self"}
  />
</HyListBrowserPage>
