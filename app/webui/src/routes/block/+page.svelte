<script lang="ts">
  import { goto } from "$app/navigation";
  import { page } from "$app/state";
  import { tr } from "@repo/hyphen-i18n";
  import { onMount } from "svelte";

  import type { ApiBlockDetail } from "$lib";
  import { getBlock } from "$lib";
  import { HyUtilReadErrorMessage, HyUtilReadErrorStatus, HyUtilWebUiName } from "$lib/HyUtil";
  import { observeDockMode } from "$lib/config/layout.svelte";
  import { HyBlSummaryPanel, HyBlTxPanel } from "$lib/block/views";
  import { HyDetailRouteState, HyPageFrame } from "$lib/views";

  let block = $state<ApiBlockDetail | null>(null);
  let loading = $state(true);
  let errorMessage = $state<string | null>(null);
  let errorStatus = $state<number | null>(null);
  let dockMode = $state(false);

  onMount((): (() => void) => {
    return observeDockMode((nextValue: boolean): void => {
      dockMode = nextValue;
    });
  });

  $effect((): void => {
    const hash = page.url.searchParams.get("hash");
    const height = page.url.searchParams.get("height");
    const query = hash ?? height;

    block = null;
    errorMessage = null;
    errorStatus = null;

    if (query == null) {
      loading = false;
      return;
    }

    loading = true;

    void getBlock(query)
      .then((response): void => {
        block = response;
      })
      .catch((caught: unknown): void => {
        errorMessage = HyUtilReadErrorMessage(caught);
        errorStatus = HyUtilReadErrorStatus(caught);
      })
      .finally((): void => {
        loading = false;
      });
  });
</script>

<svelte:head>
  <title>
    {tr(
      "{} | {}",
      HyUtilWebUiName,
      page.url.searchParams.get("hash") != null
        ? tr("Block {}", page.url.searchParams.get("hash") ?? "")
        : page.url.searchParams.get("height") != null
          ? tr("Block {}", page.url.searchParams.get("height") ?? "")
          : tr("Block")
    )}
  </title>
</svelte:head>

<HyPageFrame>
  <HyDetailRouteState
    {loading}
    {errorMessage}
    {errorStatus}
    missing={block == null}
    missingLabel={tr("Block not found")}
  >
    {#if block != null}
      <div class:block-route-dock={dockMode} class="block-route">
        <div class="block-summary">
          <HyBlSummaryPanel {block} />
        </div>
        <div class="block-transactions">
          <HyBlTxPanel
            {block}
            fullHeight={!dockMode}
            ontxclick={(txid: string): void => {
              void goto("/transaction?hash=" + txid);
            }}
            scrollMode={dockMode ? "page" : "self"}
          />
        </div>
      </div>
    {/if}
  </HyDetailRouteState>
</HyPageFrame>

<!-- nosemgrep: contrib.r2c.svelte-routes-no-style-blocks -->
<style lang="scss">
  @use "_views-mixins.scss" as view;

  .block-route {
    display: grid;
    gap: var(--hy-browser-gap);
    grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
    height: 100%;
    min-width: 0;
  }

  .block-summary,
  .block-transactions {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  .block-route.block-route-dock {
    display: flex;
    flex-direction: column;
    height: auto;
    overflow: visible;
  }

  .block-route.block-route-dock .block-summary,
  .block-route.block-route-dock .block-transactions {
    display: block;
    min-height: auto;
    overflow: visible;
  }

  .block-route.block-route-dock .block-summary {
    flex: 0 0 auto;
    order: -1;
  }

  .block-route.block-route-dock .block-transactions {
    flex: 0 0 auto;
  }

  @include view.media-up(desktop) {
    .block-route {
      grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
    }
  }
</style>
