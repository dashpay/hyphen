import { describe, expect, it } from "vitest";

import { ApCreateContractError, ApReadinessModel, ApRouteClass, type ApRouteDescriptor } from "../src/index.js";
import { createClientContext, requestJsonResult, requestResponseResult } from "../src/internal/transport.js";
import { createJsonResponse, createTextResponse } from "./support.js";

const INDEXED_ROUTE: ApRouteDescriptor = {
  method: "GET",
  name: "indexedRoute",
  path: "/indexed",
  readinessModel: ApReadinessModel.INDEXED,
  routeClass: ApRouteClass.INDEXED,
};

const LIVE_ROUTE: ApRouteDescriptor = {
  method: "GET",
  name: "liveRoute",
  path: "/live",
  readinessModel: ApReadinessModel.LIVE,
  routeClass: ApRouteClass.LIVE,
};

const NONE_ROUTE: ApRouteDescriptor = {
  method: "GET",
  name: "noneRoute",
  path: "/none",
  readinessModel: ApReadinessModel.NONE,
  routeClass: ApRouteClass.RAW,
};

describe("transport classification", (): void => {
  it("classifies 501 responses as capability-disabled", async (): Promise<void> => {
    const context = createClientContext({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createJsonResponse(
            {
              error: {
                code: "not_implemented",
                message: "disabled",
              },
            },
            { status: 501 }
          )
        ),
    });

    const result = await requestResponseResult(context, LIVE_ROUTE, LIVE_ROUTE.path);
    const error = result.error();

    expect(error?.kind).toBe("availability");
    if (error?.kind !== "availability") {
      throw new Error("expected availability error");
    }
    expect(error.availabilityKind).toBe("capabilityDisabled");
  });

  it("classifies indexed and live 503 responses separately", async (): Promise<void> => {
    const response = createJsonResponse(
      {
        error: {
          code: "service_unavailable",
          message: "not ready",
        },
      },
      { status: 503 }
    );

    const indexedContext = createClientContext({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> => Promise.resolve(response.clone()),
    });
    const liveContext = createClientContext({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> => Promise.resolve(response.clone()),
    });

    const indexedError = (await requestResponseResult(indexedContext, INDEXED_ROUTE, INDEXED_ROUTE.path)).error();
    const liveError = (await requestResponseResult(liveContext, LIVE_ROUTE, LIVE_ROUTE.path)).error();

    expect(indexedError?.kind).toBe("availability");
    expect(liveError?.kind).toBe("availability");
    if (indexedError?.kind !== "availability" || liveError?.kind !== "availability") {
      throw new Error("expected availability errors");
    }
    expect(indexedError.availabilityKind).toBe("indexNotReady");
    expect(liveError.availabilityKind).toBe("liveNotReady");
  });

  it("keeps 503 responses plain http errors for routes without readiness modeling", async (): Promise<void> => {
    const context = createClientContext({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createJsonResponse(
            {
              error: {
                code: "service_unavailable",
                message: "not ready",
              },
            },
            { status: 503 }
          )
        ),
    });

    const error = (await requestResponseResult(context, NONE_ROUTE, NONE_ROUTE.path)).error();

    expect(error?.kind).toBe("http");
  });

  it("uses the fallback status message when an http error body is not json", async (): Promise<void> => {
    const context = createClientContext({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createTextResponse("not json", {
            status: 502,
          })
        ),
    });

    const error = (await requestResponseResult(context, NONE_ROUTE, NONE_ROUTE.path)).error();

    expect(error?.kind).toBe("http");
    expect(error?.message).toBe("HTTP 502");
  });

  it("returns network errors for pre-response failures and malformed bodies", async (): Promise<void> => {
    const preResponseContext = createClientContext({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> => Promise.reject(new Error("boom")),
    });
    const brokenJsonContext = createClientContext({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          new Response("{", {
            headers: {
              "content-type": "application/json",
            },
          })
        ),
    });

    const preResponse = await requestResponseResult(preResponseContext, LIVE_ROUTE, LIVE_ROUTE.path);
    const brokenJson = await requestJsonResult(brokenJsonContext, LIVE_ROUTE, LIVE_ROUTE.path, (): number => 1);

    expect(preResponse.error()?.kind).toBe("network");
    expect(brokenJson.error()?.kind).toBe("network");
  });

  it("returns contract errors for wrong content types and parser contract failures", async (): Promise<void> => {
    const wrongTypeContext = createClientContext({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> => Promise.resolve(createTextResponse("hello")),
    });
    const parseFailContext = createClientContext({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> => Promise.resolve(createJsonResponse({ ok: true })),
    });

    const wrongType = await requestJsonResult(wrongTypeContext, LIVE_ROUTE, LIVE_ROUTE.path, (): number => 1);
    const parseFailure = await requestJsonResult(parseFailContext, LIVE_ROUTE, LIVE_ROUTE.path, (): number => {
      throw new Error(JSON.stringify(ApCreateContractError("$.payload", "bad payload")));
    });

    expect(wrongType.error()?.kind).toBe("contract");
    expect(parseFailure.error()?.kind).toBe("network");
  });
});
