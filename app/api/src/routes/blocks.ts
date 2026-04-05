import {
  ReadinessModel,
  RouteClass,
  type CursorPageParams,
  type IndexedPage,
  type RouteDescriptor,
} from "../contracts/common.js";
import type { ApiError } from "../contracts/errors.js";
import type { Result } from "../contracts/result.js";
import {
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
 * Summary tags attached to block responses.
 */
export interface BlockTags {
  readonly orphan: boolean;
}

/**
 * Nullable live tags attached to block overlay responses.
 */
export interface BlockLiveTags {
  readonly chainlock?: boolean;
}

/**
 * Nullable live overlay attached to block responses.
 */
export interface BlockLive {
  readonly tags?: BlockLiveTags;
  readonly coreHeight?: number;
  readonly seenAt?: string;
}

/**
 * Coinbase special transaction fields attached to block detail.
 */
export interface CbTxFields {
  readonly height: number;
  readonly merkleRootMnList?: string;
  readonly merkleFlags?: number;
  readonly bestClHeight?: number;
  readonly bestClSignature?: string;
  readonly creditPoolBalance?: number;
}

/**
 * Canonical block summary returned by block list routes.
 */
export interface BlockSummary {
  readonly blockHeight: number;
  readonly blockHash: string;
  readonly prevBlockHash: string;
  readonly blockTime: string;
  readonly bits: string;
  readonly nonce: number;
  readonly blockVersion: number;
  readonly blockSize: number;
  readonly txCount: number;
  readonly tags: BlockTags;
  readonly live: BlockLive;
}

/**
 * Indexed block detail response.
 */
export interface BlockDetail {
  readonly asOfHeight: number;
  readonly blockHeight: number;
  readonly blockHash: string;
  readonly prevBlockHash: string;
  readonly merkle: string;
  readonly blockTime: string;
  readonly bits: string;
  readonly nonce: number;
  readonly blockVersion: number;
  readonly blockSize: number;
  readonly txCount: number;
  readonly status: string;
  readonly tags: BlockTags;
  readonly live: BlockLive;
  readonly orphanedAt?: string;
  readonly chainwork?: string;
  readonly cbTx?: CbTxFields;
}

/**
 * Transaction summary returned by one block transaction page.
 */
export interface BlockTxSummary {
  readonly txHash: string;
  readonly blockHeight: number;
  readonly blockPos: number;
  readonly txVersion: number;
  readonly txType: string;
  readonly lockTime: number;
  readonly txSize: number;
  readonly valueOut: number;
  readonly inputCount: number;
  readonly outputCount: number;
  readonly tags: BlockTxTags;
}

/**
 * Summary tags attached to block transaction list items.
 */
export interface BlockTxTags {
  readonly mined: boolean;
  readonly coinbase: boolean;
}

/**
 * Route methods for indexed block data.
 */
export interface BlockRoutes {
  /**
   * Read the canonical block list.
   */
  list(params?: CursorPageParams): Promise<Result<IndexedPage<BlockSummary>, ApiError>>;

  /**
   * Read one indexed block by height or hash.
   */
  get(hashOrHeight: number | string): Promise<Result<BlockDetail, ApiError>>;

  /**
   * Read one indexed page of transactions for a block.
   */
  getTransactions(
    hashOrHeight: number | string,
    params?: CursorPageParams
  ): Promise<Result<IndexedPage<BlockTxSummary>, ApiError>>;
}

const BLOCKS_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "blocks",
  path: "/api/v1/blocks",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED_OVERLAY,
};

const BLOCK_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "block",
  path: "/api/v1/block/{hashOrHeight}",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED_OVERLAY,
};

const BLOCK_TXS_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "blockTransactions",
  path: "/api/v1/block/{hashOrHeight}/txs",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED,
};

/**
 * Public route descriptors for block routes.
 */
export const blockRouteDescriptors = {
  detail: BLOCK_ROUTE,
  list: BLOCKS_ROUTE,
  transactions: BLOCK_TXS_ROUTE,
} as const;

