import assert from "node:assert/strict";
import test from "node:test";

import { HY_GRAPH_FULLSCREEN_MAX_WIDTH, HyGraphGetPresentationMode } from "../../src/lib/views/HyGraphUtil.ts";

void test("HyGraphGetPresentationMode uses fullscreen through tablet width", (): void => {
  assert.equal(HyGraphGetPresentationMode(375), "fullscreen");
  assert.equal(HyGraphGetPresentationMode(HY_GRAPH_FULLSCREEN_MAX_WIDTH), "fullscreen");
});

void test("HyGraphGetPresentationMode uses overlay above tablet width", (): void => {
  assert.equal(HyGraphGetPresentationMode(HY_GRAPH_FULLSCREEN_MAX_WIDTH + 1), "overlay");
  assert.equal(HyGraphGetPresentationMode(1280), "overlay");
});
