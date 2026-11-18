import { describe, expect, it } from "vitest";

import { ApAvailabilityKind, ApCreateClient } from "../src/index.js";
import { createJsonResponse, createNdjsonResponse, expectErr, expectOk } from "./support.js";

describe("stream routes", (): void => {
  it("upserts a re-sent mempool row when its lock arrives", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createNdjsonResponse(
            [{}, { tags: { islock: true } }].map((live, index): string =>
              JSON.stringify({
                type: "mempool_tx",
                ts: 1,
                payload: { sequence: 11 + index * 29, txid: "pending-tx", rawtx: "0011", live },
              })
            )
          )
        ),
    });
    const iterable = expectOk(await client.stream.streamEvents());
    const locks = new Map<string, boolean | undefined>();
    for await (const event of iterable) {
      if (event.type !== "mempool_tx") {
        throw new Error("expected mempool event");
      }
      locks.set(event.payload.txid, event.payload.live.tags?.islock);
    }
    expect([...locks]).toEqual([["pending-tx", true]]);
  });

  it("rejects a non-boolean streamed lock flag", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createNdjsonResponse([
            JSON.stringify({
              type: "mempool_tx",
              ts: 1,
              payload: { sequence: 11, txid: "mempool-a", rawtx: "0011", live: { tags: { islock: 1 } } },
            }),
          ])
        ),
    });
    const iterable = expectOk(await client.stream.streamEvents());
    await expect(async (): Promise<void> => {
      for await (const event of iterable) {
        void event;
      }
    }).rejects.toMatchObject({ path: "$.payload.live.tags.islock", message: "expected boolean" });
  });

  it("consumes ndjson live events as one typed async iterable", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createNdjsonResponse([
            JSON.stringify({
              type: "mempool_tx",
              ts: 1,
              payload: {
                sequence: 11,
                txid: "mempool-a",
                rawtx: "0011",
              },
            }),
            JSON.stringify({
              type: "chain_lock",
              ts: 2,
              payload: {
                sequence: 12,
                blockHash: "block-a",
                blockHeight: 2441849,
                rawChainlock: "ccdd",
              },
            }),
            JSON.stringify({
              type: "governance_object",
              ts: 3,
              payload: {
                sequence: 13,
                objectHash: "gov-object",
                payload: {
                  name: "proposal",
                },
              },
            }),
            JSON.stringify({
              type: "governance_vote",
              ts: 4,
              payload: {
                sequence: 14,
                objectHash: "gov-object",
                voteHash: "gov-vote",
                payload: {
                  vote: "yes",
                },
              },
            }),
            JSON.stringify({
              type: "network",
              ts: 5,
              payload: {
                payload: {
                  connections: 14,
                },
              },
            }),
            JSON.stringify({
              type: "heartbeat",
              ts: 6,
            }),
          ])
        ),
    });

    const iterable = expectOk(await client.stream.streamEvents());
    const events: unknown[] = [];
    for await (const event of iterable) {
      events.push(event);
    }

    expect(events).toHaveLength(6);
    expect((events[0] as { type: string }).type).toBe("mempool_tx");
    expect((events[1] as { type: string }).type).toBe("chain_lock");
    expect((events[5] as { type: string }).type).toBe("heartbeat");
  });

  it("returns a contract error when the stream is not ndjson", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          new Response("{}", {
            headers: {
              "content-type": "application/json",
            },
            status: 200,
          })
        ),
    });

    const error = expectErr(await client.stream.streamEvents());
    expect(error.kind).toBe("contract");
    if (error.kind !== "contract") {
      throw new Error("expected contract error");
    }
    expect(error.path).toBe("/api/v1/stream");
  });

  it("decodes a tail ndjson event without a trailing newline", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createNdjsonResponse([
            JSON.stringify({
              type: "heartbeat",
              ts: 1,
            }),
          ]).clone()
        ),
    });

    const iterable = expectOk(await client.stream.streamEvents());
    const events: unknown[] = [];
    for await (const event of iterable) {
      events.push(event);
    }

    expect(events).toHaveLength(1);
    expect((events[0] as { type: string }).type).toBe("heartbeat");
  });

  it("returns a contract error when the stream body is missing", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          new Response(null, {
            headers: {
              "content-type": "application/x-ndjson",
            },
            status: 200,
          })
        ),
    });

    const error = expectErr(await client.stream.streamEvents());
    expect(error.kind).toBe("contract");
    if (error.kind !== "contract") {
      throw new Error("expected contract error");
    }
    expect(error.message).toBe("stream response body is missing");
  });

  it("fails iteration when one streamed event type is unsupported", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createNdjsonResponse([
            JSON.stringify({
              type: "mystery",
              ts: 1,
            }),
          ])
        ),
    });

    const iterable = expectOk(await client.stream.streamEvents());

    await expect(async (): Promise<void> => {
      for await (const _event of iterable) {
        void _event;
      }
    }).rejects.toThrowError("unsupported stream event type mystery");
  });

  it("propagates live availability failures from the stream transport", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createJsonResponse(
            {
              error: {
                code: "service_unavailable",
                message: "stream not ready",
              },
            },
            { status: 503 }
          )
        ),
    });

    const error = expectErr(await client.stream.streamEvents());
    expect(error.kind).toBe("availability");
    if (error.kind !== "availability") {
      throw new Error("expected availability error");
    }
    expect(error.availabilityKind).toBe(ApAvailabilityKind.LIVE_NOT_READY);
  });

  it("accepts governance stream payloads without optional hashes", async (): Promise<void> => {
    const client = ApCreateClient({
      baseUrl: "http://example.test",
      fetch: (): Promise<Response> =>
        Promise.resolve(
          createNdjsonResponse([
            JSON.stringify({
              type: "governance_object",
              ts: 1,
              payload: {
                payload: {
                  name: "proposal",
                },
              },
            }),
            JSON.stringify({
              type: "governance_vote",
              ts: 2,
              payload: {
                payload: {
                  vote: "yes",
                },
              },
            }),
          ])
        ),
    });

    const iterable = expectOk(await client.stream.streamEvents());
    const events: unknown[] = [];
    for await (const event of iterable) {
      events.push(event);
    }

    expect((events[0] as { payload: { objectHash?: string } }).payload.objectHash).toBeUndefined();
    expect((events[1] as { payload: { voteHash?: string } }).payload.voteHash).toBeUndefined();
  });
});
