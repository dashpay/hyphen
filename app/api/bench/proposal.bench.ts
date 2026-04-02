import { beforeAll, bench, describe } from "vitest";

import { benchClient, getBenchDiscovery, type BenchDiscoveryContext, unwrapBenchResult } from "./_shared.js";

let discovery: BenchDiscoveryContext | null = null;

beforeAll(async (): Promise<void> => {
  discovery = await getBenchDiscovery();
}, 60_000);

describe("proposal", (): void => {
  bench("governanceSearch.getProposal", async (): Promise<void> => {
    const current = discovery;

    if (current?.proposalHash == null) {
      throw new Error("governanceSearch.getProposal requires at least one live proposal");
    }

    unwrapBenchResult(
      "governanceSearch.getProposal",
      await benchClient.governanceSearch.getProposal(current.proposalHash)
    );
  });
});
