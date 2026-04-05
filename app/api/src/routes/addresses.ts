import {
  ReadinessModel,
  RouteClass,
  type CursorPageParams,
  type IndexedPage,
  type RouteDescriptor,
} from "../contracts/common.js";
import type { ApiError } from "../contracts/errors.js";
import { type Result } from "../contracts/result.js";
import { failContract } from "../internal/errors.js";
import {
  expectNumber,
  expectObject,
  expectOptionalNumber,
  expectOptionalString,
  expectString,
  parseIndexedPage,
  readField,
  readOptionalField,
} from "../internal/json.js";
import { encodePathSegment, createQuery } from "../internal/query.js";
import { requestJsonResult, type ClientContext } from "../internal/transport.js";
import { parseTransactionTags, type TransactionTags } from "./transactions.js";

/**
 * Snapshot-backed address summary assembled from the spent/address index.
 */
export interface AddressSummary {
  readonly asOfHeight: number;
  readonly address: string;
  readonly balance: number;
  readonly totalReceived: number;
  readonly totalSent: number;
  readonly txCount: number;
  readonly firstSeen?: string;
  readonly lastSeen?: string;
}

/**
 * One indexed address history row.
 */
export type AddressTransactionDirection = "in" | "out";

/**
 * One indexed address history row.
 */
export interface AddressTransactionSummary {
  readonly blockTime?: string;
  readonly txHash: string;
  readonly blockHeight?: number;
  readonly blockPos?: number;
  readonly direction: AddressTransactionDirection;
  readonly txVersion: number;
  readonly netValue: number;
  readonly txType: string;
  readonly txSize: number;
  readonly valueOut: number;
  readonly fee?: number;
  readonly tags: TransactionTags;
}

/**
 * One indexed unspent output row for an address.
 */
export interface AddressUtxo {
  readonly txid: string;
  readonly vout: number;
  readonly value: number;
  readonly height: number;
  readonly scriptPubKey: string;
}

/**
 * One ranked balance row from the balances endpoint.
 */
export interface BalanceEntry {
  readonly rank: number;
  readonly address: string;
  readonly balance: number;
  readonly pctSupply: number;
  readonly totalReceived: number;
  readonly totalSent: number;
  readonly txCount: number;
}

/**
 * Extra ranked-page context attached to the balances endpoint.
 */
export interface BalancePageMeta {
  readonly includedSupply: number;
  readonly supplyDefinition: string;
}

/**
 * Snapshot-backed ranked balance page.
 */
export interface BalancePage {
  readonly asOfHeight: number;
  readonly totalCount?: number;
  readonly value: readonly BalanceEntry[];
  readonly includedSupply: number;
  readonly supplyDefinition: string;
  readonly nextCursor?: string;
}

/**
 * Route methods for address-domain indexed products.
 */
export interface AddressRoutes {
  /**
   * Read one snapshot-backed address summary.
   */
  get(address: string): Promise<Result<AddressSummary, ApiError>>;

  /**
   * Read one indexed page of transaction history for an address.
   */
  getTransactions(
    address: string,
    params?: CursorPageParams
  ): Promise<Result<IndexedPage<AddressTransactionSummary>, ApiError>>;

  /**
   * Read one indexed page of UTXOs for an address.
   */
  getUtxos(address: string, params?: CursorPageParams): Promise<Result<IndexedPage<AddressUtxo>, ApiError>>;

  /**
   * Read one snapshot-backed ranked balances page.
   */
  listBalances(params?: CursorPageParams): Promise<Result<BalancePage, ApiError>>;
}

const ADDRESS_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "address",
  path: "/api/v1/address/{address}",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED,
};

const ADDRESS_TRANSACTIONS_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "addressTransactions",
  path: "/api/v1/address/{address}/txs",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED,
};

const ADDRESS_UTXOS_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "addressUtxos",
  path: "/api/v1/address/{address}/utxos",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED,
};

const BALANCES_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "balances",
  path: "/api/v1/balances",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED,
};

