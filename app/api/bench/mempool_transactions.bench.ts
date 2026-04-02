import { bench, describe } from "vitest";

import { benchClient, unwrapBenchResult } from "./_shared.js";

describe("mempool transactions", (): void => {
  bench("live.listMempoolTransactions", async (): Promise<void> => {
    unwrapBenchResult("live.listMempoolTransactions", await benchClient.live.listMempoolTransactions({ limit: 50 }));
  });
});
