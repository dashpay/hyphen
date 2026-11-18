import {
  ReadinessModel,
  RouteClass,
  type CursorPageParams,
  type LivePage,
  type RouteDescriptor,
} from "../contracts/common.js";
import type { ApiError } from "../contracts/errors.js";
import { type Result } from "../contracts/result.js";
import {
  expectArray,
  expectBoolean,
  expectNumber,
  expectObject,
  expectOptionalNumber,
  expectOptionalString,
  expectString,
  parseLivePage,
  readField,
  readOptionalField,
} from "../internal/json.js";
import { createQuery } from "../internal/query.js";
import { requestJsonResult, type ClientContext } from "../internal/transport.js";
import { readTransactionLive, type TransactionLive, type TransactionTags } from "./transactions.js";

/**
 * Live mempool summary snapshot.
 */
export interface MempoolSummary {
  readonly count: number;
  readonly totalBytes: number;
  readonly usageBytes?: number;
  readonly maxBytes?: number;
  readonly updatedAt: string;
  readonly recentHashes: readonly string[];
}

/**
 * One live mempool transaction summary row.
 */
export interface MempoolTransactionSummary {
  readonly txHash: string;
  readonly txType: string;
  readonly txSize: number;
  readonly valueOut: number;
  readonly firstSeenAt: string;
  readonly lastSeenAt: string;
  readonly tags: TransactionTags;
  readonly live: TransactionLive;
}

/**
 * Typed networkInfo payload from the live network snapshot.
 */
export interface NetworkInfo {
  readonly version?: number;
  readonly subversion?: string;
  readonly protocolVersion?: number;
  readonly timeOffset?: number;
  readonly connections?: number;
  readonly relayFee?: number;
  readonly warnings?: string;
}

/**
 * Typed peer summary from the live network snapshot.
 */
export interface PeerInfo {
  readonly id?: number;
  readonly addr?: string;
  readonly addrLocal?: string;
  readonly subversion?: string;
  readonly inbound?: boolean;
  readonly startingHeight?: number;
  readonly syncedHeaders?: number;
  readonly syncedBlocks?: number;
  readonly connectionType?: string;
}

/**
 * Validator status aggregates by validator class.
 */
export interface NetworkValidatorClassSummary {
  readonly total: number;
  readonly status: Readonly<Record<string, number>>;
}

/**
 * Validator aggregates attached to the live network snapshot.
 */
export interface NetworkValidatorSummary {
  readonly regular: NetworkValidatorClassSummary;
  readonly evonode: NetworkValidatorClassSummary;
}

function readOptionalAliasField(object: Record<string, unknown>, names: readonly string[]): unknown {
  for (const name of names) {
    const value = readOptionalField(object, name);
    if (value !== undefined) {
      return value;
    }
  }
  return undefined;
}

/**
 * Live network snapshot response.
 */
export interface NetworkSnapshot {
  readonly networkInfo: NetworkInfo;
  readonly peerInfo: readonly PeerInfo[];
  readonly peerCount: number;
  readonly validators: NetworkValidatorSummary;
  readonly updatedAt: string;
}

/**
 * Route methods for live mempool and network capabilities.
 */
export interface LiveRoutes {
  /**
   * Read the live mempool summary snapshot.
   */
  getMempoolSummary(): Promise<Result<MempoolSummary, ApiError>>;

  /**
   * Read one live page of recent mempool transactions.
   */
  listMempoolTransactions(params?: CursorPageParams): Promise<Result<LivePage<MempoolTransactionSummary>, ApiError>>;

  /**
   * Read the current live network snapshot.
   */
  getNetworkSnapshot(): Promise<Result<NetworkSnapshot, ApiError>>;
}

const MEMPOOL_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "mempool",
  path: "/api/v1/mempool",
  readinessModel: ReadinessModel.LIVE,
  routeClass: RouteClass.LIVE,
};

const MEMPOOL_TRANSACTIONS_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "mempoolTransactions",
  path: "/api/v1/mempool/txs",
  readinessModel: ReadinessModel.LIVE,
  routeClass: RouteClass.LIVE,
};

const NETWORK_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "network",
  path: "/api/v1/network",
  readinessModel: ReadinessModel.LIVE,
  routeClass: RouteClass.LIVE,
};

