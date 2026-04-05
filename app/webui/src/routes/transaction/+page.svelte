<script lang="ts">
  import { page } from "$app/state";
  import { tr } from "@repo/hyphen-i18n";
  import { onMount } from "svelte";

  import type { ApiTxDetail, TransactionPayloadField, TransactionRenderer } from "$lib";
  import { genericPayload, getBlock, getTransaction, getTransactionRenderer } from "$lib";
  import { HyUtilReadErrorMessage, HyUtilReadErrorStatus, HyUtilWebUiName } from "$lib/HyUtil";
  import { observeDockMode } from "$lib/config/layout.svelte";
  import { getTransactionDisplay } from "$lib/tx/display";
  import { HyTxDetailPanel, HyTxSummaryPanel } from "$lib/tx/views";
  import { HyDetailRouteState, HyPageFrame } from "$lib/views";

  let transaction = $state<ApiTxDetail | null>(null);
  let transactionHash = $state<string | null>(null);
  let loading = $state(true);
  let errorMessage = $state<string | null>(null);
  let errorStatus = $state<number | null>(null);
  let transactionRenderer = $state<TransactionRenderer | null>(null);
  let payloadFields = $state<readonly TransactionPayloadField[]>([]);
  let transactionBlockTime = $state<string | null>(null);
  let dockMode = $state(false);
  const transactionLabel = $derived.by((): string => {
    if (transaction == null || transactionRenderer == null) {
      return tr("Transaction");
    }

    return getTransactionDisplay(transactionRenderer.type, transaction.blockPos).label;
  });

  onMount((): (() => void) => {
    return observeDockMode((nextValue: boolean): void => {
      dockMode = nextValue;
    });
  });

  $effect((): void => {
    const nextHash = page.url.searchParams.get("hash");

    transactionHash = nextHash;
    transaction = null;
    transactionRenderer = null;
    payloadFields = [];
    transactionBlockTime = null;
    errorMessage = null;
    errorStatus = null;

    if (nextHash == null) {
      loading = false;
      errorMessage = tr("Transaction not found.");
      return;
    }

    loading = true;

    void getTransaction(nextHash)
      .then((response): void => {
        transaction = response;
        transactionHash = response?.txHash ?? nextHash;
        if (response == null) {
          transactionRenderer = null;
          payloadFields = [];
          return;
        }

        const renderer = getTransactionRenderer(response);
        transactionRenderer = renderer;
        payloadFields = renderer.parsePayload?.(response) ?? genericPayload(response);

        if (response.blockHeight != null) {
          void getBlock(response.blockHeight)
            .then((block): void => {
              transactionBlockTime = block?.blockTime ?? null;
            })
            .catch((): void => {
              transactionBlockTime = null;
            });
        }
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
      transactionHash == null ? tr("Transaction") : tr("Transaction {}", transactionHash)
    )}
  </title>
</svelte:head>

<HyPageFrame>
  <HyDetailRouteState
    {loading}
    {errorMessage}
    {errorStatus}
    missing={transaction == null || transactionRenderer == null}
    missingLabel={tr("Transaction not found.")}
  >
    {#if transaction != null && transactionRenderer != null}
      <div class:transaction-route-dock={dockMode} class="transaction-route">
        <div class="transaction-summary">
          <HyTxSummaryPanel
            {transaction}
            blockTime={transactionBlockTime}
            transactionHash={transactionHash ?? transaction.txHash}
            {transactionLabel}
            {transactionRenderer}
          />
        </div>
        <div class="transaction-detail">
          <HyTxDetailPanel
            blockTime={transactionBlockTime}
            fullHeight={!dockMode}
            {payloadFields}
            scrollMode={dockMode ? "page" : "self"}
            {transaction}
            {transactionRenderer}
          />
        </div>
      </div>
    {/if}
  </HyDetailRouteState>
</HyPageFrame>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .transaction-route {
    display: grid;
    gap: var(--hy-browser-gap);
    grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
    height: 100%;
    min-width: 0;
  }

  .transaction-summary,
  .transaction-detail {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  .transaction-route.transaction-route-dock {
    display: flex;
    flex-direction: column;
    height: auto;
    overflow: visible;
  }

  .transaction-route.transaction-route-dock .transaction-summary,
  .transaction-route.transaction-route-dock .transaction-detail {
    display: block;
    min-height: auto;
    overflow: visible;
  }

  .transaction-route.transaction-route-dock .transaction-summary {
    flex: 0 0 auto;
    order: -1;
  }

  .transaction-route.transaction-route-dock .transaction-detail {
    flex: 0 0 auto;
  }

  @include view.media-up(desktop) {
    .transaction-route {
      grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
    }
  }
</style>