function parseBlockTags(value: unknown, path: string): BlockTags {
  const object = expectObject(value, path);
  return {
    orphan: expectOptionalBoolean(readOptionalField(object, "orphan"), `${path}.orphan`) ?? false,
  };
}

function parseBlockLiveTags(value: unknown, path: string): BlockLiveTags {
  const object = expectObject(value, path);
  const chainlock = expectOptionalBoolean(readOptionalField(object, "chainlock"), `${path}.chainlock`);
  return {
    ...(chainlock == null ? {} : { chainlock }),
  };
}

function parseBlockLive(value: unknown, path: string): BlockLive {
  const object = expectObject(value, path);
  const tagsValue = readOptionalField(object, "tags");
  const coreHeight = expectOptionalNumber(readOptionalField(object, "coreHeight"), `${path}.coreHeight`);
  const seenAt = expectOptionalString(readOptionalField(object, "seenAt"), `${path}.seenAt`);
  return {
    ...(tagsValue == null ? {} : { tags: parseBlockLiveTags(tagsValue, `${path}.tags`) }),
    ...(coreHeight == null ? {} : { coreHeight }),
    ...(seenAt == null ? {} : { seenAt }),
  };
}

function parseCbTxFields(value: unknown, path: string): CbTxFields {
  const object = expectObject(value, path);
  const merkleRootMnList = expectOptionalString(
    readOptionalField(object, "merkleRootMnList"),
    `${path}.merkleRootMnList`
  );
  const merkleFlags = expectOptionalNumber(readOptionalField(object, "merkleFlags"), `${path}.merkleFlags`);
  const bestClHeight = expectOptionalNumber(readOptionalField(object, "bestClHeight"), `${path}.bestClHeight`);
  const bestClSignature = expectOptionalString(readOptionalField(object, "bestClSignature"), `${path}.bestClSignature`);
  const creditPoolBalance = expectOptionalNumber(
    readOptionalField(object, "creditPoolBalance"),
    `${path}.creditPoolBalance`
  );
  return {
    height: expectNumber(readField(object, "height", path), `${path}.height`),
    ...(merkleRootMnList == null ? {} : { merkleRootMnList }),
    ...(merkleFlags == null ? {} : { merkleFlags }),
    ...(bestClHeight == null ? {} : { bestClHeight }),
    ...(bestClSignature == null ? {} : { bestClSignature }),
    ...(creditPoolBalance == null ? {} : { creditPoolBalance }),
  };
}

function parseBlockSummary(value: unknown, path: string): BlockSummary {
  const object = expectObject(value, path);
  return {
    blockHeight: expectNumber(readField(object, "blockHeight", path), `${path}.blockHeight`),
    blockHash: expectString(readField(object, "blockHash", path), `${path}.blockHash`),
    prevBlockHash: expectString(readField(object, "prevBlockHash", path), `${path}.prevBlockHash`),
    blockTime: expectString(readField(object, "blockTime", path), `${path}.blockTime`),
    bits: expectString(readField(object, "bits", path), `${path}.bits`),
    nonce: expectNumber(readField(object, "nonce", path), `${path}.nonce`),
    blockVersion: expectNumber(readField(object, "blockVersion", path), `${path}.blockVersion`),
    blockSize: expectNumber(readField(object, "blockSize", path), `${path}.blockSize`),
    txCount: expectNumber(readField(object, "txCount", path), `${path}.txCount`),
    tags: parseBlockTags(readField(object, "tags", path), `${path}.tags`),
    live: parseBlockLive(readField(object, "live", path), `${path}.live`),
  };
}

