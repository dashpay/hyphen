import { ReadinessModel, RouteClass, type RouteDescriptor } from "../contracts/common.js";
import type { ApiError } from "../contracts/errors.js";
import { type Result } from "../contracts/result.js";
import {
  expectArray,
  expectNumber,
  expectObject,
  expectOptionalNumber,
  readField,
  readOptionalField,
} from "../internal/json.js";
import { createQuery } from "../internal/query.js";
import { requestJsonResult, type ClientContext } from "../internal/transport.js";

/**
 * Derived block metrics snapshot.
 */
export interface BlockMetrics {
  readonly height: number;
  readonly txCount: number;
  readonly utxosSpendable: number;
  readonly utxosUnspendable: number;
  readonly blockchainSize: number;
  readonly creditPoolBalance?: number;
  readonly circulatingSupply: number;
  readonly difficulty: number;
  readonly hashrate: number;
}

/**
 * Query parameters for the metrics chart route.
 */
export interface MetricsChartParams {
  readonly blocks?: number;
  readonly points?: number;
}

/**
 * Snapshot-backed metrics chart response.
 */
export interface MetricsChart {
  readonly asOfHeight: number;
  readonly startHeight: number;
  readonly endHeight: number;
  readonly bucketBlocks: number;
  readonly value: readonly BlockMetrics[];
}

/**
 * Route methods for metrics indexed products.
 */
export interface MetricsRoutes {
  /**
   * Read the latest metrics snapshot.
   */
  getLatest(): Promise<Result<BlockMetrics, ApiError>>;

  /**
   * Read one snapshot-backed metrics chart response.
   */
  getChart(params?: MetricsChartParams): Promise<Result<MetricsChart, ApiError>>;
}

const METRICS_LATEST_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "metricsLatest",
  path: "/api/v1/metrics/latest",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED,
};

const METRICS_CHART_ROUTE: RouteDescriptor = {
  method: "GET",
  name: "metricsChart",
  path: "/api/v1/metrics/chart",
  readinessModel: ReadinessModel.INDEXED,
  routeClass: RouteClass.INDEXED,
};

/**
 * Public route descriptors for metrics routes.
 */
export const metricsRouteDescriptors = {
  chart: METRICS_CHART_ROUTE,
  latest: METRICS_LATEST_ROUTE,
} as const;

function parseBlockMetrics(value: unknown, path: string): BlockMetrics {
  const object = expectObject(value, path);
  const creditPoolBalance = expectOptionalNumber(
    readOptionalField(object, "creditPoolBalance"),
    `${path}.creditPoolBalance`
  );
  return {
    height: expectNumber(readField(object, "height", path), `${path}.height`),
    txCount: expectNumber(readField(object, "txCount", path), `${path}.txCount`),
    utxosSpendable: expectNumber(readField(object, "utxosSpendable", path), `${path}.utxosSpendable`),
    utxosUnspendable: expectNumber(readField(object, "utxosUnspendable", path), `${path}.utxosUnspendable`),
    blockchainSize: expectNumber(readField(object, "blockchainSize", path), `${path}.blockchainSize`),
    ...(creditPoolBalance == null ? {} : { creditPoolBalance }),
    circulatingSupply: expectNumber(readField(object, "circulatingSupply", path), `${path}.circulatingSupply`),
    difficulty: expectNumber(readField(object, "difficulty", path), `${path}.difficulty`),
    hashrate: expectNumber(readField(object, "hashrate", path), `${path}.hashrate`),
  };
}

function parseMetricsChart(value: unknown, path: string): MetricsChart {
  const object = expectObject(value, path);
  return {
    asOfHeight: expectNumber(readField(object, "asOfHeight", path), `${path}.asOfHeight`),
    startHeight: expectNumber(readField(object, "startHeight", path), `${path}.startHeight`),
    endHeight: expectNumber(readField(object, "endHeight", path), `${path}.endHeight`),
    bucketBlocks: expectNumber(readField(object, "bucketBlocks", path), `${path}.bucketBlocks`),
    value: expectArray(readField(object, "value", path), `${path}.value`, parseBlockMetrics),
  };
}

function createMetricsChartQuery(params?: MetricsChartParams): URLSearchParams | undefined {
  if (params == null) {
    return undefined;
  }
  const query = createQuery({
    blocks: params.blocks,
    points: params.points,
  });
  return query.size > 0 ? query : undefined;
}

/**
 * Build the metrics route family for one client transport context.
 */
export function buildMetricsRoutes(context: ClientContext): MetricsRoutes {
  return {
    async getChart(params?: MetricsChartParams): Promise<Result<MetricsChart, ApiError>> {
      const query = createMetricsChartQuery(params);
      return requestJsonResult(
        context,
        METRICS_CHART_ROUTE,
        METRICS_CHART_ROUTE.path,
        parseMetricsChart,
        ...(query == null ? [] : [{ query }])
      );
    },

    async getLatest(): Promise<Result<BlockMetrics, ApiError>> {
      return requestJsonResult(context, METRICS_LATEST_ROUTE, METRICS_LATEST_ROUTE.path, parseBlockMetrics);
    },
  };
}
