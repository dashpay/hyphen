<script lang="ts">
  import { page } from "$app/state";
  import { tr } from "@repo/hyphen-i18n";
  import { onMount } from "svelte";

  import type { ApiValidatorDetail } from "$lib";
  import { ApiError, getValidator } from "$lib";
  import { HyUtilReadErrorMessage, HyUtilReadErrorStatus, HyUtilWebUiName } from "$lib/HyUtil";
  import { observeDockMode } from "$lib/config/layout.svelte";
  import { HyVlHistoryPanel, HyVlSummaryPanel } from "$lib/validator/views";
  import { HyDetailRouteState, HyPageFrame } from "$lib/views";

  let validator = $state<ApiValidatorDetail | null>(null);
  let loading = $state(true);
  let errorMessage = $state<string | null>(null);
  let errorStatus = $state<number | null>(null);
  let dockMode = $state(false);

  onMount((): (() => void) => {
    return observeDockMode((nextValue: boolean): void => {
      dockMode = nextValue;
    });
  });

  $effect(() => {
    const hash = page.url.searchParams.get("hash");

    validator = null;
    errorMessage = null;
    errorStatus = null;

    if (hash == null) {
      loading = false;
      return;
    }

    loading = true;

    const readValidator = async (): Promise<void> => {
      try {
        const response = await getValidator(hash);
        validator = response;
      } catch (caught: unknown) {
        if (caught instanceof ApiError && (caught.status === 400 || caught.status === 404)) {
          validator = null;
          errorMessage = null;
          errorStatus = null;
          return;
        }

        errorMessage = HyUtilReadErrorMessage(caught);
        errorStatus = HyUtilReadErrorStatus(caught);
      } finally {
        loading = false;
      }
    };

    void readValidator();

    const refreshHandle = globalThis.setInterval((): void => {
      void readValidator();
    }, 30_000);

    return (): void => {
      globalThis.clearInterval(refreshHandle);
    };
  });
</script>

<svelte:head>
  <title>
    {tr(
      "{} | {}",
      HyUtilWebUiName,
      page.url.searchParams.get("hash") == null ? tr("Validator") : tr("Validator {}", page.url.searchParams.get("hash") ?? "")
    )}
  </title>
</svelte:head>

<HyPageFrame>
  <HyDetailRouteState
    {loading}
    {errorMessage}
    {errorStatus}
    missing={validator == null}
    missingLabel={tr("Validator not found")}
  >
    {#if validator != null}
      <div class:validator-route-dock={dockMode} class="validator-route">
        <div class="validator-summary">
          <HyVlSummaryPanel {validator} />
        </div>
        <div class="validator-history">
          <HyVlHistoryPanel fullHeight={!dockMode} scrollMode={dockMode ? "page" : "self"} {validator} />
        </div>
      </div>
    {/if}
  </HyDetailRouteState>
</HyPageFrame>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .validator-route {
    display: grid;
    gap: var(--hy-browser-gap);
    grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
    height: 100%;
    min-width: 0;
  }

  .validator-summary,
  .validator-history {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  .validator-route.validator-route-dock {
    display: flex;
    flex-direction: column;
    height: auto;
    overflow: visible;
  }

  .validator-route.validator-route-dock .validator-summary,
  .validator-route.validator-route-dock .validator-history {
    display: block;
    min-height: auto;
    overflow: visible;
  }

  .validator-route.validator-route-dock .validator-summary {
    flex: 0 0 auto;
    order: -1;
  }

  .validator-route.validator-route-dock .validator-history {
    flex: 0 0 auto;
  }

  @include view.media-up(desktop) {
    .validator-route {
      grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
    }
  }
</style>
