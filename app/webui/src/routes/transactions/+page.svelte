<script lang="ts">
  import { goto } from "$app/navigation";
  import { tr } from "@repo/hyphen-i18n";
  import { onMount } from "svelte";

  import type { ApiMempoolSummary, ApiMempoolTxSummary } from "$lib";
  import { observeDockMode } from "$lib/config/layout.svelte";
  import { getMempool, getMempoolTxs } from "$lib";
  import { HyUtilReadErrorMessage, HyUtilReadErrorStatus, HyUtilWebUiName } from "$lib/HyUtil";
  import { hydrateMempoolTransactions } from "$lib/mempool/hydrate";
  import { HyMpFormatTransactionCount } from "$lib/mempool/HyMpUtil";
  import { HyMpTxTable } from "$lib/mempool/views";
  import { HyPageFrame, HyPageState, HyListPanel } from "$lib/views";

  const PAGE_SIZE = 50;

  let summary = $state<ApiMempoolSummary | null>(null);
  let transactions = $state<readonly ApiMempoolTxSummary[]>([]);
  let loading = $state(true);
  let loadingMore = $state(false);
  let errorMessage = $state<string | null>(null);
  let errorStatus = $state<number | null>(null);
  let nextCursor = $state<string | null>(null);
  let dockMode = $state(false);

  onMount((): (() => void) => {
    const stopObservingDockMode = observeDockMode((nextValue: boolean): void => {
      dockMode = nextValue;
    });

    void Promise.all([getMempool(), getMempoolTxs(PAGE_SIZE)])
      .then(([nextSummary, txResponse]): void => {
        summary = nextSummary;
        transactions = txResponse.value;
        nextCursor = txResponse.nextCursor ?? null;
        void hydrateTransactionTypes(txResponse.value);
      })
      .catch((caught: unknown): void => {
        errorMessage = HyUtilReadErrorMessage(caught);
        errorStatus = HyUtilReadErrorStatus(caught);
        nextCursor = null;
      })
      .finally((): void => {
        loading = false;
      });

    return (): void => {
      stopObservingDockMode();
    };
  });

  async function hydrateTransactionTypes(rows: readonly ApiMempoolTxSummary[]): Promise<void> {
    const hydratedRows = await hydrateMempoolTransactions(rows);

    const hydratedMap = new Map(hydratedRows.map((row): [string, ApiMempoolTxSummary] => [row.txHash, row]));
    transactions = transactions.map((row): ApiMempoolTxSummary => hydratedMap.get(row.txHash) ?? row);
  }

  async function loadMore(): Promise<void> {
    if (loadingMore || nextCursor == null) {
      return;
    }

    loadingMore = true;

    try {
      const response = await getMempoolTxs(PAGE_SIZE, nextCursor);
      const nextRows = response.value;
      transactions = [...transactions, ...nextRows];
      nextCursor = response.nextCursor ?? null;
      void hydrateTransactionTypes(nextRows);
    } catch {
      nextCursor = null;
    } finally {
      loadingMore = false;
    }
  }
</script>

<svelte:head>
  <title>{tr("{} | {}", HyUtilWebUiName, tr("Transactions"))}</title>
</svelte:head>

<HyPageFrame>
  <HyPageState
    empty={summary == null}
    emptyMessage={tr("No live mempool snapshot is available.")}
    emptyTitle={tr("No mempool")}
    {errorMessage}
    {errorStatus}
    {loading}
  >
    {#if summary != null}
      <HyListPanel
        fullHeight={!dockMode}
        subtitle={HyMpFormatTransactionCount(summary.count)}
        title={tr("Transactions")}
      >
        <HyMpTxTable
          hasMore={nextCursor != null}
          {loadingMore}
          onloadmore={loadMore}
          scrollMode={dockMode ? "page" : "self"}
          {transactions}
          ontxclick={(txid: string): void => {
            void goto("/transaction?hash=" + encodeURIComponent(txid));
          }}
        />
      </HyListPanel>
    {/if}
  </HyPageState>
</HyPageFrame>
