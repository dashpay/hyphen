<script lang="ts">
  /** Address transaction panel for summary rows tied to one address. */
  import { tr } from "@repo/hyphen-i18n";
  import type { ApiAddressTxSummary } from "$lib";
  import { HyUtilFormatFullDate, HyUtilFormatNumber } from "$lib/HyUtil";
  import { HyAdFormatTransactionCount } from "$lib/address/HyAdUtil";
  import type { HyTxExpandableRow } from "$lib/tx/HyTxUtil";
  import type { TransactionTone } from "$lib/tx/display";
  import { HyTxExpandablePanel } from "$lib/tx/views";

  interface AddressTransactionRow extends ApiAddressTxSummary {
    readonly label: string;
    readonly tone: TransactionTone;
  }

  interface Props {
    readonly fullHeight?: boolean;
    readonly hasMore?: boolean;
    readonly loadingMore?: boolean;
    readonly onloadmore?: (() => unknown) | undefined;
    readonly scrollMode?: "page" | "self";
    readonly totalCount: number;
    readonly transactions: readonly AddressTransactionRow[];
    readonly ontxclick?: (txHash: string) => void;
  }

  const {
    fullHeight = true,
    hasMore = false,
    loadingMore = false,
    onloadmore,
    scrollMode = "self",
    totalCount,
    transactions,
    ontxclick,
  }: Props = $props();

  function formatMetaLabel(transaction: AddressTransactionRow): string | null {
    return transaction.blockTime != null ? HyUtilFormatFullDate(transaction.blockTime) : null;
  }

  function getAmountPrefix(transaction: AddressTransactionRow): string | undefined {
    const direction = transaction.direction.toLowerCase();

    if (direction === "in" || direction === "incoming") {
      return "+";
    }

    if (direction === "out" || direction === "outgoing") {
      return "-";
    }

    return undefined;
  }

  function getAmountTone(transaction: AddressTransactionRow): "default" | "negative" | "positive" {
    const direction = transaction.direction.toLowerCase();

    if (direction === "in" || direction === "incoming") {
      return "positive";
    }

    if (direction === "out" || direction === "outgoing") {
      return "negative";
    }

    return "default";
  }

  const rows = $derived.by((): readonly HyTxExpandableRow[] => {
    return transactions.map((transaction): HyTxExpandableRow => {
      const amountPrefix = getAmountPrefix(transaction);

      return {
        ...(amountPrefix == null ? {} : { amountPrefix }),
        amountTone: getAmountTone(transaction),
        label: transaction.label,
        metaLabel: formatMetaLabel(transaction),
        metaValue: transaction.blockHeight != null ? "#" + HyUtilFormatNumber(transaction.blockHeight) : "Mempool",
        size: transaction.txSize,
        tone: transaction.tone,
        txid: transaction.txHash,
        valueOut: Math.abs(transaction.netValue),
      };
    });
  });
</script>

<HyTxExpandablePanel
  {fullHeight}
  {hasMore}
  hidePayloadForTypes={["coinbase"]}
  {loadingMore}
  ioAmountLayout="stacked"
  {onloadmore}
  {ontxclick}
  {rows}
  {scrollMode}
  subtitle={HyAdFormatTransactionCount(totalCount)}
  title={tr("Transactions")}
/>
