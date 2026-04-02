import { describe, expect, it } from "vitest";

import { createClient, expectOk } from "./support.js";

describe("metrics routes", (): void => {
  it("reads the latest metrics snapshot and one chart response", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/metrics/latest": {
        height: 2441849,
        txCount: 123,
        utxosSpendable: 10,
        utxosUnspendable: 2,
        blockchainSize: 1000000,
        creditPoolBalance: 18.5,
        circulatingSupply: 123456.78,
        difficulty: 1.234,
        hashrate: 987654.32,
      },
      "/api/v1/metrics/chart?blocks=144&points=2": {
        asOfHeight: 2441849,
        startHeight: 2441705,
        endHeight: 2441849,
        bucketBlocks: 72,
        value: [
          {
            height: 2441777,
            txCount: 110,
            utxosSpendable: 8,
            utxosUnspendable: 1,
            blockchainSize: 999000,
            circulatingSupply: 123400,
            difficulty: 1.2,
            hashrate: 950000,
          },
          {
            height: 2441849,
            txCount: 123,
            utxosSpendable: 10,
            utxosUnspendable: 2,
            blockchainSize: 1000000,
            creditPoolBalance: 18.5,
            circulatingSupply: 123456.78,
            difficulty: 1.234,
            hashrate: 987654.32,
          },
        ],
      },
    });

    const latest = expectOk(await client.metrics.getLatest());
    const chart = expectOk(await client.metrics.getChart({ blocks: 144, points: 2 }));

    expect(latest.creditPoolBalance).toBe(18.5);
    expect(chart.bucketBlocks).toBe(72);
    expect(chart.value[0]?.creditPoolBalance).toBeUndefined();
    expect(chart.value[1]?.creditPoolBalance).toBe(18.5);
  });

  it("accepts metrics snapshots without an optional credit pool balance", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/metrics/latest": {
        height: 2441849,
        txCount: 100,
        utxosSpendable: 12,
        utxosUnspendable: 3,
        blockchainSize: 1000,
        circulatingSupply: 99.5,
        difficulty: 2.5,
        hashrate: 500,
      },
      "/api/v1/metrics/chart": {
        asOfHeight: 2441849,
        startHeight: 2441849,
        endHeight: 2441849,
        bucketBlocks: 1,
        value: [
          {
            height: 2441849,
            txCount: 100,
            utxosSpendable: 12,
            utxosUnspendable: 3,
            blockchainSize: 1000,
            circulatingSupply: 99.5,
            difficulty: 2.5,
            hashrate: 500,
          },
        ],
      },
      "/api/v1/metrics/chart?blocks=24&points=1": {
        asOfHeight: 2441849,
        startHeight: 2441825,
        endHeight: 2441849,
        bucketBlocks: 24,
        value: [
          {
            height: 2441849,
            txCount: 100,
            utxosSpendable: 12,
            utxosUnspendable: 3,
            blockchainSize: 1000,
            circulatingSupply: 99.5,
            difficulty: 2.5,
            hashrate: 500,
          },
        ],
      },
    });

    const latest = expectOk(await client.metrics.getLatest());
    const defaultChart = expectOk(await client.metrics.getChart());
    const chart = expectOk(await client.metrics.getChart({ blocks: 24, points: 1 }));

    expect(latest.creditPoolBalance).toBeUndefined();
    expect(defaultChart.bucketBlocks).toBe(1);
    expect(chart.value[0]?.creditPoolBalance).toBeUndefined();
  });
});
