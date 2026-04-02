import { describe, expect, it } from "vitest";

import { ApAvailabilityKind, ApCreateClient } from "../src/index.js";
import { createClient, createJsonResponse, expectErr, expectOk, loadJsonFixture } from "./support.js";

describe("live routes", (): void => {
  it("reads the mempool summary, pages mempool transactions, and normalizes network aliases", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/mempool": {
        count: 2,
        totalBytes: 500,
        usageBytes: 600,
        maxBytes: 1000,
        updatedAt: "2026-03-21T00:00:00Z",
        recentHashes: ["mempool-a", "mempool-b"],
      },
      "/api/v1/mempool/txs?limit=2": {
        value: [
          {
            txid: "mempool-a",
            size: 225,
            valueOut: 123_456,
            firstSeenAt: "2026-03-21T00:00:01Z",
            lastSeenAt: "2026-03-21T00:00:02Z",
            tags: {
              mined: false,
              coinbase: false,
            },
          },
        ],
        nextCursor: "mempool-next",
      },
      "/api/v1/network": {
        networkInfo: {
          version: 220000,
          subversion: "/dashcore:22.0.0/",
          protocolversion: 70231,
          timeoffset: 1,
          connections: 14,
          relayfee: 0.00001,
          warnings: "",
        },
        peerInfo: [
          {
            id: 1,
            addr: "127.0.0.1:9999",
            addrlocal: "10.0.0.5:9999",
            subver: "/dashcore:22.0.0/",
            inbound: false,
            startingheight: 2441800,
            syncedHeaders: 2441849,
            syncedBlocks: 2441849,
            connectionType: "outbound-full-relay",
          },
        ],
        peerCount: 1,
        updatedAt: "2026-03-21T00:00:03Z",
      },
    });

    const summary = expectOk(await client.live.getMempoolSummary());
    const page = expectOk(await client.live.listMempoolTransactions({ limit: 2 }));
    const network = expectOk(await client.live.getNetworkSnapshot());

    expect(summary.maxBytes).toBe(1000);
    expect(page.nextCursor).toBe("mempool-next");
    expect(page.value[0]?.valueOut).toBe(123_456);
    expect(page.value[0]?.tags.mined).toBe(false);
    expect(network.networkInfo.protocolVersion).toBe(70231);
    expect(network.peerInfo[0]?.addrLocal).toBe("10.0.0.5:9999");
    expect(network.peerInfo[0]?.startingHeight).toBe(2441800);
  });

  it("classifies a disabled live capability as capability-unavailable", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createJsonResponse(
            {
              error: {
                code: "not_implemented",
                message: "network disabled",
              },
            },
            { status: 501 }
          )
        ),
    });

    const error = expectErr(await client.live.getNetworkSnapshot());
    expect(error.kind).toBe("availability");
    if (error.kind !== "availability") {
      throw new Error("expected availability error");
    }
    expect(error.availabilityKind).toBe(ApAvailabilityKind.CAPABILITY_DISABLED);
  });

  it("accepts daemon-style alias fixtures and sparse live pages", async (): Promise<void> => {
    const networkFixture = await loadJsonFixture("network-aliases.json");
    const client = createClient({
      "/api/v1/mempool/txs": {
        value: [],
      },
      "/api/v1/network": networkFixture,
    });

    const page = expectOk(await client.live.listMempoolTransactions());
    const network = expectOk(await client.live.getNetworkSnapshot());

    expect(page.nextCursor).toBeUndefined();
    expect(network.networkInfo.relayFee).toBe(0.00001);
    expect(network.peerInfo[0]?.subversion).toBe("/dashcore:22.0.0/");
  });

  it("accepts normalized live field names and absent optional live fields", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/mempool": {
        count: 0,
        totalBytes: 0,
        updatedAt: "2026-03-21T00:00:00Z",
        recentHashes: [],
      },
      "/api/v1/network": {
        networkInfo: {
          version: 230000,
          protocolVersion: 70231,
          timeOffset: 0,
          connections: 0,
        },
        peerInfo: [
          {
            id: 8,
            addr: "192.0.2.11:9999",
            addrLocal: "10.0.0.6:9999",
            subversion: "/dashcore:23.0.0/",
            inbound: true,
            startingHeight: 2441849,
            syncedHeaders: 2441849,
            syncedBlocks: 2441849,
          },
        ],
        peerCount: 1,
        updatedAt: "2026-03-21T00:00:04Z",
      },
    });

    const summary = expectOk(await client.live.getMempoolSummary());
    const network = expectOk(await client.live.getNetworkSnapshot());

    expect(summary.usageBytes).toBeUndefined();
    expect(summary.maxBytes).toBeUndefined();
    expect(network.networkInfo.subversion).toBeUndefined();
    expect(network.networkInfo.protocolVersion).toBe(70231);
    expect(network.peerInfo[0]?.inbound).toBe(true);
    expect(network.peerInfo[0]?.connectionType).toBeUndefined();
  });

  it("treats empty live cursor params the same as no query params", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/mempool/txs": {
        value: [],
      },
    });

    const page = expectOk(await client.live.listMempoolTransactions({}));

    expect(page.value).toEqual([]);
    expect(page.nextCursor).toBeUndefined();
  });

  it("accepts network snapshots with only required top-level fields", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/network": {
        networkInfo: {},
        peerInfo: [{}],
        peerCount: 1,
        updatedAt: "2026-03-21T00:00:05Z",
      },
    });

    const network = expectOk(await client.live.getNetworkSnapshot());

    expect(network.networkInfo.version).toBeUndefined();
    expect(network.networkInfo.protocolVersion).toBeUndefined();
    expect(network.networkInfo.timeOffset).toBeUndefined();
    expect(network.networkInfo.connections).toBeUndefined();
    expect(network.networkInfo.relayFee).toBeUndefined();
    expect(network.networkInfo.warnings).toBeUndefined();
    expect(network.peerInfo[0]?.id).toBeUndefined();
    expect(network.peerInfo[0]?.addr).toBeUndefined();
    expect(network.peerInfo[0]?.addrLocal).toBeUndefined();
    expect(network.peerInfo[0]?.subversion).toBeUndefined();
    expect(network.peerInfo[0]?.inbound).toBeUndefined();
    expect(network.peerInfo[0]?.startingHeight).toBeUndefined();
    expect(network.peerInfo[0]?.syncedHeaders).toBeUndefined();
    expect(network.peerInfo[0]?.syncedBlocks).toBeUndefined();
    expect(network.peerInfo[0]?.connectionType).toBeUndefined();
  });
});
