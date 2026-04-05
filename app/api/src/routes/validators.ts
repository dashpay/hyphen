import { ReadinessModel, RouteClass, type IndexedPage, type RouteDescriptor } from "../contracts/common.js";
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
  parseIndexedPage,
  readField,
  readOptionalField,
} from "../internal/json.js";
import { encodePathSegment, createQuery } from "../internal/query.js";
import { requestJsonResult, type ClientContext } from "../internal/transport.js";

/**
 * Validator address roles grouped by purpose.
 */
export interface ValidatorAddresses {
  readonly collateral?: string;
  readonly owner?: string;
  readonly payee?: string;
  readonly voting?: string;
  readonly payout?: string;
}

/**
 * Nullable PoSe overlay attached to validator routes.
 */
export interface ValidatorPose {
  readonly banHeight?: number;
  readonly banScore?: number;
  readonly revivedHeight?: number;
}

/**
 * Validator service endpoints grouped by transport purpose.
 */
export interface ValidatorServices {
  readonly coreP2p: readonly string[];
  readonly platformP2p: readonly string[];
  readonly platformHttps: readonly string[];
}

/**
 * Nullable live overlay attached to validator routes.
 */
export interface ValidatorLive {
  readonly lastPaidAt?: string;
  readonly lastPaidHeight?: number;
  readonly consecutivePayments?: number;
  readonly pose?: ValidatorPose;
  readonly revocationReason?: number;
  readonly status?: string;
  readonly type?: string;
  readonly valid?: boolean;
}

/**
 * Indexed validator summary returned by list routes.
 */
export interface ValidatorSummary {
  readonly proTxHash: string;
  readonly collateralHash: string;
  readonly collateralIndex: number;
  readonly addresses: ValidatorAddresses;
  readonly live: ValidatorLive;
  readonly operatorPubKey?: string;
  readonly operatorReward: number;
  readonly type: string;
  readonly status: string;
  readonly services: ValidatorServices;
  readonly registeredHeight: number;
  readonly lastUpdatedHeight: number;
}

/**
 * One validator special-transaction history row.
 */
export interface ValidatorHistoryEntry {
  readonly txid: string;
  readonly txType: string;
  readonly payload: unknown;
  readonly blockHeight: number;
}

/**
 * Full validator detail including history.
 */
export interface ValidatorDetail {
  readonly asOfHeight: number;
  readonly proTxHash: string;
  readonly collateralHash: string;
  readonly collateralIndex: number;
  readonly addresses: ValidatorAddresses;
  readonly live: ValidatorLive;
  readonly operatorPubKey?: string;
  readonly operatorReward: number;
  readonly type: string;
  readonly status: string;
  readonly services: ValidatorServices;
  readonly registeredHeight: number;
  readonly lastUpdatedHeight: number;
  readonly history: readonly ValidatorHistoryEntry[];
}

/**
 * Query parameters for the validator list route.
 */
export interface ValidatorListParams {
  readonly cursor?: string;
  readonly limit?: number;
  readonly status?: string;
  readonly type?: string;
}

/**
 * Route methods for validator indexed products.
 */
export interface ValidatorRoutes {
  /**
   * Read one indexed page of validators.
   */
  list(params?: ValidatorListParams): Promise<Result<IndexedPage<ValidatorSummary>, ApiError>>;

  /**
   * Read one validator by proTxHash.
   */
  get(proTxHash: string): Promise<Result<ValidatorDetail, ApiError>>;
}

const VALIDATORS_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "validators",
  path: "/api/v1/validators",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED_OVERLAY,
};

const VALIDATOR_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "validator",
  path: "/api/v1/validator/{proTxHash}",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED_OVERLAY,
};

/**
 * Public route descriptors for validator routes.
 */
export const validatorRouteDescriptors = {
  detail: VALIDATOR_ROUTE,
  list: VALIDATORS_ROUTE,
} as const;

