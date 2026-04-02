/**
 * Shared contract interfaces and enums for the Hyphen REST API.
 */

/**
 * Describes how one route is backed on the daemon side.
 */
export enum RouteClass {
  INDEXED = "indexed",
  INDEXED_OVERLAY = "indexedOverlay",
  LIVE = "live",
  RAW = "raw",
  CROSS_DOMAIN = "crossDomain",
}

/**
 * Describes which readiness model applies to one route.
 */
export enum ReadinessModel {
  INDEXED = "indexed",
  LIVE = "live",
  NONE = "none",
}

/**
 * Classifies one availability failure into a client-meaningful state.
 */
export enum AvailabilityKind {
  CAPABILITY_DISABLED = "capabilityDisabled",
  INDEX_NOT_READY = "indexNotReady",
  LIVE_NOT_READY = "liveNotReady",
}

/**
 * Standard JSON error body returned by the daemon.
 */
export interface ErrorBody {
  readonly code: string;
  readonly message: string;
  readonly target?: string;
}

/**
 * Standard JSON error envelope returned by the daemon.
 */
export interface ErrorResponse {
  readonly error: ErrorBody;
}

/**
 * Shared indexed page envelope used by snapshot-bound collection routes.
 */
export interface IndexedPage<TValue> {
  readonly asOfHeight: number;
  readonly totalCount?: number;
  readonly value: readonly TValue[];
  readonly nextCursor?: string;
}

/**
 * Shared live page envelope used by non-snapshot collection routes.
 */
export interface LivePage<TValue> {
  readonly totalCount?: number;
  readonly value: readonly TValue[];
  readonly nextCursor?: string;
}

/**
 * Static metadata describing one daemon route.
 */
export interface RouteDescriptor {
  readonly name: string;
  readonly method: "GET";
  readonly path: string;
  readonly routeClass: RouteClass;
  readonly readinessModel: ReadinessModel;
}

/**
 * Shared client options used to construct a Hyphen API transport.
 */
export interface ClientOptions {
  readonly baseUrl: string;
  readonly fetch?: typeof fetch;
  readonly headers?: HeadersInit;
}

/**
 * One low-level request configuration used by route clients.
 */
export interface RequestOptions {
  readonly query?: URLSearchParams;
}

/**
 * Shared pagination query accepted by cursor-based collection routes.
 */
export interface CursorPageParams {
  readonly cursor?: string;
  readonly limit?: number;
}
