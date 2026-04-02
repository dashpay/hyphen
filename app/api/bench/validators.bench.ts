import { beforeAll, bench, describe } from "vitest";

import { benchClient, getBenchDiscovery, type BenchDiscoveryContext, unwrapBenchResult } from "./_shared.js";

let discovery: BenchDiscoveryContext | null = null;

beforeAll(async (): Promise<void> => {
  discovery = await getBenchDiscovery();
}, 60_000);

describe("validators", (): void => {
  bench("validators.list", async (): Promise<void> => {
    unwrapBenchResult("validators.list", await benchClient.validators.list({ limit: 100 }));
  });

  bench("validators.get", async (): Promise<void> => {
    const current = discovery;

    if (current == null) {
      throw new Error("benchmark discovery did not run");
    }

    unwrapBenchResult("validators.get", await benchClient.validators.get(current.validator.proTxHash));
  });
});