function parseValidatorAddresses(value: unknown, path: string): ValidatorAddresses {
  const object = expectObject(value, path);
  const collateral = expectOptionalString(readOptionalField(object, "collateral"), `${path}.collateral`);
  const owner = expectOptionalString(readOptionalField(object, "owner"), `${path}.owner`);
  const payee = expectOptionalString(readOptionalField(object, "payee"), `${path}.payee`);
  const voting = expectOptionalString(readOptionalField(object, "voting"), `${path}.voting`);
  const payout = expectOptionalString(readOptionalField(object, "payout"), `${path}.payout`);
  return {
    ...(collateral == null ? {} : { collateral }),
    ...(owner == null ? {} : { owner }),
    ...(payee == null ? {} : { payee }),
    ...(voting == null ? {} : { voting }),
    ...(payout == null ? {} : { payout }),
  };
}

function parseStringList(value: unknown, path: string): readonly string[] {
  return expectArray(value, path, expectString);
}

function parseValidatorServices(value: unknown, path: string): ValidatorServices {
  const object = expectObject(value, path);
  return {
    coreP2p: parseStringList(readField(object, "coreP2p", path), `${path}.coreP2p`),
    platformP2p: parseStringList(readField(object, "platformP2p", path), `${path}.platformP2p`),
    platformHttps: parseStringList(readField(object, "platformHttps", path), `${path}.platformHttps`),
  };
}

function parseValidatorPose(value: unknown, path: string): ValidatorPose {
  const object = expectObject(value, path);
  const banHeight = expectOptionalNumber(readOptionalField(object, "banHeight"), `${path}.banHeight`);
  const banScore = expectOptionalNumber(readOptionalField(object, "banScore"), `${path}.banScore`);
  const revivedHeight = expectOptionalNumber(readOptionalField(object, "revivedHeight"), `${path}.revivedHeight`);
  return {
    ...(banHeight == null ? {} : { banHeight }),
    ...(banScore == null ? {} : { banScore }),
    ...(revivedHeight == null ? {} : { revivedHeight }),
  };
}

function parseValidatorLive(value: unknown, path: string): ValidatorLive {
  const object = expectObject(value, path);
  const lastPaidAt = expectOptionalString(readOptionalField(object, "lastPaidAt"), `${path}.lastPaidAt`);
  const lastPaidHeight = expectOptionalNumber(readOptionalField(object, "lastPaidHeight"), `${path}.lastPaidHeight`);
  const consecutivePayments = expectOptionalNumber(
    readOptionalField(object, "consecutivePayments"),
    `${path}.consecutivePayments`
  );
  const pose = readOptionalField(object, "pose");
  const revocationReason = expectOptionalNumber(
    readOptionalField(object, "revocationReason"),
    `${path}.revocationReason`
  );
  const status = expectOptionalString(readOptionalField(object, "status"), `${path}.status`);
  const type = expectOptionalString(readOptionalField(object, "type"), `${path}.type`);
  const validField = readOptionalField(object, "valid");
  const valid = validField == null ? undefined : expectBoolean(validField, `${path}.valid`);
  return {
    ...(lastPaidAt == null ? {} : { lastPaidAt }),
    ...(lastPaidHeight == null ? {} : { lastPaidHeight }),
    ...(consecutivePayments == null ? {} : { consecutivePayments }),
    ...(pose == null ? {} : { pose: parseValidatorPose(pose, `${path}.pose`) }),
    ...(revocationReason == null ? {} : { revocationReason }),
    ...(status == null ? {} : { status }),
    ...(type == null ? {} : { type }),
    ...(valid == null ? {} : { valid }),
  };
}

