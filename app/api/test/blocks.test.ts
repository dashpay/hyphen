import { describe, expect, it } from "vitest";

import { createClient, expectOk } from "./support.js";

describe("block routes", (): void => {
  it("lists blocks, reads one block detail, and pages its transactions", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/blocks?limit=2": {
        asOfHeight: 2441849,
        value: [
          {
            blockHeight: 2441849,
            blockHash: "0000blocka",
            prevBlockHash: "0000prev",
            blockTime: "2026-03-21T00:00:00Z",
            bits: "1d00ffff",
            nonce: 42,
            blockVersion: 3,
            blockSize: 1024,
            txCount: 2,
            tags: {
              orphan: false,
            },
            live: {
              tags: {
                chainlock: true,
              },
              coreHeight: 2441850,
              seenAt: "2026-03-21T00:00:01Z",
            },
          },
          {
            blockHeight: 2441848,
            blockHash: "0000blockb",
            prevBlockHash: "0000prevb",
            blockTime: "2026-03-20T23:59:00Z",
            bits: "1d00ffff",
            nonce: 11,
            blockVersion: 3,
            blockSize: 2048,
            txCount: 1,
            tags: {},
            live: {},
          },
        ],
        nextCursor: "blocks-next",
      },
      "/api/v1/block/2441849": {
        asOfHeight: 2441849,
        blockHeight: 2441849,
        blockHash: "0000blocka",
        prevBlockHash: "0000prev",
        merkle: "abc123",
        blockTime: "2026-03-21T00:00:00Z",
        bits: "1d00ffff",
        nonce: 42,
        blockVersion: 3,
        blockSize: 1024,
        txCount: 2,
        status: "canonical",
        tags: {
          orphan: false,
        },
        live: {
          tags: {
            chainlock: true,
          },
          coreHeight: 2441850,
          seenAt: "2026-03-21T00:00:01Z",
        },
        chainwork: "ff00",
        cbTx: {
          height: 2441849,
          merkleRootMnList: "mnroot",
          merkleFlags: 1,
          bestClHeight: 2441848,
          bestClSignature: "deadbeef",
          creditPoolBalance: 17.5,
        },
      },
      "/api/v1/block/0000blocka/txs?limit=2": {
        asOfHeight: 2441849,
        value: [
          {
            txHash: "blocktx-1",
            blockHeight: 2441849,
            blockPos: 0,
            txVersion: 3,
            txType: "coinbase",
            lockTime: 0,
            txSize: 250,
            valueOut: 2.88,
            inputCount: 1,
            outputCount: 2,
            tags: {
              mined: true,
              coinbase: true,
            },
          },
          {
            txHash: "blocktx-2",
            blockHeight: 2441849,
            blockPos: 1,
            txVersion: 3,
            txType: "normal",
            lockTime: 0,
            txSize: 190,
            valueOut: 1.1,
            inputCount: 2,
            outputCount: 2,
            tags: {},
          },
        ],
      },
    });

    const blocks = expectOk(await client.blocks.list({ limit: 2 }));
    const detail = expectOk(await client.blocks.get(2441849));
    const transactions = expectOk(await client.blocks.getTransactions("0000blocka", { limit: 2 }));

    expect(blocks.nextCursor).toBe("blocks-next");
    expect(blocks.value[0]?.live.tags?.chainlock).toBe(true);
    expect(blocks.value[1]?.tags.orphan).toBe(false);
    expect(detail.cbTx?.creditPoolBalance).toBe(17.5);
    expect(detail.live.coreHeight).toBe(2441850);
    expect(transactions.value[0]?.tags.coinbase).toBe(true);
    expect(transactions.value[1]?.tags.coinbase).toBe(false);
  });

  it("accepts sparse block detail payloads without cbtx data", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/blocks": {
        asOfHeight: 2441849,
        value: [
          {
            blockHeight: 2441800,
            blockHash: "0000sparse",
            prevBlockHash: "0000prevsparse",
            blockTime: "2026-03-20T00:00:00Z",
            bits: "1d00ffff",
            nonce: 7,
            blockVersion: 3,
            blockSize: 900,
            txCount: 1,
            tags: {
              orphan: true,
            },
            live: {},
          },
        ],
      },
      "/api/v1/block/0000sparse": {
        asOfHeight: 2441849,
        blockHeight: 2441800,
        blockHash: "0000sparse",
        prevBlockHash: "0000prevsparse",
        merkle: "def456",
        blockTime: "2026-03-20T00:00:00Z",
        bits: "1d00ffff",
        nonce: 7,
        blockVersion: 3,
        blockSize: 900,
        txCount: 1,
        status: "canonical",
        tags: {},
        live: {},
        orphanedAt: "2026-03-20T00:00:01Z",
        chainwork: "aa55",
      },
      "/api/v1/block/0000sparse/txs": {
        asOfHeight: 2441849,
        value: [],
      },
    });

    const page = expectOk(await client.blocks.list());
    const detail = expectOk(await client.blocks.get("0000sparse"));
    const transactions = expectOk(await client.blocks.getTransactions("0000sparse"));

    expect(page.nextCursor).toBeUndefined();
    expect(page.value[0]?.tags.orphan).toBe(true);
    expect(detail.cbTx).toBeUndefined();
    expect(detail.orphanedAt).toBe("2026-03-20T00:00:01Z");
    expect(detail.chainwork).toBe("aa55");
    expect(detail.live.tags?.chainlock).toBeUndefined();
    expect(transactions.nextCursor).toBeUndefined();
  });

  it("accepts partially populated cbtx payloads and default block tx tags", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/block/0000partial": {
        asOfHeight: 2441849,
        blockHeight: 2441801,
        blockHash: "0000partial",
        prevBlockHash: "0000prevpartial",
        merkle: "ghi789",
        blockTime: "2026-03-20T00:01:00Z",
        bits: "1d00ffff",
        nonce: 9,
        blockVersion: 3,
        blockSize: 901,
        txCount: 1,
        status: "canonical",
        tags: {},
        live: {},
        cbTx: {
          height: 2441801,
          merkleRootMnList: "mn-only",
        },
      },
      "/api/v1/block/0000partial/txs": {
        asOfHeight: 2441849,
        value: [
          {
            txHash: "blocktx-partial",
            blockHeight: 2441801,
            blockPos: 0,
            txVersion: 3,
            txType: "normal",
            lockTime: 0,
            txSize: 190,
            valueOut: 0.5,
            inputCount: 1,
            outputCount: 1,
            tags: {},
          },
        ],
      },
    });

    const detail = expectOk(await client.blocks.get("0000partial"));
    const transactions = expectOk(await client.blocks.getTransactions("0000partial"));

    expect(detail.cbTx?.height).toBe(2441801);
    expect(detail.cbTx?.merkleRootMnList).toBe("mn-only");
    expect(detail.cbTx?.merkleFlags).toBeUndefined();
    expect(detail.cbTx?.bestClHeight).toBeUndefined();
    expect(detail.cbTx?.bestClSignature).toBeUndefined();
    expect(detail.cbTx?.creditPoolBalance).toBeUndefined();
    expect(transactions.value[0]?.tags.mined).toBe(false);
    expect(transactions.value[0]?.tags.coinbase).toBe(false);
  });
});
