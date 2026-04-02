import { type ApCreateClient } from "../../src/index.js";
import { ApCreateClient as createClientImpl } from "../../src/index.js";

export interface JsonFixtureMap {
  readonly [path: string]: unknown;
}

export function createJsonResponse(body: unknown, init?: ResponseInit): Response {
  return new Response(JSON.stringify(body), {
    headers: {
      "content-type": "application/json",
    },
    status: 200,
    ...init,
  });
}

export function createNdjsonResponse(lines: readonly string[], init?: ResponseInit): Response {
  return new Response(lines.join("\n"), {
    headers: {
      "content-type": "application/x-ndjson",
    },
    status: 200,
    ...init,
  });
}

export function createTextResponse(body: string, init?: ResponseInit): Response {
  return new Response(body, {
    headers: {
      "content-type": "text/plain",
    },
    status: 200,
    ...init,
  });
}

export function createClient(fixtures: JsonFixtureMap): ReturnType<typeof ApCreateClient> {
  return createClientImpl({
    baseUrl: "http://example.test",
    fetch: (input: RequestInfo | URL): Promise<Response> => {
      const url = typeof input === "string" ? input : input instanceof URL ? input.toString() : input.url;
      const parsedUrl = new URL(url);
      const key = `${parsedUrl.pathname}${parsedUrl.search}`;
      const fixture = fixtures[key] ?? fixtures[parsedUrl.pathname];
      if (fixture instanceof Response) {
        return Promise.resolve(fixture);
      }
      if (fixture === undefined) {
        throw new Error(`missing fixture for ${key}`);
      }
      return Promise.resolve(createJsonResponse(fixture));
    },
  });
}