function parseValidatorSummary(value: unknown, path: string): ValidatorSummary {
  const object = expectObject(value, path);
  const operatorPubKey = expectOptionalString(readOptionalField(object, "operatorPubKey"), `${path}.operatorPubKey`);
  return {
    proTxHash: expectString(readField(object, "proTxHash", path), `${path}.proTxHash`),
    collateralHash: expectString(readField(object, "collateralHash", path), `${path}.collateralHash`),
    collateralIndex: expectNumber(readField(object, "collateralIndex", path), `${path}.collateralIndex`),
    addresses: parseValidatorAddresses(readField(object, "addresses", path), `${path}.addresses`),
    live: parseValidatorLive(readField(object, "live", path), `${path}.live`),
    ...(operatorPubKey == null ? {} : { operatorPubKey }),
    operatorReward: expectNumber(readField(object, "operatorReward", path), `${path}.operatorReward`),
    type: expectString(readField(object, "type", path), `${path}.type`),
    status: expectString(readField(object, "status", path), `${path}.status`),
    services: parseValidatorServices(readField(object, "services", path), `${path}.services`),
    registeredHeight: expectNumber(readField(object, "registeredHeight", path), `${path}.registeredHeight`),
    lastUpdatedHeight: expectNumber(readField(object, "lastUpdatedHeight", path), `${path}.lastUpdatedHeight`),
  };
}

function parseValidatorHistoryEntry(value: unknown, path: string): ValidatorHistoryEntry {
  const object = expectObject(value, path);
  return {
    txid: expectString(readField(object, "txHash", path), `${path}.txHash`),
    txType: expectString(readField(object, "txType", path), `${path}.txType`),
    payload: readField(object, "payload", path),
    blockHeight: expectNumber(readField(object, "blockHeight", path), `${path}.blockHeight`),
  };
}

function parseValidatorDetail(value: unknown, path: string): ValidatorDetail {
  const object = expectObject(value, path);
  const operatorPubKey = expectOptionalString(readOptionalField(object, "operatorPubKey"), `${path}.operatorPubKey`);
  return {
    asOfHeight: expectNumber(readField(object, "asOfHeight", path), `${path}.asOfHeight`),
    proTxHash: expectString(readField(object, "proTxHash", path), `${path}.proTxHash`),
    collateralHash: expectString(readField(object, "collateralHash", path), `${path}.collateralHash`),
    collateralIndex: expectNumber(readField(object, "collateralIndex", path), `${path}.collateralIndex`),
    addresses: parseValidatorAddresses(readField(object, "addresses", path), `${path}.addresses`),
    live: parseValidatorLive(readField(object, "live", path), `${path}.live`),
    ...(operatorPubKey == null ? {} : { operatorPubKey }),
    operatorReward: expectNumber(readField(object, "operatorReward", path), `${path}.operatorReward`),
    type: expectString(readField(object, "type", path), `${path}.type`),
    status: expectString(readField(object, "status", path), `${path}.status`),
    services: parseValidatorServices(readField(object, "services", path), `${path}.services`),
    registeredHeight: expectNumber(readField(object, "registeredHeight", path), `${path}.registeredHeight`),
    lastUpdatedHeight: expectNumber(readField(object, "lastUpdatedHeight", path), `${path}.lastUpdatedHeight`),
    history: expectArray(readField(object, "history", path), `${path}.history`, parseValidatorHistoryEntry),
  };
}

function createValidatorQuery(params?: ValidatorListParams): URLSearchParams | undefined {
  if (params == null) {
    return undefined;
  }
  const query = createQuery({
    cursor: params.cursor,
    limit: params.limit,
    status: params.status,
    type: params.type,
  });
  return query.size > 0 ? query : undefined;
}

/**
 * Build the validator route family for one client transport context.
 */
export function buildValidatorRoutes(context: ClientContext): ValidatorRoutes {
  return {
    async get(proTxHash: string): Promise<Result<ValidatorDetail, ApiError>> {
      return requestJsonResult(
        context,
        VALIDATOR_ROUTE,
        `/api/v1/validator/${encodePathSegment(proTxHash)}`,
        parseValidatorDetail
      );
    },

    async list(params?: ValidatorListParams): Promise<Result<IndexedPage<ValidatorSummary>, ApiError>> {
      const query = createValidatorQuery(params);
      return requestJsonResult(
        context,
        VALIDATORS_ROUTE,
        VALIDATORS_ROUTE.path,
        (value: unknown, path: string) => parseIndexedPage(value, path, parseValidatorSummary),
        ...(query == null ? [] : [{ query }])
      );
    },
  };
}
