import { beforeAll, bench, describe } from "vitest";

import { benchClient, getBenchDiscovery, type BenchDiscoveryContext, unwrapBenchResult } from "./_shared.js";

let discovery: BenchDiscoveryContext | null = null;

beforeAll(async (): Promise<void> => {
  discovery = await getBenchDiscovery();
}, 60_000);

describe("raw block", (): void => {
  bench("raw.getBlock", async (): Promise<void> => {
    const current = discovery;

    if (current == null) {
      throw new Error("benchmark discovery did not run");
    }

    unwrapBenchResult("raw.getBlock", await benchClient.raw.getBlock(current.block.hash));
  });
});
