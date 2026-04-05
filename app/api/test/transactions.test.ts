import { describe, expect, it } from "vitest";

import { createClient, createJsonResponse, expectErr, expectOk } from "./support.js";

describe("transaction routes", (): void => {
  it("lists indexed transactions and reads one transaction with live overlay tags", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/txs?limit=2": {
        asOfHeight: 2441849,
        value: [
          {
            txHash: "tx-list-1",
            blockHeight: 2441849,
            blockPos: 0,
            txVersion: 3,
            txType: "coinbase",
            lockTime: 0,
            txSize: 200,
            valueOut: 2.5,
            inputCount: 1,
            outputCount: 2,
            tags: {
              mined: true,
              coinbase: true,
            },
          },
          {
            txHash: "tx-list-2",
            blockHeight: 2441848,
            blockPos: 4,
            txVersion: 3,
            txType: "normal",
            lockTime: 3,
            txSize: 175,
            valueOut: 1.1,
            inputCount: 2,
            outputCount: 2,
            tags: {
              mined: true,
              coinbase: false,
            },
          },
        ],
        nextCursor: "tx-next",
      },
      "/api/v1/tx/tx-list-2": {
        asOfHeight: 2441849,
        txHash: "tx-list-2",
        blockHeight: 2441848,
        blockPos: 4,
        txVersion: 3,
        txType: "normal",
        lockTime: 3,
        txSize: 175,
        valueOut: 1.1,
        inputCount: 2,
        outputCount: 2,
        fee: 0.0002,
        tags: {
          mined: true,
          coinbase: false,
        },
        live: {
          tags: {
            mempool: false,
            islock: true,
          },
        },
        payload: {
          note: "special payload stays opaque",
        },
        vin: [
          {
            n: 0,
            scriptSig: {
              hex: "473044...",
            },
            sequence: 4294967295,
            address: "Xinput1",
            value: 0.7,
            txid: "prev-tx-1",
            vout: 1,
          },
          {
            n: 1,
            scriptSig: {
              hex: "473045...",
            },
            sequence: 4294967294,
            coinbase: "abcd",
          },
        ],
        vout: [
          {
            n: 0,
            value: 1,
            scriptPubKey: {
              hex: "76a914...",
            },
            address: "Xdest",
            spent: true,
          },
          {
            n: 1,
            value: 0.0998,
            scriptPubKey: {
              hex: "6a04deadbeef",
            },
            spent: false,
          },
        ],
      },
    });

    const page = expectOk(await client.transactions.list({ limit: 2 }));
    const detail = expectOk(await client.transactions.get("tx-list-2"));

    expect(page.nextCursor).toBe("tx-next");
    expect(page.value[0]?.tags.coinbase).toBe(true);
    expect(detail.live.tags?.islock).toBe(true);
    expect(detail.vin[0]?.address).toBe("Xinput1");
    expect(detail.vin[1]?.coinbase).toBe("abcd");
    expect(detail.vout[1]?.address).toBeUndefined();
    expect(detail.payload).toEqual({ note: "special payload stays opaque" });
  });

  it("returns an explicit 404 http error for a missing transaction lookup", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/tx/missing": createJsonResponse(
        {
          error: {
            code: "not_found",
            message: "transaction not found",
          },
        },
        { status: 404 }
      ),
    });

    const error = expectErr(await client.transactions.get("missing"));
    expect(error.kind).toBe("http");
    if (error.kind !== "http") {
      throw new Error("expected http error");
    }
    expect(error.status).toBe(404);
  });

  it("accepts sparse transaction detail shapes without payload or live tags", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/tx/sparse-tx": {
        asOfHeight: 2441849,
        txHash: "sparse-tx",
        txVersion: 3,
        txType: "provider_update_service",
        lockTime: 0,
        txSize: 160,
        valueOut: 0.5,
        inputCount: 1,
        outputCount: 1,
        tags: {
          mined: false,
          coinbase: false,
        },
        live: {},
        vin: [
          {
            n: 0,
            scriptSig: {
              hex: "",
            },
            sequence: 4294967295,
            txid: "prev-sparse",
            vout: 0,
          },
        ],
        vout: [
          {
            n: 0,
            value: 0.5,
            scriptPubKey: {
              hex: "76a914...",
            },
            address: "Xsparse",
            spent: false,
          },
        ],
      },
    });

    const detail = expectOk(await client.transactions.get("sparse-tx"));

    expect(detail.payload).toBeUndefined();
    expect(detail.live.tags?.mempool).toBeUndefined();
    expect(detail.vin[0]?.scriptSig.hex).toBe("");
    expect(detail.vin[0]?.coinbase).toBeUndefined();
  });

  it("treats empty transaction list cursor params as no query", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/txs": {
        asOfHeight: 2441849,
        value: [],
      },
    });

    const page = expectOk(await client.transactions.list({}));

    expect(page.value).toEqual([]);
    expect(page.nextCursor).toBeUndefined();
  });
});