/**
 * Public route descriptors for address-domain routes.
 */
export const addressRouteDescriptors = {
  balances: BALANCES_ROUTE,
  detail: ADDRESS_ROUTE,
  transactions: ADDRESS_TRANSACTIONS_ROUTE,
  utxos: ADDRESS_UTXOS_ROUTE,
} as const;

function parseAddressSummary(value: unknown, path: string): AddressSummary {
  const object = expectObject(value, path);
  const firstSeen = expectOptionalString(readOptionalField(object, "firstSeen"), `${path}.firstSeen`);
  const lastSeen = expectOptionalString(readOptionalField(object, "lastSeen"), `${path}.lastSeen`);
  return {
    asOfHeight: expectNumber(readField(object, "asOfHeight", path), `${path}.asOfHeight`),
    address: expectString(readField(object, "address", path), `${path}.address`),
    balance: expectNumber(readField(object, "balance", path), `${path}.balance`),
    totalReceived: expectNumber(readField(object, "totalReceived", path), `${path}.totalReceived`),
    totalSent: expectNumber(readField(object, "totalSent", path), `${path}.totalSent`),
    txCount: expectNumber(readField(object, "txCount", path), `${path}.txCount`),
    ...(firstSeen == null ? {} : { firstSeen }),
    ...(lastSeen == null ? {} : { lastSeen }),
  };
}

function parseAddressTransactionSummary(value: unknown, path: string): AddressTransactionSummary {
  const object = expectObject(value, path);
  const blockTime = expectOptionalString(readOptionalField(object, "blockTime"), `${path}.blockTime`);
  const blockHeight = expectOptionalNumber(readOptionalField(object, "blockHeight"), `${path}.blockHeight`);
  const blockPos = expectOptionalNumber(readOptionalField(object, "blockPos"), `${path}.blockPos`);
  const direction = expectString(readField(object, "direction", path), `${path}.direction`);
  const fee = expectOptionalNumber(readOptionalField(object, "fee"), `${path}.fee`);
  const netValue = expectNumber(readField(object, "netValue", path), `${path}.netValue`);
  if (direction !== "in" && direction !== "out") {
    throw failContract(`${path}.direction`, `expected "in" | "out", got ${JSON.stringify(direction)}`);
  }
  return {
    ...(blockTime == null ? {} : { blockTime }),
    txHash: expectString(readField(object, "txHash", path), `${path}.txHash`),
    ...(blockHeight == null ? {} : { blockHeight }),
    ...(blockPos == null ? {} : { blockPos }),
    direction,
    txVersion: expectNumber(readField(object, "txVersion", path), `${path}.txVersion`),
    netValue,
    txType: expectString(readField(object, "txType", path), `${path}.txType`),
    txSize: expectNumber(readField(object, "txSize", path), `${path}.txSize`),
    valueOut: expectNumber(readField(object, "valueOut", path), `${path}.valueOut`),
    ...(fee == null ? {} : { fee }),
    tags: parseTransactionTags(readField(object, "tags", path), `${path}.tags`),
  };
}

function parseAddressUtxo(value: unknown, path: string): AddressUtxo {
  const object = expectObject(value, path);
  return {
    txid: expectString(readField(object, "txid", path), `${path}.txid`),
    vout: expectNumber(readField(object, "vout", path), `${path}.vout`),
    value: expectNumber(readField(object, "value", path), `${path}.value`),
    height: expectNumber(readField(object, "height", path), `${path}.height`),
    scriptPubKey: expectString(readField(object, "scriptPubKey", path), `${path}.scriptPubKey`),
  };
}

