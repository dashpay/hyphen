import { describe, expect, it } from "vitest";

import { createClient, expectErr, expectOk } from "./support.js";

describe("governance and search routes", (): void => {
  it("lists proposals, reads one proposal, pages votes, and resolves all search discriminators", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/proposals?limit=1": {
        totalCount: 12,
        value: [
          {
            hash: "proposal-1",
            collateralHash: "collateral-1",
            name: "Treasury Proposal",
            url: "https://example.test/proposal",
            paymentAddress: "Xproposal",
            paymentAmount: 50,
            startTime: "2026-03-01T00:00:00Z",
            endTime: "2026-03-31T00:00:00Z",
            yesCount: 10,
            noCount: 1,
            abstainCount: 0,
            voteCount: 11,
            totalNodes: 4000,
            enabledNodes: 3900,
            participatingNodes: 3500,
            fundingThreshold: 2341,
            funded: true,
            valid: true,
            creationTime: "2026-02-28T00:00:00Z",
          },
        ],
      },
      "/api/v1/proposal/proposal-1": {
        hash: "proposal-1",
        collateralHash: "collateral-1",
        name: "Treasury Proposal",
        url: "https://example.test/proposal",
        paymentAddress: "Xproposal",
        paymentAmount: 50,
        startTime: "2026-03-01T00:00:00Z",
        endTime: "2026-03-31T00:00:00Z",
        yesCount: 10,
        noCount: 1,
        abstainCount: 0,
        voteCount: 11,
        totalNodes: 4000,
        enabledNodes: 3900,
        participatingNodes: 3500,
        fundingThreshold: 2341,
        funded: true,
        valid: true,
        creationTime: "2026-02-28T00:00:00Z",
      },
      "/api/v1/proposal/proposal-1/votes?limit=1": {
        totalCount: 200,
        value: [
          {
            proTxHash: "protx-voter-1",
            vote: "yes",
            voteTime: "2026-03-20T00:00:00Z",
          },
        ],
      },
      "/api/v1/search?q=block": {
        value: [
          {
            type: "block",
            id: "0000blocka",
            status: "canonical",
            blockTags: {
              orphan: false,
            },
          },
        ],
      },
      "/api/v1/search?q=tx": {
        value: [
          {
            type: "tx",
            id: "tx-1",
            txTags: {
              mined: true,
              coinbase: false,
            },
          },
        ],
      },
      "/api/v1/search?q=address": {
        value: [
          {
            type: "address",
            id: "Xabc",
          },
        ],
      },
      "/api/v1/search?q=validator": {
        value: [
          {
            type: "validator",
            id: "protx-1",
            status: "ENABLED",
          },
        ],
      },
      "/api/v1/search?q=proposal": {
        value: [
          {
            type: "proposal",
            id: "proposal-1",
          },
        ],
      },
    });

    const proposals = expectOk(await client.governanceSearch.listProposals({ limit: 1 }));
    const proposal = expectOk(await client.governanceSearch.getProposal("proposal-1"));
    const votes = expectOk(await client.governanceSearch.getProposalVotes("proposal-1", { limit: 1 }));
    const block = expectOk(await client.governanceSearch.search("block"));
    const tx = expectOk(await client.governanceSearch.search("tx"));
    const address = expectOk(await client.governanceSearch.search("address"));
    const validator = expectOk(await client.governanceSearch.search("validator"));
    const proposalSearch = expectOk(await client.governanceSearch.search("proposal"));

    expect(proposals.totalCount).toBe(12);
    expect(proposals.value[0]?.funded).toBe(true);
    expect(proposal.paymentAddress).toBe("Xproposal");
    expect(proposal.fundingThreshold).toBe(2341);
    expect(votes.value[0]?.vote).toBe("yes");
    expect(votes.totalCount).toBe(200);
    expect(block.value[0]?.type).toBe("block");
    expect(tx.value[0]?.type).toBe("tx");
    expect(address.value[0]?.type).toBe("address");
    expect(validator.value[0]?.type).toBe("validator");
    expect(proposalSearch.value[0]?.type).toBe("proposal");
  });

  it("returns a contract error for an unsupported search discriminator", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/search?q=bad": {
        value: [
          {
            type: "mystery",
            id: "bad",
          },
        ],
      },
    });

    const error = expectErr(await client.governanceSearch.search("bad"));
    expect(error.kind).toBe("contract");
    if (error.kind !== "contract") {
      throw new Error("expected contract error");
    }
    expect(error.path).toBe("$.value[0].type");
  });

  it("accepts sparse proposal pages and votes without timestamps", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/proposals": {
        totalCount: 0,
        value: [],
      },
      "/api/v1/proposal/proposal-2": {
        hash: "proposal-2",
        name: "Minimal Proposal",
        paymentAmount: 10,
        yesCount: 1,
        noCount: 0,
        abstainCount: 0,
        voteCount: 1,
        totalNodes: 4000,
        enabledNodes: 3900,
        participatingNodes: 1,
        fundingThreshold: 2341,
        funded: false,
        valid: true,
      },
      "/api/v1/proposal/proposal-2/votes": {
        totalCount: 1,
        value: [
          {
            proTxHash: "protx-voter-2",
            vote: "abstain",
          },
        ],
      },
    });

    const proposals = expectOk(await client.governanceSearch.listProposals());
    const proposal = expectOk(await client.governanceSearch.getProposal("proposal-2"));
    const votes = expectOk(await client.governanceSearch.getProposalVotes("proposal-2"));

    expect(proposals.value).toEqual([]);
    expect(proposals.totalCount).toBe(0);
    expect(proposals.nextCursor).toBeUndefined();
    expect(proposal.collateralHash).toBeUndefined();
    expect(proposal.url).toBeUndefined();
    expect(proposal.paymentAddress).toBeUndefined();
    expect(proposal.startTime).toBeUndefined();
    expect(proposal.endTime).toBeUndefined();
    expect(proposal.creationTime).toBeUndefined();
    expect(proposal.voteCount).toBe(1);
    expect(votes.value[0]?.voteTime).toBeUndefined();
    expect(votes.totalCount).toBe(1);
  });
});
