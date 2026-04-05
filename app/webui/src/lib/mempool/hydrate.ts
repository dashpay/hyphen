import type { ApiMempoolTxSummary } from "../api";
import { getTransaction } from "../api/client";

/** Hydrates pending mempool rows with detail-derived type fields. */
export async function hydrateMempoolTransactions(
  rows: readonly ApiMempoolTxSummary[]
): Promise<readonly ApiMempoolTxSummary[]> {
  return Promise.all(
    rows.map(async (row): Promise<ApiMempoolTxSummary> => {
      try {
        const detail = await getTransaction(row.txHash);
        if (detail == null) {
          return row;
        }

        return {
          ...row,
          txType: detail.txType,
        };
      } catch {
        return row;
      }
    })
  );
}
