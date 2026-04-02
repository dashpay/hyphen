import { bench, describe } from "vitest";

import { benchClient, unwrapBenchResult } from "./_shared.js";

describe("mempool", (): void => {
  bench("live.getMempoolSummary", async (): Promise<void> => {
    unwrapBenchResult("live.getMempoolSummary", await benchClient.live.getMempoolSummary());
  });
});
