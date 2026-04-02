<script lang="ts">
  /** Block transaction panel for expandable transaction rows within one block. */
  import { tr } from "@repo/hyphen-i18n";
  import { VwText } from "@repo/hyphen-views";

  import type { ApiBlockDetail, ApiTxSummary } from "$lib";
  import { getBlockTxs } from "$lib";
  import { HyUtilFormatFullDate, HyUtilFormatNumber, HyUtilReadErrorMessage } from "$lib/HyUtil";
  import { getTransactionDisplay } from "$lib/tx/display";
  import type { HyTxExpandableRow } from "$lib/tx/HyTxUtil";
  import { HyDetailSection, HyListPanel, HyScrollBody } from "$lib/views";
  import { HyTxExpandablePanel } from "$lib/tx/views";

  interface Props {
    readonly block: ApiBlockDetail;
    readonly fullHeight?: boolean;
    readonly ontxclick?: (txid: string) => void;
    readonly scrollMode?: "page" | "self";
  }

  const { block, fullHeight = true, ontxclick, scrollMode = "self" }: Props = $props();

  let transactions = $state<readonly ApiTxSummary[]>([]);
  let transactionsError = $state<string | null>(null);
  let nextCursor = $state<string | null>(null);
  let loadingMore = $state(false);

  const rows = $derived.by((): readonly HyTxExpandableRow[] => {
    return transactions.map((transaction): HyTxExpandableRow => {
      const txDisplay = getTransactionDisplay(transaction.txType, transaction.blockPos);

      return {
        label: txDisplay.label,
        metaLabel: HyUtilFormatFullDate(block.time),
        metaValue: "#" + HyUtilFormatNumber(block.height),
        size: transaction.size,
        tone: txDisplay.tone,
        txid: transaction.txid,
        valueOut: transaction.valueOut,
      };
    });
  });

  $effect((): void => {
    transactions = [];
    transactionsError = null;

    void getBlockTxs(block.hash, 100)
      .then((response): void => {
        transactions = response.value;
        nextCursor = response.nextCursor ?? null;
      })
      .catch((caught: unknown): void => {
        transactions = [];
        transactionsError = HyUtilReadErrorMessage(caught);
        nextCursor = null;
      });
  });

  async function loadMore(): Promise<void> {
    if (loadingMore || nextCursor == null) {
      return;
    }

    loadingMore = true;

    try {
      const response = await getBlockTxs(block.hash, 100, nextCursor);
      transactions = [...transactions, ...response.value];
      nextCursor = response.nextCursor ?? null;
    } catch {
      nextCursor = null;
    } finally {
      loadingMore = false;
    }
  }
</script>

{#if transactionsError != null}
  <div class="tx-panel">
    <HyListPanel
      {fullHeight}
      subtitle={tr("{count, plural, one {# transaction} other {# transactions}}", { count: block.txCount })}
      title={tr("Transactions")}
    >
      <HyScrollBody {scrollMode}>
        <HyDetailSection>
          <VwText color="muted" size="sm">{transactionsError}</VwText>
        </HyDetailSection>
      </HyScrollBody>
    </HyListPanel>
  </div>
{:else}
  <HyTxExpandablePanel
    {fullHeight}
    hasMore={nextCursor != null}
    ioAmountLayout="stacked"
    {loadingMore}
    onloadmore={loadMore}
    {ontxclick}
    {rows}
    {scrollMode}
    subtitle={tr("{count, plural, one {# transaction} other {# transactions}}", { count: block.txCount })}
    title={tr("Transactions")}
  />
{/if}

<style>
  .tx-panel {
    height: 100%;
    min-width: 0;
  }
</style>
