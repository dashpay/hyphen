import { beforeAll, bench, describe } from "vitest";

import { benchClient, getBenchDiscovery, type BenchDiscoveryContext, unwrapBenchResult } from "./_shared.js";

let discovery: BenchDiscoveryContext | null = null;

beforeAll(async (): Promise<void> => {
  discovery = await getBenchDiscovery();
}, 60_000);

describe("proposal votes", (): void => {
  bench("governanceSearch.getProposalVotes", async (): Promise<void> => {
    const current = discovery;

    if (current?.proposalHash == null) {
      throw new Error("governanceSearch.getProposalVotes requires at least one live proposal");
    }

    unwrapBenchResult(
      "governanceSearch.getProposalVotes",
      await benchClient.governanceSearch.getProposalVotes(current.proposalHash, { limit: 100 })
    );
  });
});
