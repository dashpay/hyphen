import {
  ReadinessModel,
  RouteClass,
  type CursorPageParams,
  type IndexedPage,
  type RouteDescriptor,
} from "../contracts/common.js";
import type { ApiError } from "../contracts/errors.js";
import { type Result } from "../contracts/result.js";
import {
  expectArray,
  expectBoolean,
  expectNumber,
  expectObject,
  expectOptionalBoolean,
  expectOptionalNumber,
  expectOptionalString,
  expectString,
  parseIndexedPage,
  readField,
  readOptionalField,
} from "../internal/json.js";
import { encodePathSegment, createQuery } from "../internal/query.js";
import { requestJsonResult, type ClientContext } from "../internal/transport.js";

/**
 * Indexed transaction tags returned by snapshot-backed transaction routes.
 */
export interface TransactionTags {
  readonly mined: boolean;
  readonly coinbase: boolean;
}

/**
 * Nullable live transaction tags attached under the overlay object.
 */
export interface TransactionLiveTags {
  readonly mempool?: boolean;
  readonly islock?: boolean;
}

/**
 * Nullable live overlay attached to indexed transaction detail routes.
 */
export interface TransactionLive {
  readonly tags?: TransactionLiveTags;
}

/**
 * Script signature payload for one transaction input.
 */
export interface TransactionScriptSig {
  readonly hex: string;
}

/**
 * Script public key payload for one transaction output.
 */
export interface TransactionScriptPubKey {
  readonly hex: string;
}

/**
 * One decoded transaction input.
 */
export interface TransactionInput {
  readonly n: number;
  readonly scriptSig: TransactionScriptSig;
  readonly sequence: number;
  readonly address?: string;
  readonly value?: number;
  readonly txid?: string;
  readonly vout?: number;
  readonly coinbase?: string;
}

/**
 * One decoded transaction output.
 */
export interface TransactionOutput {
  readonly n: number;
  readonly value: number;
  readonly scriptPubKey: TransactionScriptPubKey;
  readonly address?: string;
  readonly spent: boolean;
}

/**
 * Indexed transaction summary returned by paginated list routes.
 */
export interface TransactionSummary {
  readonly txid: string;
  readonly blockHeight: number;
  readonly blockPos: number;
  readonly version: number;
  readonly txType: string;
  readonly lockTime: number;
  readonly size: number;
  readonly valueOut: number;
  readonly inputCount: number;
  readonly outputCount: number;
  readonly tags: TransactionTags;
}

/**
 * Indexed or live-fallback transaction detail response.
 *
 * Indexed fields stay at the top level. Nullable live overlay state stays
 * nested under `live`, matching the daemon contract.
 */
export interface TransactionDetail {
  readonly asOfHeight?: number;
  readonly txid: string;
  readonly blockHeight?: number;
  readonly blockPos?: number;
  readonly version: number;
  readonly txType: string;
  readonly lockTime: number;
  readonly size: number;
  readonly valueOut: number;
  readonly inputCount: number;
  readonly outputCount: number;
  readonly fee?: number;
  readonly tags: TransactionTags;
  readonly live: TransactionLive;
  readonly payload?: unknown;
  readonly vin: readonly TransactionInput[];
  readonly vout: readonly TransactionOutput[];
}

/**
 * Route methods for indexed transaction data and indexed-with-live detail.
 */
export interface TransactionRoutes {
  /**
   * Read the canonical indexed transaction list.
   */
  list(params?: CursorPageParams): Promise<Result<IndexedPage<TransactionSummary>, ApiError>>;

  /**
   * Read one transaction by txid.
   *
   * This route is primarily indexed, but can fall back to the live mempool
   * view when the daemon exposes that capability.
   */
  get(txid: string): Promise<Result<TransactionDetail, ApiError>>;
}

const TRANSACTIONS_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "transactions",
  path: "/api/v1/txs",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED,
};

