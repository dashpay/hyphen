import {
  AvailabilityKind,
  ReadinessModel,
  type ClientOptions,
  type ErrorResponse,
  type RequestOptions,
  type RouteDescriptor,
} from "../contracts/common.js";
import { availabilityError, contractError, httpError, networkError, type ApiError } from "../contracts/errors.js";
import { err, ok, type Result } from "../contracts/result.js";
import { ContractViolationError } from "./errors.js";
import { parseErrorResponse } from "./json.js";
import { buildUrl } from "./query.js";

export interface ClientContext {
  readonly baseUrl: URL;
  readonly fetchFn: typeof fetch;
  readonly headers?: HeadersInit;
}

type Parser<TValue> = (value: unknown, path: string) => TValue;

function normalizeBaseUrl(baseUrl: string): URL {
  const normalizedBaseUrl = baseUrl.endsWith("/") ? baseUrl : `${baseUrl}/`;
  return new URL(normalizedBaseUrl);
}

export function createClientContext(options: ClientOptions): ClientContext {
  return {
    baseUrl: normalizeBaseUrl(options.baseUrl),
    fetchFn: options.fetch ?? fetch,
    ...(options.headers == null ? {} : { headers: options.headers }),
  };
}

async function maybeParseErrorResponse(response: Response): Promise<ErrorResponse | undefined> {
  const contentType = response.headers.get("content-type");
  if (!contentType?.includes("application/json")) {
    return undefined;
  }

  try {
    const value: unknown = await response.json();
    return parseErrorResponse(value);
  } catch {
    return undefined;
  }
}

function classifyAvailability(status: number, descriptor: RouteDescriptor): AvailabilityKind | undefined {
  if (status === 501) {
    return AvailabilityKind.CAPABILITY_DISABLED;
  }
  if (status !== 503) {
    return undefined;
  }
  switch (descriptor.readinessModel) {
    case ReadinessModel.INDEXED:
      return AvailabilityKind.INDEX_NOT_READY;
    case ReadinessModel.LIVE:
      return AvailabilityKind.LIVE_NOT_READY;
    case ReadinessModel.NONE:
      return undefined;
  }
}

function messageFromErrorBody(status: number, body?: ErrorResponse): string {
  if (body == null) {
    return `HTTP ${String(status)}`;
  }
  return body.error.message;
}

export async function requestResponseResult(
  context: ClientContext,
  descriptor: RouteDescriptor,
  path: string,
  options?: RequestOptions
): Promise<Result<Response, ApiError>> {
  const url = buildUrl(context.baseUrl, path, options);
  let response: Response;
  try {
    response = await context.fetchFn(url, {
      method: descriptor.method,
      ...(context.headers == null ? {} : { headers: context.headers }),
    });
  } catch (cause: unknown) {
    return err(networkError("request failed before receiving a response", cause));
  }

  if (!response.ok) {
    const body = await maybeParseErrorResponse(response);
    const message = messageFromErrorBody(response.status, body);
    const availabilityKind = classifyAvailability(response.status, descriptor);
    if (availabilityKind != null) {
      return err(availabilityError(message, response.status, descriptor, availabilityKind, body));
    }
    return err(httpError(message, response.status, descriptor, body));
  }

  return ok(response);
}

export async function requestJsonResult<TValue>(
  context: ClientContext,
  descriptor: RouteDescriptor,
  path: string,
  parse: Parser<TValue>,
  options?: RequestOptions
): Promise<Result<TValue, ApiError>> {
  const responseResult = await requestResponseResult(context, descriptor, path, options);
  const response = responseResult.value();
  if (response == null) {
    return err(responseResult.errorOr(networkError("response missing")));
  }

  const contentType = response.headers.get("content-type");
  if (!contentType?.includes("application/json")) {
    return err(contractError(path, "expected application/json response"));
  }

  let value: unknown;
  try {
    value = await response.json();
  } catch (cause: unknown) {
    return err(networkError("failed to decode json response body", cause));
  }

  try {
    return ok(parse(value, "$"));
  } catch (error: unknown) {
    if (error instanceof ContractViolationError) {
      return err(contractError(error.path, error.message));
    }
    return err(networkError("unexpected parser failure", error));
  }
}
