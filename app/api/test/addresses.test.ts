import { describe, expect, it } from "vitest";

import { ApAvailabilityKind, ApCreateClient } from "../src/index.js";
import { createClient, createJsonResponse, expectErr, expectOk, loadJsonFixture } from "./support.js";

describe("address routes", (): void => {
  it("reads one address, pages its history and utxos, and ranks balances", async (): Promise<void> => {
    const balancesFixture = await loadJsonFixture("balances-top-level.json");
    const client = createClient({
      "/api/v1/address/Xabc": {
        asOfHeight: 2441849,
        address: "Xabc",
        balance: 12.5,
        totalReceived: 44.2,
        totalSent: 31.7,
        txCount: 3,
        firstSeen: "2026-03-01T00:00:00Z",
        lastSeen: "2026-03-20T00:00:00Z",
      },
      "/api/v1/address/Xabc/txs?limit=2": {
        asOfHeight: 2441849,
        value: [
          {
            txHash: "tx-address-1",
            blockTime: "2026-03-20T00:00:00Z",
            blockHeight: 2441848,
            blockPos: 2,
            direction: "in",
            txVersion: 3,
            netValue: 1.24,
            txType: "normal",
            txSize: 225,
            valueOut: 1.25,
            fee: 0.0001,
            tags: {
              mined: true,
              coinbase: false,
            },
          },
          {
            txHash: "tx-address-2",
            direction: "out",
            txVersion: 3,
            netValue: -0.11,
            txType: "data",
            txSize: 180,
            valueOut: 0.1,
            tags: {
              mined: false,
              coinbase: false,
            },
          },
        ],
        nextCursor: "cursor-address-next",
      },
      "/api/v1/address/Xabc/utxos?limit=2": {
        asOfHeight: 2441849,
        value: [
          {
            txid: "utxo-1",
            vout: 0,
            value: 5,
            height: 2441840,
            scriptPubKey: "76a914...",
          },
        ],
      },
      "/api/v1/balances?limit=1": balancesFixture,
    });

    const summary = expectOk(await client.addresses.get("Xabc"));
    const history = expectOk(await client.addresses.getTransactions("Xabc", { limit: 2 }));
    const utxos = expectOk(await client.addresses.getUtxos("Xabc", { limit: 2 }));
    const balances = expectOk(await client.addresses.listBalances({ limit: 1 }));

    expect(summary.firstSeen).toBe("2026-03-01T00:00:00Z");
    expect(summary.lastSeen).toBe("2026-03-20T00:00:00Z");
    expect(history.nextCursor).toBe("cursor-address-next");
    expect(history.value[0]?.blockTime).toBe("2026-03-20T00:00:00Z");
    expect(history.value[0]?.direction).toBe("in");
    expect(history.value[1]?.direction).toBe("out");
    expect(history.value[1]?.netValue).toBe(-0.11);
    expect(history.value[1]?.blockHeight).toBeUndefined();
    expect(history.value[1]?.tags.mined).toBe(false);
    expect(utxos.value[0]?.scriptPubKey).toBe("76a914...");
    expect(balances.includedSupply).toBe(200);
    expect(balances.totalCount).toBe(3000);
  });

  it("accepts the nested balances meta shape and optional missing cursors", async (): Promise<void> => {
    const balancesFixture = await loadJsonFixture("balances-meta.json");
    const client = createClient({
      "/api/v1/address/Xmeta/txs": {
        asOfHeight: 2441849,
        value: [],
      },
      "/api/v1/address/Xmeta/utxos": {
        asOfHeight: 2441849,
        value: [],
      },
      "/api/v1/balances": balancesFixture,
    });

    const history = expectOk(await client.addresses.getTransactions("Xmeta"));
    const utxos = expectOk(await client.addresses.getUtxos("Xmeta"));
    const balances = expectOk(await client.addresses.listBalances());

    expect(history.nextCursor).toBeUndefined();
    expect(utxos.nextCursor).toBeUndefined();
    expect(balances.includedSupply).toBe(200);
    expect(balances.supplyDefinition).toBe("nested balances snapshot");
    expect(balances.totalCount).toBe(3000);
    expect(balances.nextCursor).toBe("cursor-meta");
  });

  it("accepts sparse address summaries and empty cursor params", async (): Promise<void> => {
    const balancesFixture = await loadJsonFixture("balances-top-level.json");
    const client = createClient({
      "/api/v1/address/Xplain": {
        asOfHeight: 2441849,
        address: "Xplain",
        balance: 1,
        totalReceived: 2,
        totalSent: 1,
        txCount: 1,
      },
      "/api/v1/address/Xplain/txs": {
        asOfHeight: 2441849,
        value: [],
      },
      "/api/v1/address/Xplain/utxos": {
        asOfHeight: 2441849,
        value: [],
      },
      "/api/v1/balances": balancesFixture,
    });

    const summary = expectOk(await client.addresses.get("Xplain"));
    const history = expectOk(await client.addresses.getTransactions("Xplain", {}));
    const utxos = expectOk(await client.addresses.getUtxos("Xplain", {}));
    const balances = expectOk(await client.addresses.listBalances({}));

    expect(summary.firstSeen).toBeUndefined();
    expect(summary.lastSeen).toBeUndefined();
    expect(history.nextCursor).toBeUndefined();
    expect(utxos.nextCursor).toBeUndefined();
    expect(balances.totalCount).toBe(3000);
    expect(balances.nextCursor).toBe("cursor-top-level");
  });

  it("rejects unsupported address transaction directions", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/address/Xbad/txs": {
        asOfHeight: 2441849,
        totalCount: 1,
        value: [
          {
            txHash: "tx-bad-direction",
            direction: "mixed",
            txVersion: 3,
            netValue: 0,
            txType: "normal",
            txSize: 225,
            valueOut: 1.25,
            tags: {
              mined: true,
              coinbase: false,
            },
          },
        ],
      },
    });

    const error = expectErr(await client.addresses.getTransactions("Xbad"));
    expect(error.kind).toBe("contract");
    if (error.kind !== "contract") {
      throw new Error("expected contract error");
    }
    expect(error.path).toBe("$.value[0].direction");
  });

  it("classifies balances as indexed-not-ready when the address dataset is unavailable", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createJsonResponse(
            {
              error: {
                code: "service_unavailable",
                message: "balances not ready",
              },
            },
            { status: 503 }
          )
        ),
    });

    const error = expectErr(await client.addresses.listBalances());
    expect(error.kind).toBe("availability");
    if (error.kind !== "availability") {
      throw new Error("expected availability error");
    }
    expect(error.availabilityKind).toBe(ApAvailabilityKind.INDEX_NOT_READY);
    expect(error.descriptor.path).toBe("/api/v1/balances");
  });
});
