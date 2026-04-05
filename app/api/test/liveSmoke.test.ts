import { describe, expect, it } from "vitest";

import { ApCreateClient } from "../src/index.js";

const LIVE_URL = process.env["HYPHEN_API_LIVE_URL"];

describe("live smoke", (): void => {
  it("round-trips live endpoint discovery when a target url is provided", async (): Promise<void> => {
    if (LIVE_URL == null || LIVE_URL.length === 0) {
      return;
    }

    const client = ApCreateClient({
      baseUrl: LIVE_URL,
    });

    const statusResult = await client.status.getStatus();
    expect(statusResult.error()).toBeUndefined();

    const blocksResult = await client.blocks.list({ limit: 1 });
    const blocksPage = blocksResult.value();
    expect(blocksResult.error()).toBeUndefined();
    expect(blocksPage?.value.length).toBeGreaterThan(0);

    const blockHash = blocksPage?.value[0]?.blockHash;
    expect(blockHash).toBeDefined();
    if (blockHash == null) {
      throw new Error("expected one discovered block hash");
    }

    const blockResult = await client.blocks.get(blockHash);
    expect(blockResult.error()).toBeUndefined();

    const transactionsResult = await client.transactions.list({ limit: 1 });
    const transactionsPage = transactionsResult.value();
    expect(transactionsResult.error()).toBeUndefined();
    expect(transactionsPage?.value.length).toBeGreaterThan(0);

    const txid = transactionsPage?.value[0]?.txHash;
    expect(txid).toBeDefined();
    if (txid == null) {
      throw new Error("expected one discovered transaction id");
    }

    const transactionResult = await client.transactions.get(txid);
    expect(transactionResult.error()).toBeUndefined();
  }, 60_000);
});
