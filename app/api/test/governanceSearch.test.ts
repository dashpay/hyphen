import { describe, expect, it } from "vitest";

import { createClient, expectErr, expectOk } from "./support.js";

describe("governance and search routes", (): void => {
  it.each([
    [{ tags: { islock: true } }, true],
    [{}, undefined],
    [undefined, undefined],
  ])("reads transaction search lock state from live tags %j", async (live, islock): Promise<void> => {
    const client = createClient({
      "/api/v1/search?q=pending": {
        value: [
          {
            type: "tx",
            txHash: "pending-tx",
            txTags: { mined: false, coinbase: false },
            ...(live === undefined ? {} : { live }),
          },
        ],
      },
    });
    const page = expectOk(await client.governanceSearch.search("pending"));
    const result = page.value[0];
    if (result?.type !== "tx") {
      throw new Error("expected transaction result");
    }
    expect(result.live.tags?.islock).toBe(islock);
  });

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
            votes: { yes: 10, no: 1, abstain: 0 },
            fundingThreshold: 2341,
            meetsFundingThreshold: true,
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
        votes: { yes: 10, no: 1, abstain: 0 },
        fundingThreshold: 2341,
        meetsFundingThreshold: true,
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
      // Identifier keys below are the daemon's own: one per variant, named
      // after what it is. They are NOT `id` — the client normalizes to that.
      "/api/v1/search?q=block": {
        value: [
          {
            type: "block",
            blockHash: "0000blocka",
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
            txHash: "tx-1",
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
            address: "Xabc",
          },
        ],
      },
      "/api/v1/search?q=validator": {
        value: [
          {
            type: "validator",
            proTxHash: "protx-1",
            status: "ENABLED",
          },
        ],
      },
      "/api/v1/search?q=proposal": {
        value: [
          {
            type: "proposal",
            hash: "proposal-1",
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
    expect(proposal.votes.yes).toBe(10);
    expect(proposal.meetsFundingThreshold).toBe(true);
    expect(votes.value[0]?.vote).toBe("yes");
    expect(votes.totalCount).toBe(200);
    expect(block.value[0]?.type).toBe("block");
    expect(tx.value[0]?.type).toBe("tx");
    expect(address.value[0]?.type).toBe("address");
    expect(validator.value[0]?.type).toBe("validator");
    expect(proposalSearch.value[0]?.type).toBe("proposal");

    // The identifier itself, per variant. Asserting only `type` is what let a
    // parser that read every id from a key the daemon never sends pass green
    // while the search dropdown rendered nothing.
    expect(block.value[0]?.id).toBe("0000blocka");
    expect(tx.value[0]?.id).toBe("tx-1");
    expect(address.value[0]?.id).toBe("Xabc");
    expect(validator.value[0]?.id).toBe("protx-1");
    expect(proposalSearch.value[0]?.id).toBe("proposal-1");
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
        votes: { yes: 1, no: 0, abstain: 0 },
        fundingThreshold: 2341,
        meetsFundingThreshold: false,
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
    expect(proposal.votes.yes).toBe(1);
    expect(proposal.meetsFundingThreshold).toBe(false);
    expect(votes.value[0]?.voteTime).toBeUndefined();
    expect(votes.totalCount).toBe(1);
  });
});