const TRANSACTION_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "transaction",
  path: "/api/v1/tx/{txid}",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED_OVERLAY,
};

/**
 * Public route descriptors for transaction routes.
 */
export const transactionRouteDescriptors = {
  detail: TRANSACTION_ROUTE,
  list: TRANSACTIONS_ROUTE,
} as const;

export function parseTransactionTags(value: unknown, path: string): TransactionTags {
  const object = expectObject(value, path);
  return {
    mined: expectBoolean(readField(object, "mined", path), `${path}.mined`),
    coinbase: expectBoolean(readField(object, "coinbase", path), `${path}.coinbase`),
  };
}

function parseTransactionLiveTags(value: unknown, path: string): TransactionLiveTags {
  const object = expectObject(value, path);
  const mempool = expectOptionalBoolean(readOptionalField(object, "mempool"), `${path}.mempool`);
  const islock = expectOptionalBoolean(readOptionalField(object, "islock"), `${path}.islock`);
  return {
    ...(mempool == null ? {} : { mempool }),
    ...(islock == null ? {} : { islock }),
  };
}

function parseTransactionLive(value: unknown, path: string): TransactionLive {
  const object = expectObject(value, path);
  const tagsValue = readOptionalField(object, "tags");
  return {
    ...(tagsValue == null ? {} : { tags: parseTransactionLiveTags(tagsValue, `${path}.tags`) }),
  };
}

function parseTransactionScriptSig(value: unknown, path: string): TransactionScriptSig {
  const object = expectObject(value, path);
  return {
    hex: expectString(readField(object, "hex", path), `${path}.hex`),
  };
}

function parseTransactionScriptPubKey(value: unknown, path: string): TransactionScriptPubKey {
  const object = expectObject(value, path);
  return {
    hex: expectString(readField(object, "hex", path), `${path}.hex`),
  };
}

function parseTransactionInput(value: unknown, path: string): TransactionInput {
  const object = expectObject(value, path);
  const address = expectOptionalString(readOptionalField(object, "address"), `${path}.address`);
  const valueAmount = expectOptionalNumber(readOptionalField(object, "value"), `${path}.value`);
  const txid = expectOptionalString(readOptionalField(object, "txid"), `${path}.txid`);
  const vout = expectOptionalNumber(readOptionalField(object, "vout"), `${path}.vout`);
  const coinbase = expectOptionalString(readOptionalField(object, "coinbase"), `${path}.coinbase`);
  return {
    n: expectNumber(readField(object, "n", path), `${path}.n`),
    scriptSig: parseTransactionScriptSig(readField(object, "scriptSig", path), `${path}.scriptSig`),
    sequence: expectNumber(readField(object, "sequence", path), `${path}.sequence`),
    ...(address == null ? {} : { address }),
    ...(valueAmount == null ? {} : { value: valueAmount }),
    ...(txid == null ? {} : { txid }),
    ...(vout == null ? {} : { vout }),
    ...(coinbase == null ? {} : { coinbase }),
  };
}

function parseTransactionOutput(value: unknown, path: string): TransactionOutput {
  const object = expectObject(value, path);
  const address = expectOptionalString(readOptionalField(object, "address"), `${path}.address`);
  return {
    n: expectNumber(readField(object, "n", path), `${path}.n`),
    value: expectNumber(readField(object, "value", path), `${path}.value`),
    scriptPubKey: parseTransactionScriptPubKey(readField(object, "scriptPubKey", path), `${path}.scriptPubKey`),
    ...(address == null ? {} : { address }),
    spent: expectBoolean(readField(object, "spent", path), `${path}.spent`),
  };
}