function parseBalanceEntry(value: unknown, path: string): BalanceEntry {
  const object = expectObject(value, path);
  return {
    rank: expectNumber(readField(object, "rank", path), `${path}.rank`),
    address: expectString(readField(object, "address", path), `${path}.address`),
    balance: expectNumber(readField(object, "balance", path), `${path}.balance`),
    pctSupply: expectNumber(readField(object, "pctSupply", path), `${path}.pctSupply`),
    totalReceived: expectNumber(readField(object, "totalReceived", path), `${path}.totalReceived`),
    totalSent: expectNumber(readField(object, "totalSent", path), `${path}.totalSent`),
    txCount: expectNumber(readField(object, "txCount", path), `${path}.txCount`),
  };
}

function parseBalancePageMeta(value: unknown, path: string): BalancePageMeta {
  const object = expectObject(value, path);
  return {
    includedSupply: expectNumber(readField(object, "includedSupply", path), `${path}.includedSupply`),
    supplyDefinition: expectString(readField(object, "supplyDefinition", path), `${path}.supplyDefinition`),
  };
}

function parseBalancePage(value: unknown, path: string): BalancePage {
  const object = expectObject(value, path);
  const nextCursor = expectOptionalString(readOptionalField(object, "nextCursor"), `${path}.nextCursor`);
  const totalCount = expectOptionalNumber(readOptionalField(object, "totalCount"), `${path}.totalCount`);
  const topLevelIncludedSupply = expectOptionalNumber(
    readOptionalField(object, "includedSupply"),
    `${path}.includedSupply`
  );
  const topLevelSupplyDefinition = expectOptionalString(
    readOptionalField(object, "supplyDefinition"),
    `${path}.supplyDefinition`
  );
  const metaValue = readOptionalField(object, "meta");
  const meta = metaValue == null ? undefined : parseBalancePageMeta(metaValue, `${path}.meta`);
  const includedSupply = topLevelIncludedSupply ?? meta?.includedSupply;
  const supplyDefinition = topLevelSupplyDefinition ?? meta?.supplyDefinition;
  return {
    asOfHeight: expectNumber(readField(object, "asOfHeight", path), `${path}.asOfHeight`),
    ...(totalCount == null ? {} : { totalCount }),
    value: parseIndexedPage(value, path, parseBalanceEntry).value,
    includedSupply: expectNumber(includedSupply, `${path}.includedSupply`),
    supplyDefinition: expectString(supplyDefinition, `${path}.supplyDefinition`),
    ...(nextCursor == null ? {} : { nextCursor }),
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
 * Build the address-domain route family for one client transport context.
 */
export function buildAddressRoutes(context: ClientContext): AddressRoutes {
  return {
    async get(address: string): Promise<Result<AddressSummary, ApiError>> {
      return requestJsonResult(
        context,
        ADDRESS_ROUTE,
        `/api/v1/address/${encodePathSegment(address)}`,
        parseAddressSummary
      );
    },

    async getTransactions(
      address: string,
      params?: CursorPageParams
    ): Promise<Result<IndexedPage<AddressTransactionSummary>, ApiError>> {
      const query = createCursorQuery(params);
      return requestJsonResult(
        context,
        ADDRESS_TRANSACTIONS_ROUTE,
        `/api/v1/address/${encodePathSegment(address)}/txs`,
        (value: unknown, path: string) => parseIndexedPage(value, path, parseAddressTransactionSummary),
        ...(query == null ? [] : [{ query }])
      );
    },

    async getUtxos(address: string, params?: CursorPageParams): Promise<Result<IndexedPage<AddressUtxo>, ApiError>> {
      const query = createCursorQuery(params);
      return requestJsonResult(
        context,
        ADDRESS_UTXOS_ROUTE,
        `/api/v1/address/${encodePathSegment(address)}/utxos`,
        (value: unknown, path: string) => parseIndexedPage(value, path, parseAddressUtxo),
        ...(query == null ? [] : [{ query }])
      );
    },

    async listBalances(params?: CursorPageParams): Promise<Result<BalancePage, ApiError>> {
      const query = createCursorQuery(params);
      return requestJsonResult(
        context,
        BALANCES_ROUTE,
        BALANCES_ROUTE.path,
        parseBalancePage,
        ...(query == null ? [] : [{ query }])
      );
    },
  };
}
