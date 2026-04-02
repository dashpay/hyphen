<script lang="ts">
  import { onMount } from "svelte";
  import { goto } from "$app/navigation";
  import { page } from "$app/state";

  import type { ApiAddressSummary, ApiAddressTxSummary } from "$lib";
  import { getAddress, getAddressTxs } from "$lib";
  import { HyUtilReadErrorMessage, HyUtilReadErrorStatus } from "$lib/HyUtil";
  import { observeDockMode } from "$lib/config/layout.svelte";
  import { HyAdSummaryPanel, HyAdTxPanel } from "$lib/address/views";
  import { getTransactionDisplay, type TransactionTone } from "$lib/tx/display";
  import { HyDetailRouteState, HyPageFrame } from "$lib/views";

  interface AddressTransactionRow extends ApiAddressTxSummary {
    readonly label: string;
    readonly tone: TransactionTone;
  }

  interface AddressPageData {
    readonly nextCursor: string | null;
    readonly summary: ApiAddressSummary;
    readonly transactions: readonly AddressTransactionRow[];
  }

  let summary = $state<ApiAddressSummary | null>(null);
  let transactions = $state<readonly AddressTransactionRow[]>([]);
  let loading = $state(true);
  let loadingMore = $state(false);
  let errorMessage = $state<string | null>(null);
  let errorStatus = $state<number | null>(null);
  let nextCursor = $state<string | null>(null);
  let dockMode = $state(false);

  onMount((): (() => void) => {
    return observeDockMode((nextValue: boolean): void => {
      dockMode = nextValue;
    });
  });

  function mapAddressTransactions(rows: readonly ApiAddressTxSummary[]): readonly AddressTransactionRow[] {
    return rows.map((tx): AddressTransactionRow => {
      const display = getTransactionDisplay(tx.txType, tx.blockPos);
      return {
        ...tx,
        label: display.label,
        tone: display.tone,
      };
    });
  }

  async function loadMore(): Promise<void> {
    const address = summary?.address;

    if (loadingMore || nextCursor == null || address == null) {
      return;
    }

    loadingMore = true;

    try {
      const txResponse = await getAddressTxs(address, 50, nextCursor);
      transactions = [...transactions, ...mapAddressTransactions(txResponse.value)];
      nextCursor = txResponse.nextCursor ?? null;
    } catch {
      nextCursor = null;
    } finally {
      loadingMore = false;
    }
  }

  $effect((): void => {
    const address = page.url.searchParams.get("id");

    summary = null;
    transactions = [];
    nextCursor = null;
    errorMessage = null;
    errorStatus = null;

    if (address == null) {
      loading = false;
      return;
    }

    loading = true;

    void getAddress(address)
      .then(async (response): Promise<AddressPageData | null> => {
        if (response == null) {
          return null;
        }

        const txResponse = await getAddressTxs(response.address, 50);

        return {
          summary: response,
          nextCursor: txResponse.nextCursor ?? null,
          transactions: mapAddressTransactions(txResponse.value),
        };
      })
      .then((response): void => {
        if (response == null) {
          summary = null;
          transactions = [];
          nextCursor = null;
          return;
        }

        summary = response.summary;
        transactions = response.transactions;
        nextCursor = response.nextCursor;
      })
      .catch((caught: unknown): void => {
        summary = null;
        transactions = [];
        nextCursor = null;
        errorMessage = HyUtilReadErrorMessage(caught);
        errorStatus = HyUtilReadErrorStatus(caught);
      })
      .finally((): void => {
        loading = false;
      });
  });
</script>

<HyPageFrame>
  <HyDetailRouteState
    {loading}
    {errorMessage}
    {errorStatus}
    missing={summary == null || page.url.searchParams.get("id") == null}
    missingLabel="Address not found"
  >
    {#if summary != null}
      <div class:dock-mode={dockMode} class="address-explorer">
        <div class="address-summary">
          <HyAdSummaryPanel address={page.url.searchParams.get("id") ?? ""} {summary} />
        </div>
        <div class="address-transactions">
          <HyAdTxPanel
            fullHeight={!dockMode}
            hasMore={nextCursor != null}
            {loadingMore}
            onloadmore={loadMore}
            scrollMode={dockMode ? "page" : "self"}
            totalCount={summary.txCount}
            {transactions}
            ontxclick={(txid: string): void => {
              void goto("/transaction?hash=" + txid);
            }}
          />
        </div>
      </div>
    {/if}
  </HyDetailRouteState>
</HyPageFrame>

<style lang="scss">
  @use "_views-mixins.scss" as view;

  .address-explorer {
    display: grid;
    gap: var(--hy-browser-gap);
    grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
    height: 100%;
    min-width: 0;
  }

  .address-summary,
  .address-transactions {
    display: flex;
    flex-direction: column;
    min-height: 0;
    min-width: 0;
    overflow: hidden;
  }

  .address-explorer.dock-mode {
    display: flex;
    flex-direction: column;
    height: auto;
    overflow: visible;
  }

  .address-explorer.dock-mode .address-summary,
  .address-explorer.dock-mode .address-transactions {
    display: block;
    min-height: auto;
    overflow: visible;
  }

  .address-explorer.dock-mode .address-summary {
    flex: 0 0 auto;
    order: -1;
  }

  .address-explorer.dock-mode .address-transactions {
    flex: 0 0 auto;
  }

  /* --hy-breakpoint-desktop */
  @include view.media-up(desktop) {
    .address-explorer {
      grid-template-columns: minmax(0, 2fr) minmax(0, 3fr);
    }
  }
</style>
