import { describe, expect, it } from "vitest";

import { ApCreateNetworkError, ApErr, ApOk } from "../src/index.js";

describe("result contract", (): void => {
  it("preserves ok values through value access and transformations", (): void => {
    const result = ApOk<number, string>(7);

    expect(result.value()).toBe(7);
    expect(result.error()).toBeUndefined();
    expect(result.valueOr(3)).toBe(7);
    expect(result.errorOr("fallback")).toBe("fallback");
    expect(result.transform((value: number): number => value * 2).value()).toBe(14);
    expect(result.andThen((value: number) => ApOk<number, string>(value + 1)).value()).toBe(8);
    expect(result.transformError((error: string): number => error.length).value()).toBe(7);
    expect(result.orElse((error: string) => ApErr<number, number>(error.length)).value()).toBe(7);
  });

  it("preserves err values through error access and recovery", (): void => {
    const failure = ApCreateNetworkError("network down");
    const result = ApErr<number, typeof failure>(failure);

    expect(result.value()).toBeUndefined();
    expect(result.error()).toEqual(failure);
    expect(result.valueOr(3)).toBe(3);
    expect(result.errorOr(ApCreateNetworkError("fallback"))).toEqual(failure);
    expect(result.transform((value: number): number => value * 2).error()).toEqual(failure);
    expect(result.andThen((value: number) => ApOk<number, typeof failure>(value + 1)).error()).toEqual(failure);
    expect(result.transformError((error: typeof failure): string => error.message).error()).toBe("network down");
    expect(result.orElse((error: typeof failure) => ApOk<number>(error.message.length)).value()).toBe(12);
  });
});