function parseTransactionSummary(value: unknown, path: string): TransactionSummary {
  const object = expectObject(value, path);
  return {
    txid: expectString(readField(object, "txid", path), `${path}.txid`),
    blockHeight: expectNumber(readField(object, "blockHeight", path), `${path}.blockHeight`),
    blockPos: expectNumber(readField(object, "blockPos", path), `${path}.blockPos`),
    version: expectNumber(readField(object, "version", path), `${path}.version`),
    txType: expectString(readField(object, "txType", path), `${path}.txType`),
    lockTime: expectNumber(readField(object, "lockTime", path), `${path}.lockTime`),
    size: expectNumber(readField(object, "size", path), `${path}.size`),
    valueOut: expectNumber(readField(object, "valueOut", path), `${path}.valueOut`),
    inputCount: expectNumber(readField(object, "inputCount", path), `${path}.inputCount`),
    outputCount: expectNumber(readField(object, "outputCount", path), `${path}.outputCount`),
    tags: parseTransactionTags(readField(object, "tags", path), `${path}.tags`),
  };
}

function parseTransactionDetail(value: unknown, path: string): TransactionDetail {
  const object = expectObject(value, path);
  const asOfHeight = expectOptionalNumber(readOptionalField(object, "asOfHeight"), `${path}.asOfHeight`);
  const blockHeight = expectOptionalNumber(readOptionalField(object, "blockHeight"), `${path}.blockHeight`);
  const blockPos = expectOptionalNumber(readOptionalField(object, "blockPos"), `${path}.blockPos`);
  const fee = expectOptionalNumber(readOptionalField(object, "fee"), `${path}.fee`);
  const payload = readOptionalField(object, "payload");
  return {
    ...(asOfHeight == null ? {} : { asOfHeight }),
    txid: expectString(readField(object, "txid", path), `${path}.txid`),
    ...(blockHeight == null ? {} : { blockHeight }),
    ...(blockPos == null ? {} : { blockPos }),
    version: expectNumber(readField(object, "version", path), `${path}.version`),
    txType: expectString(readField(object, "txType", path), `${path}.txType`),
    lockTime: expectNumber(readField(object, "lockTime", path), `${path}.lockTime`),
    size: expectNumber(readField(object, "size", path), `${path}.size`),
    valueOut: expectNumber(readField(object, "valueOut", path), `${path}.valueOut`),
    inputCount: expectNumber(readField(object, "inputCount", path), `${path}.inputCount`),
    outputCount: expectNumber(readField(object, "outputCount", path), `${path}.outputCount`),
    ...(fee == null ? {} : { fee }),
    tags: parseTransactionTags(readField(object, "tags", path), `${path}.tags`),
    live: parseTransactionLive(readField(object, "live", path), `${path}.live`),
    ...(payload == null ? {} : { payload }),
    vin: parseInputs(readField(object, "vin", path), `${path}.vin`),
    vout: parseOutputs(readField(object, "vout", path), `${path}.vout`),
  };
}

function parseInputs(value: unknown, path: string): readonly TransactionInput[] {
  return expectArray(value, path, parseTransactionInput);
}

function parseOutputs(value: unknown, path: string): readonly TransactionOutput[] {
  return expectArray(value, path, parseTransactionOutput);
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
 * Build the transaction route family for one client transport context.
 */
export function buildTransactionRoutes(context: ClientContext): TransactionRoutes {
  return {
    async get(txid: string): Promise<Result<TransactionDetail, ApiError>> {
      return requestJsonResult(
        context,
        TRANSACTION_ROUTE,
        `/api/v1/tx/${encodePathSegment(txid)}`,
        parseTransactionDetail
      );
    },

    async list(params?: CursorPageParams): Promise<Result<IndexedPage<TransactionSummary>, ApiError>> {
      const query = createCursorQuery(params);
      return requestJsonResult(
        context,
        TRANSACTIONS_ROUTE,
        TRANSACTIONS_ROUTE.path,
        (value: unknown, path: string) => parseIndexedPage(value, path, parseTransactionSummary),
        ...(query == null ? [] : [{ query }])
      );
    },
  };
}
