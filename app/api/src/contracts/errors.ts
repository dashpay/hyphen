import type { AvailabilityKind, ErrorResponse, RouteDescriptor } from "./common.js";

/**
 * Error returned when the daemon responds with a non-availability HTTP error.
 */
export interface HttpError {
  readonly kind: "http";
  readonly message: string;
  readonly status: number;
  readonly descriptor: RouteDescriptor;
  readonly body?: ErrorResponse;
}

/**
 * Error returned when a route is disabled or its backing dataset is not ready.
 */
export interface AvailabilityError {
  readonly kind: "availability";
  readonly message: string;
  readonly status: number;
  readonly descriptor: RouteDescriptor;
  readonly availabilityKind: AvailabilityKind;
  readonly body?: ErrorResponse;
}

/**
 * Error returned when one daemon payload violates the expected contract.
 */
export interface ContractError {
  readonly kind: "contract";
  readonly message: string;
  readonly path: string;
}

/**
 * Error returned when the transport cannot reach the daemon or read the body.
 */
export interface NetworkError {
  readonly kind: "network";
  readonly message: string;
  readonly cause?: unknown;
}

/**
 * Public error union returned by `ApResult` failure branches.
 */
export type ApiError = AvailabilityError | ContractError | HttpError | NetworkError;

/**
 * Create one HTTP error value.
 */
export function httpError(
  message: string,
  status: number,
  descriptor: RouteDescriptor,
  body?: ErrorResponse
): HttpError {
  return {
    kind: "http",
    message,
    status,
    descriptor,
    ...(body == null ? {} : { body }),
  };
}

/**
 * Create one availability error value.
 */
export function availabilityError(
  message: string,
  status: number,
  descriptor: RouteDescriptor,
  availabilityKind: AvailabilityKind,
  body?: ErrorResponse
): AvailabilityError {
  return {
    kind: "availability",
    message,
    status,
    descriptor,
    availabilityKind,
    ...(body == null ? {} : { body }),
  };
}

/**
 * Create one contract error value.
 */
export function contractError(path: string, message: string): ContractError {
  return {
    kind: "contract",
    message,
    path,
  };
}

/**
 * Create one network error value.
 */
export function networkError(message: string, cause?: unknown): NetworkError {
  return {
    kind: "network",
    message,
    ...(cause === undefined ? {} : { cause }),
  };
}