/**
 * Public route descriptors for live JSON routes.
 */
export const liveRouteDescriptors = {
  mempool: MEMPOOL_ROUTE,
  mempoolTransactions: MEMPOOL_TRANSACTIONS_ROUTE,
  network: NETWORK_ROUTE,
} as const;

function parseTransactionTags(value: unknown, path: string): TransactionTags {
  const object = expectObject(value, path);
  return {
    mined: expectBoolean(readField(object, "mined", path), `${path}.mined`),
    coinbase: expectBoolean(readField(object, "coinbase", path), `${path}.coinbase`),
  };
}

function parseStringArray(value: unknown, path: string): readonly string[] {
  return expectArray(value, path, expectString);
}

function parseMempoolSummary(value: unknown, path: string): MempoolSummary {
  const object = expectObject(value, path);
  const usageBytes = expectOptionalNumber(readOptionalField(object, "usageBytes"), `${path}.usageBytes`);
  const maxBytes = expectOptionalNumber(readOptionalField(object, "maxBytes"), `${path}.maxBytes`);
  return {
    count: expectNumber(readField(object, "count", path), `${path}.count`),
    totalBytes: expectNumber(readField(object, "totalBytes", path), `${path}.totalBytes`),
    ...(usageBytes == null ? {} : { usageBytes }),
    ...(maxBytes == null ? {} : { maxBytes }),
    updatedAt: expectString(readField(object, "updatedAt", path), `${path}.updatedAt`),
    recentHashes: parseStringArray(readField(object, "recentHashes", path), `${path}.recentHashes`),
  };
}

function parseMempoolTransactionSummary(value: unknown, path: string): MempoolTransactionSummary {
  const object = expectObject(value, path);
  return {
    txHash: expectString(readField(object, "txHash", path), `${path}.txHash`),
    txType: expectString(readField(object, "txType", path), `${path}.txType`),
    txSize: expectNumber(readField(object, "txSize", path), `${path}.txSize`),
    valueOut: expectNumber(readField(object, "valueOut", path), `${path}.valueOut`),
    firstSeenAt: expectString(readField(object, "firstSeenAt", path), `${path}.firstSeenAt`),
    lastSeenAt: expectString(readField(object, "lastSeenAt", path), `${path}.lastSeenAt`),
    tags: parseTransactionTags(readField(object, "tags", path), `${path}.tags`),
    live: readTransactionLive(object, path),
  };
}

function parseNetworkValidatorClassSummary(value: unknown, path: string): NetworkValidatorClassSummary {
  const object = expectObject(value, path);
  const statusObject = expectObject(readField(object, "status", path), `${path}.status`);
  const status: Record<string, number> = {};
  for (const [key, raw] of Object.entries(statusObject)) {
    status[key] = expectNumber(raw, `${path}.status.${key}`);
  }
  return {
    total: expectNumber(readField(object, "total", path), `${path}.total`),
    status,
  };
}

function parseNetworkValidatorSummary(value: unknown, path: string): NetworkValidatorSummary {
  const object = expectObject(value, path);
  return {
    regular: parseNetworkValidatorClassSummary(readField(object, "regular", path), `${path}.regular`),
    evonode: parseNetworkValidatorClassSummary(readField(object, "evonode", path), `${path}.evonode`),
  };
}

function parseNetworkInfo(value: unknown, path: string): NetworkInfo {
  const object = expectObject(value, path);
  const version = expectOptionalNumber(readOptionalField(object, "version"), `${path}.version`);
  const subversion = expectOptionalString(readOptionalField(object, "subversion"), `${path}.subversion`);
  const protocolVersion = expectOptionalNumber(
    readOptionalAliasField(object, ["protocolVersion", "protocolversion"]),
    `${path}.protocolVersion`
  );
  const timeOffset = expectOptionalNumber(
    readOptionalAliasField(object, ["timeOffset", "timeoffset"]),
    `${path}.timeOffset`
  );
  const connections = expectOptionalNumber(readOptionalField(object, "connections"), `${path}.connections`);
  const relayFee = expectOptionalNumber(readOptionalAliasField(object, ["relayFee", "relayfee"]), `${path}.relayFee`);
  const warnings = expectOptionalString(readOptionalField(object, "warnings"), `${path}.warnings`);
  return {
    ...(version == null ? {} : { version }),
    ...(subversion == null ? {} : { subversion }),
    ...(protocolVersion == null ? {} : { protocolVersion }),
    ...(timeOffset == null ? {} : { timeOffset }),
    ...(connections == null ? {} : { connections }),
    ...(relayFee == null ? {} : { relayFee }),
    ...(warnings == null ? {} : { warnings }),
  };
}