function parseBlockDetail(value: unknown, path: string): BlockDetail {
  const object = expectObject(value, path);
  const orphanedAt = expectOptionalString(readOptionalField(object, "orphanedAt"), `${path}.orphanedAt`);
  const chainwork = expectOptionalString(readOptionalField(object, "chainwork"), `${path}.chainwork`);
  const cbTx = readOptionalField(object, "cbTx");
  return {
    asOfHeight: expectNumber(readField(object, "asOfHeight", path), `${path}.asOfHeight`),
    blockHeight: expectNumber(readField(object, "blockHeight", path), `${path}.blockHeight`),
    blockHash: expectString(readField(object, "blockHash", path), `${path}.blockHash`),
    prevBlockHash: expectString(readField(object, "prevBlockHash", path), `${path}.prevBlockHash`),
    merkle: expectString(readField(object, "merkle", path), `${path}.merkle`),
    blockTime: expectString(readField(object, "blockTime", path), `${path}.blockTime`),
    bits: expectString(readField(object, "bits", path), `${path}.bits`),
    nonce: expectNumber(readField(object, "nonce", path), `${path}.nonce`),
    blockVersion: expectNumber(readField(object, "blockVersion", path), `${path}.blockVersion`),
    blockSize: expectNumber(readField(object, "blockSize", path), `${path}.blockSize`),
    txCount: expectNumber(readField(object, "txCount", path), `${path}.txCount`),
    status: expectString(readField(object, "status", path), `${path}.status`),
    tags: parseBlockTags(readField(object, "tags", path), `${path}.tags`),
    live: parseBlockLive(readField(object, "live", path), `${path}.live`),
    ...(orphanedAt == null ? {} : { orphanedAt }),
    ...(chainwork == null ? {} : { chainwork }),
    ...(cbTx == null ? {} : { cbTx: parseCbTxFields(cbTx, `${path}.cbTx`) }),
  };
}

function parseBlockTxTags(value: unknown, path: string): BlockTxTags {
  const object = expectObject(value, path);
  return {
    mined: expectOptionalBoolean(readOptionalField(object, "mined"), `${path}.mined`) ?? false,
    coinbase: expectOptionalBoolean(readOptionalField(object, "coinbase"), `${path}.coinbase`) ?? false,
  };
}

function parseBlockTxSummary(value: unknown, path: string): BlockTxSummary {
  const object = expectObject(value, path);
  return {
    txHash: expectString(readField(object, "txHash", path), `${path}.txHash`),
    blockHeight: expectNumber(readField(object, "blockHeight", path), `${path}.blockHeight`),
    blockPos: expectNumber(readField(object, "blockPos", path), `${path}.blockPos`),
    txVersion: expectNumber(readField(object, "txVersion", path), `${path}.txVersion`),
    txType: expectString(readField(object, "txType", path), `${path}.txType`),
    lockTime: expectNumber(readField(object, "lockTime", path), `${path}.lockTime`),
    txSize: expectNumber(readField(object, "txSize", path), `${path}.txSize`),
    valueOut: expectNumber(readField(object, "valueOut", path), `${path}.valueOut`),
    inputCount: expectNumber(readField(object, "inputCount", path), `${path}.inputCount`),
    outputCount: expectNumber(readField(object, "outputCount", path), `${path}.outputCount`),
    tags: parseBlockTxTags(readField(object, "tags", path), `${path}.tags`),
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

export function buildBlockRoutes(context: ClientContext): BlockRoutes {
  return {
    async get(hashOrHeight: number | string): Promise<Result<BlockDetail, ApiError>> {
      return requestJsonResult(
        context,
        BLOCK_ROUTE,
        `/api/v1/block/${encodePathSegment(String(hashOrHeight))}`,
        parseBlockDetail
      );
    },

    async getTransactions(
      hashOrHeight: number | string,
      params?: CursorPageParams
    ): Promise<Result<IndexedPage<BlockTxSummary>, ApiError>> {
      const query = createCursorQuery(params);
      return requestJsonResult(
        context,
        BLOCK_TXS_ROUTE,
        `/api/v1/block/${encodePathSegment(String(hashOrHeight))}/txs`,
        (value: unknown, path: string) => parseIndexedPage(value, path, parseBlockTxSummary),
        ...(query == null ? [] : [{ query }])
      );
    },

    async list(params?: CursorPageParams): Promise<Result<IndexedPage<BlockSummary>, ApiError>> {
      const query = createCursorQuery(params);
      return requestJsonResult(
        context,
        BLOCKS_ROUTE,
        BLOCKS_ROUTE.path,
        (value: unknown, path: string) => parseIndexedPage(value, path, parseBlockSummary),
        ...(query == null ? [] : [{ query }])
      );
    },
  };
}
