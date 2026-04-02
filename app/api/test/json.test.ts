import { describe, expect, it } from "vitest";

import {
  expectArray,
  expectBoolean,
  expectNumber,
  expectObject,
  expectOptionalBoolean,
  expectOptionalNumber,
  expectOptionalString,
  expectString,
  parseErrorResponse,
  parseIndexedPage,
  parseLivePage,
  readField,
  readOptionalField,
} from "../src/internal/json.js";

describe("json helpers", (): void => {
  it("accepts valid primitives and pages", (): void => {
    expect(expectObject({ ok: true }, "$")).toEqual({ ok: true });
    expect(expectString("x", "$")).toBe("x");
    expect(expectNumber(3, "$")).toBe(3);
    expect(expectBoolean(true, "$")).toBe(true);
    expect(expectOptionalString(undefined, "$")).toBeUndefined();
    expect(expectOptionalNumber(undefined, "$")).toBeUndefined();
    expect(expectOptionalBoolean(undefined, "$")).toBeUndefined();
    expect(readField({ a: 1 }, "a", "$")).toBe(1);
    expect(readOptionalField({ a: 1 }, "b")).toBeUndefined();

    const indexed = parseIndexedPage(
      {
        asOfHeight: 7,
        value: [{ id: 1 }],
        nextCursor: "next",
      },
      "$",
      (value): number => expectNumber((value as { id: number }).id, "$.id")
    );
    const live = parseLivePage(
      {
        value: ["x"],
      },
      "$",
      (value): string => expectString(value, "$")
    );

    expect(indexed.asOfHeight).toBe(7);
    expect(indexed.nextCursor).toBe("next");
    expect(live.value[0]).toBe("x");
    expect(parseErrorResponse({ error: { code: "bad", message: "oops" } }).error.message).toBe("oops");
  });

  it("reports parser paths for malformed synthetic payloads", (): void => {
    expect(() => expectObject([], "$")).toThrowError("expected object");
    expect(() => expectString(1, "$.s")).toThrowError("expected string");
    expect(() => expectNumber("1", "$.n")).toThrowError("expected number");
    expect(() => expectBoolean("true", "$.b")).toThrowError("expected boolean");
    expect(() => expectArray({}, "$.items", (value): unknown => value)).toThrowError("expected array");
    expect(() => readField({}, "missing", "$")).toThrowError("missing field");
    expect(() => parseIndexedPage({ value: [] }, "$", (value): unknown => value)).toThrowError("missing field");
    expect(() => parseErrorResponse({ error: { code: 1, message: "oops" } })).toThrowError("expected string");
  });
});
