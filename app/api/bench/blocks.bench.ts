import { beforeAll, bench, describe } from "vitest";

import { benchClient, getBenchDiscovery, type BenchDiscoveryContext, unwrapBenchResult } from "./_shared.js";

let discovery: BenchDiscoveryContext | null = null;

beforeAll(async (): Promise<void> => {
  discovery = await getBenchDiscovery();
}, 60_000);

describe("blocks", (): void => {
  bench("blocks.list", async (): Promise<void> => {
    unwrapBenchResult("blocks.list", await benchClient.blocks.list({ limit: 25 }));
  });

  bench("blocks.get", async (): Promise<void> => {
    const current = discovery;

    if (current == null) {
      throw new Error("benchmark discovery did not run");
    }

    unwrapBenchResult("blocks.get", await benchClient.blocks.get(current.block.hash));
  });
});
