import { beforeAll, bench, describe } from "vitest";

import { benchClient, getBenchDiscovery, type BenchDiscoveryContext, unwrapBenchResult } from "./_shared.js";

let discovery: BenchDiscoveryContext | null = null;

beforeAll(async (): Promise<void> => {
  discovery = await getBenchDiscovery();
}, 60_000);

describe("addresses", (): void => {
  bench("addresses.get", async (): Promise<void> => {
    const current = discovery;

    if (current == null) {
      throw new Error("benchmark discovery did not run");
    }

    unwrapBenchResult("addresses.get", await benchClient.addresses.get(current.address));
  });
});
