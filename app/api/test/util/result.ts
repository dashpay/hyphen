import { type ApResult } from "../../src/index.js";

export function expectOk<TValue, TError>(result: ApResult<TValue, TError>): TValue {
  const error = result.error();
  if (error !== undefined) {
    throw new Error(`expected ok result, got error ${JSON.stringify(error)}`);
  }
  const value = result.value();
  if (value === undefined) {
    throw new Error("expected ok result");
  }
  return value;
}

export function expectErr<TValue, TError>(result: ApResult<TValue, TError>): TError {
  const value = result.value();
  if (value !== undefined) {
    throw new Error(`expected error result, got value ${JSON.stringify(value)}`);
  }
  const error = result.error();
  if (error === undefined) {
    throw new Error("expected error result");
  }
  return error;
}
