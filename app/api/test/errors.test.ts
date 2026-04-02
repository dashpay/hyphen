import { describe, expect, it } from "vitest";

import {
  ApAvailabilityKind,
  ApCreateAvailabilityError,
  ApCreateHttpError,
  ApCreateNetworkError,
  ApReadinessModel,
  ApRouteClass,
} from "../src/index.js";

const DESCRIPTOR = {
  method: "GET" as const,
  name: "testRoute",
  path: "/test",
  readinessModel: ApReadinessModel.NONE,
  routeClass: ApRouteClass.RAW,
};

describe("error factories", (): void => {
  it("creates http and availability errors without optional bodies", (): void => {
    const httpError = ApCreateHttpError("missing", 404, DESCRIPTOR);
    const availabilityError = ApCreateAvailabilityError(
      "not ready",
      503,
      DESCRIPTOR,
      ApAvailabilityKind.INDEX_NOT_READY
    );

    expect(httpError.body).toBeUndefined();
    expect(availabilityError.body).toBeUndefined();
    expect(availabilityError.availabilityKind).toBe(ApAvailabilityKind.INDEX_NOT_READY);
  });

  it("creates network errors without a cause", (): void => {
    const networkError = ApCreateNetworkError("offline");

    expect(networkError.cause).toBeUndefined();
    expect(networkError.message).toBe("offline");
  });
});
