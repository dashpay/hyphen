import { describe, expect, it } from "vitest";

import { createClient, expectErr, expectOk } from "./support.js";

describe("status and raw routes", (): void => {
  it("reads daemon status and raw payload responses", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/status": {
        syncStatus: "synced",
        blockHeight: 2441849,
        indices: {
          block: {
            indexedHeight: 2441849,
            updatedAt: "2026-03-21T00:00:00Z",
          },
          spent: {
            indexedHeight: 2441849,
            updatedAt: "2026-03-21T00:00:01Z",
          },
        },
        version: "0.1.0",
      },
      "/api/v1/rawblock/2441849": {
        asOfHeight: 2441849,
        source: "indexed",
        rawblock: "001122",
      },
      "/api/v1/rawtx/rawtx-1": {
        source: "live",
        rawtx: "aabbcc",
      },
    });

    const status = expectOk(await client.status.getStatus());
    const rawBlock = expectOk(await client.raw.getBlock(2441849));
    const rawTx = expectOk(await client.raw.getTx("rawtx-1"));

    expect(status.indices["block"]?.indexedHeight).toBe(2441849);
    expect(rawBlock.source).toBe("indexed");
    expect(rawTx.source).toBe("live");
    expect(rawTx.asOfHeight).toBeUndefined();
  });

  it("returns a contract error for an unsupported raw source discriminator", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/rawtx/rawtx-bad": {
        source: "mystery",
        rawtx: "aabbcc",
      },
    });

    const error = expectErr(await client.raw.getTx("rawtx-bad"));
    expect(error.kind).toBe("contract");
    if (error.kind !== "contract") {
      throw new Error("expected contract error");
    }
    expect(error.path).toBe("$.source");
  });
});
