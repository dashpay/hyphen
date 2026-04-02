import { describe, expect, it } from "vitest";

import { createClient, expectOk } from "./support.js";

describe("validator routes", (): void => {
  it("lists validators with filters and reads validator detail with live overlay and history", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/validators?cursor=cursor-1&limit=1&status=POSE_BANNED&type=evonode": {
        asOfHeight: 2441849,
        totalCount: 4200,
        value: [
          {
            proTxHash: "protx-1",
            collateralHash: "collateral-1",
            collateralIndex: 1,
            addresses: {
              collateral: "Xcollateral",
              owner: "Xowner",
              voting: "Xvoting",
              payout: "Xpayout",
            },
            live: {
              enabled: false,
              currentStatus: "POSE_BANNED",
              posePenalty: 11,
              poseBanHeight: 2441840,
            },
            operatorPubKey: "operator-key",
            operatorReward: 5,
            type: "evonode",
            status: "POSE_BANNED",
            services: {
              coreP2p: ["127.0.0.1:9999"],
              platformP2p: ["/ip4/127.0.0.1/tcp/26656"],
              platformHttps: ["https://example.test"],
            },
            registeredHeight: 2400000,
            lastUpdatedHeight: 2441848,
          },
        ],
        nextCursor: "validator-next",
      },
      "/api/v1/validator/protx-1": {
        asOfHeight: 2441849,
        proTxHash: "protx-1",
        collateralHash: "collateral-1",
        collateralIndex: 1,
        addresses: {
          collateral: "Xcollateral",
          owner: "Xowner",
          voting: "Xvoting",
          payout: "Xpayout",
        },
        live: {
          enabled: false,
          currentStatus: "POSE_BANNED",
          lastPaidAt: "2026-03-20T00:00:00Z",
          lastPaidHeight: 2441800,
          consecutivePayments: 3,
          posePenalty: 11,
          poseRevivedHeight: 2441830,
          poseBanHeight: 2441840,
          revocationReason: 2,
        },
        operatorPubKey: "operator-key",
        operatorReward: 5,
        type: "evonode",
        status: "POSE_BANNED",
        services: {
          coreP2p: ["127.0.0.1:9999"],
          platformP2p: ["/ip4/127.0.0.1/tcp/26656"],
          platformHttps: ["https://example.test"],
        },
        registeredHeight: 2400000,
        lastUpdatedHeight: 2441848,
        history: [
          {
            txid: "proregtx-1",
            txType: "provider_register",
            payload: {
              ownerAddress: "Xowner",
            },
            blockHeight: 2400000,
          },
          {
            txid: "proupservtx-1",
            txType: "provider_update_service",
            payload: {
              service: "127.0.0.1:9999",
            },
            blockHeight: 2441848,
          },
        ],
      },
    });

    const page = expectOk(
      await client.validators.list({
        cursor: "cursor-1",
        limit: 1,
        status: "POSE_BANNED",
        type: "evonode",
      })
    );
    const detail = expectOk(await client.validators.get("protx-1"));

    expect(page.nextCursor).toBe("validator-next");
    expect(page.totalCount).toBe(4200);
    expect(page.value[0]?.services.platformHttps[0]).toBe("https://example.test");
    expect(page.value[0]?.live.enabled).toBe(false);
    expect(page.value[0]?.live.currentStatus).toBe("POSE_BANNED");
    expect(detail.live.lastPaidAt).toBe("2026-03-20T00:00:00Z");
    expect(detail.live.poseBanHeight).toBe(2441840);
    expect(detail.live.revocationReason).toBe(2);
    expect(detail.history[1]?.txType).toBe("provider_update_service");
  });

  it("accepts validators without current status overlays or platform services", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/validators?limit=1": {
        asOfHeight: 2441849,
        totalCount: 1,
        value: [
          {
            proTxHash: "protx-plain",
            collateralHash: "collateral-plain",
            collateralIndex: 0,
            addresses: {
              payout: "Xpayout",
            },
            live: {
              enabled: true,
              posePenalty: 0,
            },
            operatorPubKey: "operator-key",
            operatorReward: 0,
            type: "regular",
            status: "ENABLED",
            services: {
              coreP2p: ["127.0.0.1:9999"],
              platformP2p: [],
              platformHttps: [],
            },
            registeredHeight: 2400001,
            lastUpdatedHeight: 2441849,
          },
        ],
      },
      "/api/v1/validator/protx-plain": {
        asOfHeight: 2441849,
        proTxHash: "protx-plain",
        collateralHash: "collateral-plain",
        collateralIndex: 0,
        addresses: {
          payout: "Xpayout",
        },
        live: {},
        operatorPubKey: "operator-key",
        operatorReward: 0,
        type: "regular",
        status: "ENABLED",
        services: {
          coreP2p: ["127.0.0.1:9999"],
          platformP2p: [],
          platformHttps: [],
        },
        registeredHeight: 2400001,
        lastUpdatedHeight: 2441849,
        history: [],
      },
    });

    const page = expectOk(await client.validators.list({ limit: 1 }));
    const detail = expectOk(await client.validators.get("protx-plain"));

    expect(page.value[0]?.live.currentStatus).toBeUndefined();
    expect(page.value[0]?.live.enabled).toBe(true);
    expect(page.totalCount).toBe(1);
    expect(page.value[0]?.services.platformP2p).toEqual([]);
    expect(page.value[0]?.addresses.owner).toBeUndefined();
    expect(detail.live.currentStatus).toBeUndefined();
    expect(detail.history).toEqual([]);
  });

  it("treats empty validator list params as no query", async (): Promise<void> => {
    const client = createClient({
      "/api/v1/validators": {
        asOfHeight: 2441849,
        totalCount: 0,
        value: [],
      },
    });

    const page = expectOk(await client.validators.list({}));

    expect(page.value).toEqual([]);
    expect(page.totalCount).toBe(0);
    expect(page.nextCursor).toBeUndefined();
  });
});
