import { bench, describe } from "vitest";

import { benchClient, unwrapBenchResult } from "./_shared.js";

describe("metrics latest", (): void => {
  bench("metrics.getLatest", async (): Promise<void> => {
    unwrapBenchResult("metrics.getLatest", await benchClient.metrics.getLatest());
  });
});
