import { bench, describe } from "vitest";

import { benchClient, unwrapBenchResult } from "./_shared.js";

describe("status", (): void => {
  bench("status.getStatus", async (): Promise<void> => {
    unwrapBenchResult("status.getStatus", await benchClient.status.getStatus());
  });
});
