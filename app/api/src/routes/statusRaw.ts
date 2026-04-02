import { ReadinessModel, RouteClass, type RouteDescriptor } from "../contracts/common.js";
import type { ApiError } from "../contracts/errors.js";
import type { Result } from "../contracts/result.js";
import { failContract } from "../internal/errors.js";
import type { ClientContext } from "../internal/transport.js";
import {
  expectNumber,
  expectObject,
  expectOptionalNumber,
  expectString,
  readField,
  readOptionalField,
} from "../internal/json.js";
import { encodePathSegment } from "../internal/query.js";
import { requestJsonResult } from "../internal/transport.js";

/**
 * One per-stage sync status entry from `/api/v1/status`.
 */
export interface StageStatus {
  readonly indexedHeight: number;
  readonly updatedAt: string;
}

/**
 * Daemon readiness and indexing summary.
 */
export interface StatusResponse {
  readonly syncStatus: string;
  readonly blockHeight: number;
  readonly indices: Readonly<Record<string, StageStatus>>;
  readonly version: string;
}

/**
 * Source class for one raw payload response.
 */
export enum RawValueSource {
  INDEXED = "indexed",
  LIVE = "live",
}

/**
 * Raw block payload response.
 */
export interface RawBlockResponse {
  readonly asOfHeight: number;
  readonly source: RawValueSource;
  readonly rawblock: string;
}

/**
 * Raw transaction payload response.
 */
export interface RawTxResponse {
  readonly asOfHeight?: number;
  readonly source: RawValueSource;
  readonly rawtx: string;
}

/**
 * Route methods for daemon status.
 */
export interface StatusRoutes {
  /**
   * Read the daemon status and per-stage indexing summary.
   */
  getStatus(): Promise<Result<StatusResponse, ApiError>>;
}

/**
 * Route methods for raw payload responses.
 */
export interface RawRoutes {
  /**
   * Read one raw serialized block payload by height or hash.
   */
  getBlock(hashOrHeight: number | string): Promise<Result<RawBlockResponse, ApiError>>;

  /**
   * Read one raw serialized transaction payload by txid.
   */
  getTx(txid: string): Promise<Result<RawTxResponse, ApiError>>;
}

const STATUS_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "status",
  path: "/api/v1/status",
  readinessModel: ReadinessModel.NONE,
  routeClass: RouteClass.CROSS_DOMAIN,
};

const RAW_BLOCK_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "rawBlock",
  path: "/api/v1/rawblock/{hashOrHeight}",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.RAW,
};

const RAW_TX_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "rawTx",
  path: "/api/v1/rawtx/{txid}",
  readinessModel: ReadinessModel.NONE,
  routeClass: RouteClass.RAW,
};

/**
 * Public route descriptors for status and raw payload routes.
 */
export const statusRawRouteDescriptors = {
  rawBlock: RAW_BLOCK_ROUTE,
  rawTx: RAW_TX_ROUTE,
  status: STATUS_ROUTE,
} as const;

function parseStageStatus(value: unknown, path: string): StageStatus {
  const object = expectObject(value, path);
  return {
    indexedHeight: expectNumber(readField(object, "indexedHeight", path), `${path}.indexedHeight`),
    updatedAt: expectString(readField(object, "updatedAt", path), `${path}.updatedAt`),
  };
}

function parseStatusResponse(value: unknown, path: string): StatusResponse {
  const object = expectObject(value, path);
  const rawIndices = expectObject(readField(object, "indices", path), `${path}.indices`);
  const indices = Object.fromEntries(
    Object.entries(rawIndices).map(([stage, status]) => [stage, parseStageStatus(status, `${path}.indices.${stage}`)])
  );
  return {
    syncStatus: expectString(readField(object, "syncStatus", path), `${path}.syncStatus`),
    blockHeight: expectNumber(readField(object, "blockHeight", path), `${path}.blockHeight`),
    indices,
    version: expectString(readField(object, "version", path), `${path}.version`),
  };
}

function parseRawValueSource(value: unknown, path: string): RawValueSource {
  const source = expectString(value, path);
  switch (source) {
    case "indexed":
      return RawValueSource.INDEXED;
    case "live":
      return RawValueSource.LIVE;
  }
  return failContract(path, `unsupported raw value source ${source}`);
}

function parseRawBlockResponse(value: unknown, path: string): RawBlockResponse {
  const object = expectObject(value, path);
  return {
    asOfHeight: expectNumber(readField(object, "asOfHeight", path), `${path}.asOfHeight`),
    source: parseRawValueSource(readField(object, "source", path), `${path}.source`),
    rawblock: expectString(readField(object, "rawblock", path), `${path}.rawblock`),
  };
}

function parseRawTxResponse(value: unknown, path: string): RawTxResponse {
  const object = expectObject(value, path);
  const asOfHeight = expectOptionalNumber(readOptionalField(object, "asOfHeight"), `${path}.asOfHeight`);
  return {
    ...(asOfHeight == null ? {} : { asOfHeight }),
    source: parseRawValueSource(readField(object, "source", path), `${path}.source`),
    rawtx: expectString(readField(object, "rawtx", path), `${path}.rawtx`),
  };
}

export function buildStatusRoutes(context: ClientContext): StatusRoutes {
  return {
    async getStatus(): Promise<Result<StatusResponse, ApiError>> {
      return requestJsonResult(context, STATUS_ROUTE, STATUS_ROUTE.path, parseStatusResponse);
    },
  };
}

export function buildRawRoutes(context: ClientContext): RawRoutes {
  return {
    async getBlock(hashOrHeight: number | string): Promise<Result<RawBlockResponse, ApiError>> {
      return requestJsonResult(
        context,
        RAW_BLOCK_ROUTE,
        `/api/v1/rawblock/${encodePathSegment(String(hashOrHeight))}`,
        parseRawBlockResponse
      );
    },

    async getTx(txid: string): Promise<Result<RawTxResponse, ApiError>> {
      return requestJsonResult(context, RAW_TX_ROUTE, `/api/v1/rawtx/${encodePathSegment(txid)}`, parseRawTxResponse);
    },
  };
}
