import { bench, describe } from "vitest";

import { benchClient, unwrapBenchResult } from "./_shared.js";

describe("network", (): void => {
  bench("live.getNetworkSnapshot", async (): Promise<void> => {
    unwrapBenchResult("live.getNetworkSnapshot", await benchClient.live.getNetworkSnapshot());
  });
});
