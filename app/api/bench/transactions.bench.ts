import { beforeAll, bench, describe } from "vitest";

import { benchClient, getBenchDiscovery, type BenchDiscoveryContext, unwrapBenchResult } from "./_shared.js";

let discovery: BenchDiscoveryContext | null = null;

beforeAll(async (): Promise<void> => {
  discovery = await getBenchDiscovery();
}, 60_000);

describe("transactions", (): void => {
  bench("transactions.list", async (): Promise<void> => {
    unwrapBenchResult("transactions.list", await benchClient.transactions.list({ limit: 50 }));
  });

  bench("transactions.get", async (): Promise<void> => {
    const current = discovery;

    if (current == null) {
      throw new Error("benchmark discovery did not run");
    }

    unwrapBenchResult("transactions.get", await benchClient.transactions.get(current.transaction.txid));
  });
});
