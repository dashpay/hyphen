import { bench, describe } from "vitest";

import { benchClient, unwrapBenchResult } from "./_shared.js";

describe("proposals", (): void => {
  bench("governanceSearch.listProposals", async (): Promise<void> => {
    unwrapBenchResult(
      "governanceSearch.listProposals",
      await benchClient.governanceSearch.listProposals({ limit: 100 })
    );
  });
});
