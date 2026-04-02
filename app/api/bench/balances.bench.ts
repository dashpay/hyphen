import { bench, describe } from "vitest";

import { benchClient, unwrapBenchResult } from "./_shared.js";

describe("balances", (): void => {
  bench("addresses.listBalances", async (): Promise<void> => {
    unwrapBenchResult("addresses.listBalances", await benchClient.addresses.listBalances({ limit: 100 }));
  });
});
