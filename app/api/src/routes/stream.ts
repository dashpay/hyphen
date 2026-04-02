import { ReadinessModel, RouteClass, type RouteDescriptor } from "../contracts/common.js";
import { contractError, type ApiError } from "../contracts/errors.js";
import { err, ok, type Result } from "../contracts/result.js";
import { failContract } from "../internal/errors.js";
import {
  expectNumber,
  expectObject,
  expectOptionalNumber,
  expectOptionalString,
  expectString,
  readField,
  readOptionalField,
} from "../internal/json.js";
import { requestResponseResult, type ClientContext } from "../internal/transport.js";

/**
 * Stream payload for one live mempool transaction event.
 */
export interface StreamMempoolTransactionEvent {
  readonly sequence: number;
  readonly txid: string;
  readonly rawtx: string;
}

/**
 * Stream payload for one live ChainLock event.
 */
export interface StreamChainLockEvent {
  readonly sequence?: number;
  readonly blockHash: string;
  readonly blockHeight?: number;
  readonly rawChainlock: string;
}

/**
 * Stream payload for one live governance object event.
 */
export interface StreamGovernanceObjectEvent {
  readonly sequence?: number;
  readonly objectHash?: string;
  readonly payload: unknown;
}

/**
 * Stream payload for one live governance vote event.
 */
export interface StreamGovernanceVoteEvent {
  readonly sequence?: number;
  readonly objectHash?: string;
  readonly voteHash?: string;
  readonly payload: unknown;
}

/**
 * Stream payload for one live network event.
 */
export interface StreamNetworkEvent {
  readonly payload: unknown;
}

/**
 * Stream event variant for a mempool transaction.
 */
export interface StreamMempoolEvent {
  readonly type: "mempool_tx";
  readonly ts: number;
  readonly payload: StreamMempoolTransactionEvent;
}

/**
 * Stream event variant for a ChainLock update.
 */
export interface StreamChainLockEnvelope {
  readonly type: "chain_lock";
  readonly ts: number;
  readonly payload: StreamChainLockEvent;
}

/**
 * Stream event variant for a governance object update.
 */
export interface StreamGovernanceObjectEnvelope {
  readonly type: "governance_object";
  readonly ts: number;
  readonly payload: StreamGovernanceObjectEvent;
}

/**
 * Stream event variant for a governance vote update.
 */
export interface StreamGovernanceVoteEnvelope {
  readonly type: "governance_vote";
  readonly ts: number;
  readonly payload: StreamGovernanceVoteEvent;
}

/**
 * Stream event variant for a network snapshot refresh.
 */
export interface StreamNetworkEnvelope {
  readonly type: "network";
  readonly ts: number;
  readonly payload: StreamNetworkEvent;
}

/**
 * Stream event variant for an idle heartbeat.
 */
export interface StreamHeartbeatEvent {
  readonly type: "heartbeat";
  readonly ts: number;
}

/**
 * Discriminated NDJSON event emitted by the live stream endpoint.
 */
export type StreamEvent =
  | StreamChainLockEnvelope
  | StreamGovernanceObjectEnvelope
  | StreamGovernanceVoteEnvelope
  | StreamHeartbeatEvent
  | StreamMempoolEvent
  | StreamNetworkEnvelope;

/**
 * Route methods for the live NDJSON event stream.
 */
export interface StreamRoutes {
  /**
   * Open the live NDJSON stream as an async iterable of typed events.
   */
  streamEvents(): Promise<Result<AsyncIterable<StreamEvent>, ApiError>>;
}

const STREAM_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "stream",
  path: "/api/v1/stream",
  readinessModel: ReadinessModel.LIVE,
  routeClass: RouteClass.LIVE,
};

/**
 * Public route descriptors for NDJSON stream routes.
 */
export const streamRouteDescriptors = {
  events: STREAM_ROUTE,
} as const;

function parseMempoolPayload(value: unknown, path: string): StreamMempoolTransactionEvent {
  const object = expectObject(value, path);
  return {
    sequence: expectNumber(readField(object, "sequence", path), `${path}.sequence`),
    txid: expectString(readField(object, "txid", path), `${path}.txid`),
    rawtx: expectString(readField(object, "rawtx", path), `${path}.rawtx`),
  };
}

function parseChainLockPayload(value: unknown, path: string): StreamChainLockEvent {
  const object = expectObject(value, path);
  const sequence = expectOptionalNumber(readOptionalField(object, "sequence"), `${path}.sequence`);
  const blockHeight = expectOptionalNumber(readOptionalField(object, "blockHeight"), `${path}.blockHeight`);
  return {
    ...(sequence == null ? {} : { sequence }),
    blockHash: expectString(readField(object, "blockHash", path), `${path}.blockHash`),
    ...(blockHeight == null ? {} : { blockHeight }),
    rawChainlock: expectString(readField(object, "rawChainlock", path), `${path}.rawChainlock`),
  };
}

