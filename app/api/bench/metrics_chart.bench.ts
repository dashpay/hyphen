import { bench, describe } from "vitest";

import { benchClient, unwrapBenchResult } from "./_shared.js";

describe("metrics chart", (): void => {
  bench("metrics.getChart", async (): Promise<void> => {
    unwrapBenchResult("metrics.getChart", await benchClient.metrics.getChart({ blocks: 1440, points: 300 }));
  });
});
