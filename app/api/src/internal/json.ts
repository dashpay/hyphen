import { failContract } from "./errors.js";
import type { ErrorBody, ErrorResponse, IndexedPage, LivePage } from "../contracts/common.js";

type ObjectValue = Record<string, unknown>;

function fail(path: string, message: string): never {
  return failContract(path, message);
}

export function expectObject(value: unknown, path: string): ObjectValue {
  if (typeof value !== "object" || value == null || Array.isArray(value)) {
    fail(path, "expected object");
  }
  return value as ObjectValue;
}

export function expectString(value: unknown, path: string): string {
  if (typeof value !== "string") {
    fail(path, "expected string");
  }
  return value;
}

export function expectNumber(value: unknown, path: string): number {
  if (typeof value !== "number" || Number.isNaN(value)) {
    fail(path, "expected number");
  }
  return value;
}

export function expectBoolean(value: unknown, path: string): boolean {
  if (typeof value !== "boolean") {
    fail(path, "expected boolean");
  }
  return value;
}

export function expectOptionalString(value: unknown, path: string): string | undefined {
  if (value == null) {
    return undefined;
  }
  return expectString(value, path);
}

export function expectOptionalNumber(value: unknown, path: string): number | undefined {
  if (value == null) {
    return undefined;
  }
  return expectNumber(value, path);
}

export function expectOptionalBoolean(value: unknown, path: string): boolean | undefined {
  if (value == null) {
    return undefined;
  }
  return expectBoolean(value, path);
}

export function expectArray<TValue>(
  value: unknown,
  path: string,
  parseValue: (value: unknown, path: string) => TValue
): readonly TValue[] {
  if (!Array.isArray(value)) {
    fail(path, "expected array");
  }
  return value.map((entry, index) => parseValue(entry, `${path}[${String(index)}]`));
}

export function expectObjectArray<TValue>(
  value: unknown,
  path: string,
  parseValue: (value: ObjectValue, path: string) => TValue
): readonly TValue[] {
  if (!Array.isArray(value)) {
    fail(path, "expected array");
  }
  return value.map((entry, index) => {
    const itemPath = `${path}[${String(index)}]`;
    return parseValue(expectObject(entry, itemPath), itemPath);
  });
}

export function readField(object: ObjectValue, field: string, path: string): unknown {
  if (!(field in object)) {
    fail(`${path}.${field}`, "missing field");
  }
  return object[field];
}

export function readOptionalField(object: ObjectValue, field: string): unknown {
  if (!(field in object)) {
    return undefined;
  }
  return object[field];
}

export function parseErrorBody(value: unknown, path = "$"): ErrorBody {
  const object = expectObject(value, path);
  const target = expectOptionalString(readOptionalField(object, "target"), `${path}.target`);
  return {
    code: expectString(readField(object, "code", path), `${path}.code`),
    message: expectString(readField(object, "message", path), `${path}.message`),
    ...(target == null ? {} : { target }),
  };
}

export function parseErrorResponse(value: unknown, path = "$"): ErrorResponse {
  const object = expectObject(value, path);
  return {
    error: parseErrorBody(readField(object, "error", path), `${path}.error`),
  };
}

export function parseIndexedPage<TValue>(
  value: unknown,
  path: string,
  parseItem: (value: unknown, path: string) => TValue
): IndexedPage<TValue> {
  const object = expectObject(value, path);
  const nextCursor = expectOptionalString(readOptionalField(object, "nextCursor"), `${path}.nextCursor`);
  const totalCount = expectOptionalNumber(readOptionalField(object, "totalCount"), `${path}.totalCount`);
  return {
    asOfHeight: expectNumber(readField(object, "asOfHeight", path), `${path}.asOfHeight`),
    ...(totalCount == null ? {} : { totalCount }),
    value: expectArray(readField(object, "value", path), `${path}.value`, parseItem),
    ...(nextCursor == null ? {} : { nextCursor }),
  };
}

export function parseLivePage<TValue>(
  value: unknown,
  path: string,
  parseItem: (value: unknown, path: string) => TValue
): LivePage<TValue> {
  const object = expectObject(value, path);
  const nextCursor = expectOptionalString(readOptionalField(object, "nextCursor"), `${path}.nextCursor`);
  const totalCount = expectOptionalNumber(readOptionalField(object, "totalCount"), `${path}.totalCount`);
  return {
    ...(totalCount == null ? {} : { totalCount }),
    value: expectArray(readField(object, "value", path), `${path}.value`, parseItem),
    ...(nextCursor == null ? {} : { nextCursor }),
  };
}