function parseGovernanceObjectPayload(value: unknown, path: string): StreamGovernanceObjectEvent {
  const object = expectObject(value, path);
  const sequence = expectOptionalNumber(readOptionalField(object, "sequence"), `${path}.sequence`);
  const objectHash = expectOptionalString(readOptionalField(object, "objectHash"), `${path}.objectHash`);
  return {
    ...(sequence == null ? {} : { sequence }),
    ...(objectHash == null ? {} : { objectHash }),
    payload: readField(object, "payload", path),
  };
}

function parseGovernanceVotePayload(value: unknown, path: string): StreamGovernanceVoteEvent {
  const object = expectObject(value, path);
  const sequence = expectOptionalNumber(readOptionalField(object, "sequence"), `${path}.sequence`);
  const objectHash = expectOptionalString(readOptionalField(object, "objectHash"), `${path}.objectHash`);
  const voteHash = expectOptionalString(readOptionalField(object, "voteHash"), `${path}.voteHash`);
  return {
    ...(sequence == null ? {} : { sequence }),
    ...(objectHash == null ? {} : { objectHash }),
    ...(voteHash == null ? {} : { voteHash }),
    payload: readField(object, "payload", path),
  };
}

function parseNetworkPayload(value: unknown, path: string): StreamNetworkEvent {
  const object = expectObject(value, path);
  return {
    payload: readField(object, "payload", path),
  };
}

function parseStreamEvent(value: unknown, path: string): StreamEvent {
  const object = expectObject(value, path);
  const type = expectString(readField(object, "type", path), `${path}.type`);
  const ts = expectNumber(readField(object, "ts", path), `${path}.ts`);
  switch (type) {
    case "mempool_tx":
      return {
        type,
        ts,
        payload: parseMempoolPayload(readField(object, "payload", path), `${path}.payload`),
      };
    case "chain_lock":
      return {
        type,
        ts,
        payload: parseChainLockPayload(readField(object, "payload", path), `${path}.payload`),
      };
    case "governance_object":
      return {
        type,
        ts,
        payload: parseGovernanceObjectPayload(readField(object, "payload", path), `${path}.payload`),
      };
    case "governance_vote":
      return {
        type,
        ts,
        payload: parseGovernanceVotePayload(readField(object, "payload", path), `${path}.payload`),
      };
    case "network":
      return {
        type,
        ts,
        payload: parseNetworkPayload(readField(object, "payload", path), `${path}.payload`),
      };
    case "heartbeat":
      return {
        type,
        ts,
      };
    default:
      return failContract(`${path}.type`, `unsupported stream event type ${type}`);
  }
}

async function* decodeNdjson(body: ReadableStream<Uint8Array>): AsyncIterable<StreamEvent> {
  const reader = body.getReader();
  const decoder = new TextDecoder();
  let buffer = "";

  for (;;) {
    const chunk = await reader.read();
    if (chunk.done) {
      break;
    }
    buffer += decoder.decode(chunk.value, { stream: true });
    let newlineIndex = buffer.indexOf("\n");
    while (newlineIndex >= 0) {
      const line = buffer.slice(0, newlineIndex).trim();
      buffer = buffer.slice(newlineIndex + 1);
      if (line.length > 0) {
        yield parseStreamEvent(JSON.parse(line) as unknown, "$");
      }
      newlineIndex = buffer.indexOf("\n");
    }
  }

  const tail = `${buffer}${decoder.decode()}`.trim();
  if (tail.length > 0) {
    yield parseStreamEvent(JSON.parse(tail) as unknown, "$");
  }
}

/**
 * Build the live stream route family for one client transport context.
 */
export function buildStreamRoutes(context: ClientContext): StreamRoutes {
  return {
    async streamEvents(): Promise<Result<AsyncIterable<StreamEvent>, ApiError>> {
      const responseResult = await requestResponseResult(context, STREAM_ROUTE, STREAM_ROUTE.path);
      const response = responseResult.value();
      if (response == null) {
        return err(responseResult.errorOr(contractError(STREAM_ROUTE.path, "stream request failed")));
      }
      const contentType = response.headers.get("content-type");
      if (!contentType?.includes("application/x-ndjson")) {
        return err(contractError(STREAM_ROUTE.path, "expected application/x-ndjson response"));
      }
      const body = response.body;
      if (body == null) {
        return err(contractError(STREAM_ROUTE.path, "stream response body is missing"));
      }
      return ok<AsyncIterable<StreamEvent>, ApiError>(decodeNdjson(body));
    },
  };
}
