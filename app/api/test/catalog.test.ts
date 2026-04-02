import { describe, expect, it } from "vitest";

import {
  ApAddressRouteDescriptors,
  ApBlockRouteDescriptors,
  ApGovernanceSearchRouteDescriptors,
  ApLiveRouteDescriptors,
  ApMetricsRouteDescriptors,
  ApRoutes,
  ApStatusRawRouteDescriptors,
  ApStreamRouteDescriptors,
  ApTransactionRouteDescriptors,
  ApValidatorRouteDescriptors,
} from "../src/index.js";

describe("route catalog", (): void => {
  it("exposes each descriptor group through the public catalog", (): void => {
    expect(ApRoutes.addresses).toBe(ApAddressRouteDescriptors);
    expect(ApRoutes.blocks).toBe(ApBlockRouteDescriptors);
    expect(ApRoutes.governanceSearch).toBe(ApGovernanceSearchRouteDescriptors);
    expect(ApRoutes.live).toBe(ApLiveRouteDescriptors);
    expect(ApRoutes.metrics).toBe(ApMetricsRouteDescriptors);
    expect(ApRoutes.statusRaw).toBe(ApStatusRawRouteDescriptors);
    expect(ApRoutes.stream).toBe(ApStreamRouteDescriptors);
    expect(ApRoutes.transactions).toBe(ApTransactionRouteDescriptors);
    expect(ApRoutes.validators).toBe(ApValidatorRouteDescriptors);
  });

  it("keeps descriptor names aligned with their public catalog keys", (): void => {
    expect(ApRoutes.addresses["detail"]?.name).toBe("address");
    expect(ApRoutes.addresses["transactions"]?.name).toBe("addressTransactions");
    expect(ApRoutes.addresses["utxos"]?.name).toBe("addressUtxos");
    expect(ApRoutes.addresses["balances"]?.name).toBe("balances");
    expect(ApRoutes.blocks["detail"]?.name).toBe("block");
    expect(ApRoutes.blocks["transactions"]?.name).toBe("blockTransactions");
    expect(ApRoutes.transactions["detail"]?.name).toBe("transaction");
    expect(ApRoutes.validators["detail"]?.name).toBe("validator");
    expect(ApRoutes.governanceSearch["search"]?.name).toBe("search");
    expect(ApRoutes.stream["events"]?.name).toBe("stream");
  });
});
