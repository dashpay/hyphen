import assert from "node:assert/strict";
import test from "node:test";

import { HySearchIsHeightQuery, HySearchShouldUseExactLookup } from "../../src/lib/views/HySearchBoxUtil.ts";

void test("a decimal block height is sent to the exact-lookup endpoint", (): void => {
  // The regression: digits are a subset of the hex alphabet, so testing hex
  // first routed every height into the full-hash branch and dropped it. The
  // daemon answers heights as a primary-key probe with no length floor, so all
  // of these must reach it — including genesis and single digits.
  assert.equal(HySearchShouldUseExactLookup("0"), true);
  assert.equal(HySearchShouldUseExactLookup("1"), true);
  assert.equal(HySearchShouldUseExactLookup("42"), true);
  assert.equal(HySearchShouldUseExactLookup("1088640"), true);
  assert.equal(HySearchShouldUseExactLookup(" 2100000 "), true);
});

void test("a height beyond the daemon's i32 parse is not treated as a height", (): void => {
  assert.equal(HySearchIsHeightQuery("2147483647"), true);
  assert.equal(HySearchIsHeightQuery("2147483648"), false);
  assert.equal(HySearchIsHeightQuery("99999999999999999999"), false);
  assert.equal(HySearchIsHeightQuery("-1"), false);
  assert.equal(HySearchIsHeightQuery("1.5"), false);
  assert.equal(HySearchIsHeightQuery(""), false);
});

void test("hex queries still need to be a complete hash", (): void => {
  const fullHash = "a".repeat(64);
  assert.equal(HySearchShouldUseExactLookup(fullHash), true);
  // A 64-digit string is all-digits but overflows i32, so it falls through to
  // the hash branch and is accepted as a hash rather than rejected as a height.
  assert.equal(HySearchShouldUseExactLookup("1".repeat(64)), true);
  // Partial hex prefixes stay with the locally-scored suggestion pool.
  assert.equal(HySearchShouldUseExactLookup("abcd"), false);
  assert.equal(HySearchShouldUseExactLookup("deadbeef"), false);
});

void test("base58 and free-text queries keep the three-character floor", (): void => {
  assert.equal(HySearchShouldUseExactLookup("XjUQQHuVvrXqRHqmWarqmGsd79HbeUAqN7"), true);
  assert.equal(HySearchShouldUseExactLookup("treasury"), true);
  assert.equal(HySearchShouldUseExactLookup("xy"), false);
  assert.equal(HySearchShouldUseExactLookup("   "), false);
});