function parsePeerInfo(value: unknown, path: string): PeerInfo {
  const object = expectObject(value, path);
  const id = expectOptionalNumber(readOptionalField(object, "id"), `${path}.id`);
  const addr = expectOptionalString(readOptionalField(object, "addr"), `${path}.addr`);
  const addrLocal = expectOptionalString(
    readOptionalAliasField(object, ["addrLocal", "addrlocal"]),
    `${path}.addrLocal`
  );
  const subversion = expectOptionalString(
    readOptionalAliasField(object, ["subversion", "subver"]),
    `${path}.subversion`
  );
  const inbound = readOptionalField(object, "inbound");
  const startingHeight = expectOptionalNumber(
    readOptionalAliasField(object, ["startingHeight", "startingheight"]),
    `${path}.startingHeight`
  );
  const syncedHeaders = expectOptionalNumber(readOptionalField(object, "syncedHeaders"), `${path}.syncedHeaders`);
  const syncedBlocks = expectOptionalNumber(readOptionalField(object, "syncedBlocks"), `${path}.syncedBlocks`);
  const connectionType = expectOptionalString(readOptionalField(object, "connectionType"), `${path}.connectionType`);
  return {
    ...(id == null ? {} : { id }),
    ...(addr == null ? {} : { addr }),
    ...(addrLocal == null ? {} : { addrLocal }),
    ...(subversion == null ? {} : { subversion }),
    ...(inbound == null ? {} : { inbound: expectBoolean(inbound, `${path}.inbound`) }),
    ...(startingHeight == null ? {} : { startingHeight }),
    ...(syncedHeaders == null ? {} : { syncedHeaders }),
    ...(syncedBlocks == null ? {} : { syncedBlocks }),
    ...(connectionType == null ? {} : { connectionType }),
  };
}

function parseNetworkSnapshot(value: unknown, path: string): NetworkSnapshot {
  const object = expectObject(value, path);
  return {
    networkInfo: parseNetworkInfo(readField(object, "networkInfo", path), `${path}.networkInfo`),
    peerInfo: expectArray(readField(object, "peerInfo", path), `${path}.peerInfo`, parsePeerInfo),
    peerCount: expectNumber(readField(object, "peerCount", path), `${path}.peerCount`),
    validators: parseNetworkValidatorSummary(readField(object, "validators", path), `${path}.validators`),
    updatedAt: expectString(readField(object, "updatedAt", path), `${path}.updatedAt`),
  };
}

function createCursorQuery(params?: CursorPageParams): URLSearchParams | undefined {
  if (params == null) {
    return undefined;
  }
  const query = createQuery({
    cursor: params.cursor,
    limit: params.limit,
  });
  return query.size > 0 ? query : undefined;
}

/**
 * Build the live mempool and network route family for one client transport context.
 */
export function buildLiveRoutes(context: ClientContext): LiveRoutes {
  return {
    async getMempoolSummary(): Promise<Result<MempoolSummary, ApiError>> {
      return requestJsonResult(context, MEMPOOL_ROUTE, MEMPOOL_ROUTE.path, parseMempoolSummary);
    },

    async getNetworkSnapshot(): Promise<Result<NetworkSnapshot, ApiError>> {
      return requestJsonResult(context, NETWORK_ROUTE, NETWORK_ROUTE.path, parseNetworkSnapshot);
    },

    async listMempoolTransactions(
      params?: CursorPageParams
    ): Promise<Result<LivePage<MempoolTransactionSummary>, ApiError>> {
      const query = createCursorQuery(params);
      return requestJsonResult(
        context,
        MEMPOOL_TRANSACTIONS_ROUTE,
        MEMPOOL_TRANSACTIONS_ROUTE.path,
        (value: unknown, path: string) => parseLivePage(value, path, parseMempoolTransactionSummary),
        ...(query == null ? [] : [{ query }])
      );
    },
  };
}
